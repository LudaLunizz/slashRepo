//! GitHub, through `gh api graphql` (the user's own `gh` login; no token handled here).

use serde::Deserialize;

use super::{ForgeError, Id, Login, Name, Net, Nodes, Res, parse};
use crate::report::{CheckState, Fetched, Issue, Pr, Release};

const GITHUB_QUERY: &str = "query($o:String!,$n:String!){ repository(owner:$o,name:$n){
  url pushedAt stargazerCount forkCount isPrivate
  defaultBranchRef{ name target{ ... on Commit{ statusCheckRollup{state} } } }
  releases(first:1, orderBy:{field:CREATED_AT, direction:DESC}){ nodes{tagName publishedAt isPrerelease} }
  issues(states:OPEN, first:30, orderBy:{field:CREATED_AT, direction:DESC}){ totalCount nodes{number title url createdAt labels(first:3){nodes{name}}} }
  pullRequests(states:OPEN, first:30){ totalCount nodes{number title url isDraft reviewDecision author{login}
    commits(last:1){nodes{commit{statusCheckRollup{state}}}} } }
  vulnerabilityAlerts(states:OPEN){totalCount}
}}";

#[derive(Deserialize)]
struct GhAnswer {
    data: Option<GhData>,
}

#[derive(Deserialize)]
struct GhData {
    repository: Option<GhRepo>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct GhRepo {
    url: String,
    /// null for a repo nobody pushed to
    pushed_at: Option<String>,
    stargazer_count: u64,
    fork_count: u64,
    is_private: bool,
    /// null for an empty repo
    default_branch_ref: Option<GhBranch>,
    releases: Nodes<GhRelease>,
    issues: GhCounted<GhIssue>,
    pull_requests: GhCounted<GhPr>,
    /// null without the right to see the alerts
    vulnerability_alerts: Option<GhTotal>,
}

#[derive(Deserialize)]
struct GhBranch {
    name: String,
    target: Option<GhCommit>,
}

/// `... on Commit { statusCheckRollup }`: absent on a non-commit target, null without checks.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct GhCommit {
    status_check_rollup: Option<GhRollup>,
}

