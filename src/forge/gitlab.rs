//! GitLab (gitlab.com or any host): one GraphQL query, the pipeline list and a compare over REST.

use serde::Deserialize;
use serde::de::IgnoredAny;
use serde_json::{Value, json};

use super::{ForgeError, Id, Net, Nodes, Res, Title, Username, parse, quote};
use crate::report::{CheckState, Fetched, Issue, Pr, Release};

const GITLAB_QUERY: &str = "query($p:ID!){ project(fullPath:$p){
  webUrl lastActivityAt starCount forksCount visibility repository{rootRef}
  releases(first:1, sort:RELEASED_AT_DESC){ nodes{tagName releasedAt upcomingRelease} }
  issues(state:opened, first:30, sort:CREATED_DESC){ count nodes{iid title webUrl createdAt labels(first:3){nodes{title}}} }
  mergeRequests(state:opened, first:30){ nodes{iid title webUrl draft author{username} headPipeline{status}} }
}}";

#[derive(Deserialize)]
struct GlAnswer {
    data: Option<GlData>,
    #[serde(default)]
    errors: Vec<GlError>,
}

#[derive(Deserialize)]
struct GlData {
    project: Option<GlProject>,
}

#[derive(Deserialize)]
struct GlError {
    message: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct GlProject {
    web_url: String,
    last_activity_at: Option<String>,
    star_count: u64,
    forks_count: u64,
    visibility: String,
    repository: Option<GlRepository>,
    releases: Nodes<GlRelease>,
    issues: GlIssues,
    merge_requests: Nodes<GlMr>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct GlRepository {
    root_ref: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct GlRelease {
    tag_name: String,
    released_at: Option<String>,
    upcoming_release: bool,
}

#[derive(Deserialize)]
struct GlIssues {
    count: u64,
    nodes: Vec<GlIssue>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct GlIssue {
    iid: Id,
    title: String,
    web_url: String,
    created_at: String,
    labels: Option<Nodes<Title>>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct GlMr {
    iid: Id,
    title: String,
    web_url: String,
    draft: bool,
    author: Option<Username>,
    head_pipeline: Option<GlPipeline>,
}

#[derive(Deserialize)]
struct GlPipeline {
    status: String,
}

#[derive(Deserialize)]
struct GlCompare {
    commits: Vec<IgnoredAny>,
}

/// GitLab pipeline states (REST lower case, GraphQL upper case); None = no pipeline.
fn gitlab_state(status: Option<&str>) -> CheckState {
    let Some(s) = status else { return CheckState::NoChecks };
    match s.to_lowercase().as_str() {
        "success" => CheckState::Passed,
        "failed" => CheckState::Failed,
        "running" | "pending" | "created" | "preparing" | "waiting_for_resource" | "waiting_for_callback" => {
            CheckState::Pending
        }
        // ran nothing that judges the commit
        "canceled" | "canceling" | "skipped" | "manual" | "scheduled" => CheckState::NoChecks,
        other => CheckState::unknown(other),
    }
}

/// GITLAB_TOKEN is only sent to GITLAB_HOST (default gitlab.com), never to other hosts in the list.
fn gitlab_auth(net: &dyn Net, host: &str) -> Option<String> {
    let token = net.env("GITLAB_TOKEN")?;
    let want = net.env("GITLAB_HOST").unwrap_or_else(|| "gitlab.com".into());
    let want = want.strip_prefix("https://").unwrap_or(&want).trim_end_matches('/');
    (host == want).then(|| format!("Bearer {token}"))
}

pub(super) fn fetch(net: &dyn Net, full: &str) -> Res<Fetched> {
    let (host, path) = full.split_once('/').ok_or_else(|| ForgeError::NotFound("not host/group/project".into()))?;
    let auth = gitlab_auth(net, host);
    let api = |p: &str, body: Option<&Value>| net.http(&format!("https://{host}/api/{p}"), auth.as_deref(), body);
    let got: GlAnswer =
        parse("GitLab project", api("graphql", Some(&json!({"query": GITLAB_QUERY, "variables": {"p": path}})))?.json)?;
    let Some(r) = got.data.and_then(|d| d.project) else {
        let errs: Vec<String> = got.errors.into_iter().map(|e| e.message).collect();
        return Err(if errs.is_empty() {
            ForgeError::NotFound("project not found (private? set GITLAB_TOKEN)".into())
        } else {
            ForgeError::Refused(errs.join("; "))
        });
    };
    let branch = r.repository.and_then(|x| x.root_ref).filter(|b| !b.is_empty()).unwrap_or_else(|| "main".into());
    let pid = quote(path, "");
    let runs: Vec<GlPipeline> = parse(
        "GitLab pipelines",
        api(&format!("v4/projects/{pid}/pipelines?ref={}&per_page=1", quote(&branch, "/")), None)?.json,
    )?;
    let rel = r.releases.nodes.into_iter().next();
    let unreleased = rel.as_ref().and_then(|rel| {
        let p = format!(
            "v4/projects/{pid}/repository/compare?straight=true&from={}&to={}",
            quote(&rel.tag_name, "/"),
            quote(&branch, "/")
        );
        parse::<GlCompare>("compare", api(&p, None).ok()?.json).ok().map(|c| c.commits.len() as u64)
    });
    Ok(Fetched {
        url: r.web_url,
        pushed: r.last_activity_at.unwrap_or_default(),
        stars: r.star_count,
        forks: r.forks_count,
        ci: gitlab_state(runs.first().map(|x| x.status.as_str())),
        branch,
        pr: "MR !",
        unreleased,
        release: rel.map(|x| Release {
            tag: x.tag_name,
            when: x.released_at.unwrap_or_default(),
            pre: x.upcoming_release,
        }),
        prs: r
            .merge_requests
            .nodes
            .into_iter()
            .map(|m| Pr {
                ci: gitlab_state(m.head_pipeline.as_ref().map(|p| p.status.as_str())),
                n: m.iid.text(),
                title: m.title,
                url: Some(m.web_url),
                draft: m.draft,
                by: m.author.map_or_else(|| "?".into(), |a| a.username),
            })
            .collect(),
        issues: r
            .issues
            .nodes
            .into_iter()
            .map(|i| Issue {
                n: i.iid.text(),
                title: i.title,
                url: Some(i.web_url),
                created: i.created_at,
                labels: i.labels.map_or_else(Vec::new, |l| l.nodes.into_iter().map(|x| x.title).collect()),
            })
            .collect(),
        issue_total: r.issues.count,
        alerts: 0,
        private: r.visibility != "public",
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::forge::Reply;
    use crate::forge::fake::{Fake, reply};
    use crate::report::{Light, check_with};
    use std::sync::Mutex;

    fn gitlab_answer(url: &str) -> Reply {
        if url.ends_with("/graphql") {
            return reply(json!({"data": {"project": {
                "webUrl": "https://gitlab.com/g/p", "lastActivityAt": "2026-01-01T00:00:00Z", "starCount": 7,
                "forksCount": 2, "visibility": "internal", "repository": {"rootRef": "main"},
                "releases": {"nodes": [{"tagName": "v1", "releasedAt": "2026-01-01T00:00:00Z", "upcomingRelease": false}]},
                "issues": {"count": 1, "nodes": [{"iid": "4", "title": "bug", "webUrl": "https://gitlab.com/g/p/-/issues/4",
                    "createdAt": "2026-01-01T00:00:00.123Z", "labels": {"nodes": [{"title": "bug"}]}}]},
                "mergeRequests": {"nodes": [{"iid": "9", "title": "fix", "webUrl": "https://gitlab.com/g/p/-/merge_requests/9",
                    "draft": false, "author": {"username": "x"}, "headPipeline": {"status": "FAILED"}}]}}}}));
        }
        if url.contains("/pipelines") {
            return reply(json!([{"status": "success"}]));
        }
        reply(json!({"commits": [{}, {}]}))
    }

    #[test]
    fn gitlab_fills_the_card_and_sends_its_token_only_to_gitlab_host() {
        let net = Fake { sent: Mutex::new(vec![]), answer: gitlab_answer, env: vec![("GITLAB_TOKEN", "secret")] };
        let card = check_with(&net, "gitlab.com/g/p");
        let ok = card.ok().unwrap();
        assert_eq!((&ok.ci, ok.stars, ok.unreleased, ok.private), (&CheckState::Passed, 7, Some(2), true));
        assert!(card.todo.contains(&"MR !9 has failing checks".to_string()), "{:?}", card.todo);
        assert!(card.todo.contains(&"2 commit(s) on main since v1: release when ready".to_string()));
        assert_eq!(
            card.summary_lines()[1],
            (
                "MR !9 fix  [checks failure, by x]".to_string(),
                Some("https://gitlab.com/g/p/-/merge_requests/9".to_string())
            )
        );
        assert_eq!(card.light(), Light::Orange);
        assert!(net.sent.lock().unwrap().iter().all(|(_, a)| a.as_deref() == Some("Bearer secret")));
        net.sent.lock().unwrap().clear();
        check_with(&net, "git.example.org/g/p");
        let sent = net.sent.lock().unwrap();
        assert!(!sent.is_empty() && sent.iter().all(|(_, a)| a.is_none()), "{sent:?}");
    }

    #[test]
    fn pipeline_states() {
        assert_eq!(gitlab_state(Some("FAILED")), CheckState::Failed);
        assert_eq!(gitlab_state(Some("canceled")), CheckState::NoChecks);
        assert_eq!(gitlab_state(None), CheckState::NoChecks);
        assert_eq!(gitlab_state(Some("brand_new")), CheckState::Unknown("brand_new".into()));
    }
}