#[derive(Deserialize)]
struct GhRollup {
    state: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct GhTotal {
    total_count: u64,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct GhCounted<T> {
    total_count: u64,
    nodes: Vec<T>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct GhRelease {
    tag_name: String,
    published_at: Option<String>,
    is_prerelease: bool,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct GhIssue {
    number: Id,
    title: String,
    url: String,
    created_at: String,
    labels: Option<Nodes<Name>>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct GhPr {
    number: Id,
    title: String,
    url: String,
    is_draft: bool,
    /// null for a deleted account
    author: Option<Login>,
    commits: Nodes<GhPrCommit>,
}

#[derive(Deserialize)]
struct GhPrCommit {
    commit: GhCommit,
}

#[derive(Deserialize)]
struct GhCompare {
    ahead_by: u64,
}

/// GitHub's rollup states (null = no checks report on the commit).
fn github_state(rollup: Option<&GhRollup>) -> CheckState {
    match rollup.map(|r| r.state.as_str()) {
        None => CheckState::NoChecks,
        Some("SUCCESS") => CheckState::Passed,
        Some("FAILURE") => CheckState::Failed,
        Some("ERROR") => CheckState::Errored,
        Some("PENDING" | "EXPECTED") => CheckState::Pending,
        Some(other) => CheckState::unknown(other),
    }
}

fn rollup(c: Option<&GhCommit>) -> CheckState {
    github_state(c.and_then(|c| c.status_check_rollup.as_ref()))
}

pub(super) fn fetch(net: &dyn Net, full: &str) -> Res<Fetched> {
    let (owner, name) = full.split_once('/').ok_or_else(|| ForgeError::NotFound("not owner/name".into()))?;
    let q = format!("query={GITHUB_QUERY}");
    let (o, n) = (format!("o={owner}"), format!("n={name}"));
    // -f, not -F: -F would turn a name like 123 or true into a number or boolean, and @x into
    // the contents of file x. Owner and name are always strings.
    let got: GhAnswer = parse("GitHub repository", net.gh(&["api", "graphql", "-f", &q, "-f", &o, "-f", &n])?)?;
    let r = got.data.and_then(|d| d.repository).ok_or_else(|| ForgeError::NotFound("repository not found".into()))?;
    let (branch, ci) = match &r.default_branch_ref {
        Some(b) => (b.name.clone(), rollup(b.target.as_ref())),
        None => (String::new(), CheckState::NoChecks),
    };
    let rel = r.releases.nodes.into_iter().next();
    let unreleased = rel.as_ref().and_then(|rel| {
        let path = format!("repos/{full}/compare/{}...{branch}", rel.tag_name);
        parse::<GhCompare>("compare", net.gh(&["api", &path]).ok()?).ok().map(|c| c.ahead_by)
    });
    Ok(Fetched {
        url: r.url,
        pushed: r.pushed_at.unwrap_or_default(),
        stars: r.stargazer_count,
        forks: r.fork_count,
        ci,
        branch,
        pr: "PR #",
        unreleased,
        release: rel.map(|x| Release {
            tag: x.tag_name,
            when: x.published_at.unwrap_or_default(),
            pre: x.is_prerelease,
        }),
        prs: r
            .pull_requests
            .nodes
            .into_iter()
            .map(|p| Pr {
                ci: rollup(p.commits.nodes.first().map(|c| &c.commit)),
                n: p.number.text(),
                title: p.title,
                url: Some(p.url),
                draft: p.is_draft,
                by: p.author.map_or_else(|| "?".into(), |a| a.login),
            })
            .collect(),
        issues: r
            .issues
            .nodes
            .into_iter()
            .map(|i| Issue {
                n: i.number.text(),
                title: i.title,
                url: Some(i.url),
                created: i.created_at,
                labels: i.labels.map_or_else(Vec::new, |l| l.nodes.into_iter().map(|x| x.name).collect()),
            })
            .collect(),
        issue_total: r.issues.total_count,
        alerts: r.vulnerability_alerts.map_or(0, |a| a.total_count),
        private: r.is_private,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::forge::Reply;
    use crate::forge::fake::{fake, reply};
    use crate::report::{Light, check_with};
    use serde_json::json;

    fn github_answer(args: &str) -> Reply {
        if args.contains("/compare/") {
            return reply(json!({"ahead_by": 3, "status": "ahead"}));
        }
        reply(json!({"data": {"repository": {
            "url": "https://github.com/o/r", "pushedAt": "2026-01-01T00:00:00Z", "stargazerCount": 5,
            "forkCount": 1, "isPrivate": false,
            "defaultBranchRef": {"name": "main", "target": {"statusCheckRollup": {"state": "SUCCESS"}}},
            "releases": {"nodes": [{"tagName": "v1", "publishedAt": "2026-01-01T00:00:00Z", "isPrerelease": false}]},
            "issues": {"totalCount": 0, "nodes": []},
            "pullRequests": {"totalCount": 2, "nodes": [
                {"number": 7, "title": "new", "url": "https://github.com/o/r/pull/7", "isDraft": false,
                 "reviewDecision": null, "author": null, "commits": {"nodes": [{"commit": {"statusCheckRollup": {"state": "STALE"}}}]}},
                {"number": 8, "title": "wip", "url": "https://github.com/o/r/pull/8", "isDraft": false,
                 "reviewDecision": null, "author": {"login": "x"}, "commits": {"nodes": [{"commit": {"statusCheckRollup": null}}]}}]},
            "vulnerabilityAlerts": null}}}))
    }

    #[test]
    fn github_reads_typed_and_keeps_an_unknown_ci_state_visible() {
        let card = check_with(&fake(github_answer), "o/r");
        let ok = card.ok().unwrap();
        assert_eq!((&ok.ci, ok.unreleased, ok.alerts), (&CheckState::Passed, Some(3), 0));
        assert_eq!(ok.prs[0].ci, CheckState::Unknown("stale".into()));
        assert_eq!(ok.prs[0].by, "?");
        assert_eq!(ok.prs[1].ci, CheckState::NoChecks);
        assert_eq!(card.summary_lines()[1].0, "PR #7 new  [checks stale, by ?]");
        // an unknown state neither passes as green nor fails as red
        assert!(!card.todo.iter().any(|t| t.starts_with("PR #7")), "{:?}", card.todo);
        assert!(card.todo.contains(&"PR #8 is green: review / merge".to_string()));
    }

    fn github_wrong_shape(_: &str) -> Reply {
        // stargazerCount as text: an API change must not read as 0 stars
        reply(json!({"data": {"repository": {"url": "https://github.com/o/r", "stargazerCount": "many"}}}))
    }

    #[test]
    fn an_answer_of_another_shape_is_an_error_card() {
        let card = check_with(&fake(github_wrong_shape), "o/r");
        match &card.result {
            Err(ForgeError::BadAnswer(m)) => assert!(m.starts_with("GitHub repository:"), "{m}"),
            other => panic!("{:?}", other.as_ref().err()),
        }
        assert_eq!(card.light(), Light::Red);
        assert_eq!(card.todo, ["the forge changed its answer: please report this to slashRepo"]);
    }

    #[test]
    fn rollup_states() {
        assert_eq!(github_state(Some(&GhRollup { state: "EXPECTED".into() })), CheckState::Pending);
        assert_eq!(github_state(None), CheckState::NoChecks);
    }
}
