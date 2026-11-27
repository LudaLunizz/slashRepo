//! Codeberg (Forgejo API v1); only codeberg.org itself, CODEBERG_TOKEN goes nowhere else.

use serde::Deserialize;

use super::{ForgeError, Id, Login, Name, Net, Res, parse, quote};
use crate::config::CODEBERG;
use crate::report::{CheckState, Fetched, Issue, Pr, Release};

fn yes() -> bool {
    true
}

#[derive(Deserialize)]
struct CbRepo {
    html_url: String,
    updated_at: Option<String>,
    stars_count: u64,
    forks_count: u64,
    default_branch: String,
    private: bool,
    // a repo can switch these off; asking anyway gives a 404
    #[serde(default = "yes")]
    has_releases: bool,
    #[serde(default = "yes")]
    has_issues: bool,
    #[serde(default = "yes")]
    has_pull_requests: bool,
}

#[derive(Deserialize)]
struct CbRelease {
    tag_name: String,
    published_at: Option<String>,
    #[serde(default)]
    prerelease: bool,
}

#[derive(Deserialize)]
struct CbIssue {
    number: Id,
    title: String,
    html_url: Option<String>,
    created_at: String,
    #[serde(default)]
    labels: Vec<Name>,
}

#[derive(Deserialize)]
struct CbPull {
    number: Id,
    title: String,
    html_url: Option<String>,
    #[serde(default)]
    draft: bool,
    user: Option<Login>,
    head: CbHead,
}

#[derive(Deserialize)]
struct CbHead {
    sha: String,
}

#[derive(Deserialize)]
struct CbStatus {
    #[serde(default)]
    state: String,
}

/// Forgejo's combined commit status ("" = nothing reports on the commit).
fn codeberg_state(state: &str) -> CheckState {
    match state {
        "" => CheckState::NoChecks,
        "success" => CheckState::Passed,
        "failure" => CheckState::Failed,
        "error" => CheckState::Errored,
        "pending" => CheckState::Pending,
        other => CheckState::unknown(other),
    }
}

/// Only Codeberg itself; self-hosted Forgejo/Gitea would need a host list.
pub(super) fn fetch(net: &dyn Net, full: &str) -> Res<Fetched> {
    // CODEBERG_TOKEN only ever goes to codeberg.org.
    let auth = net.env("CODEBERG_TOKEN").map(|t| format!("token {t}"));
    let base = format!("repos/{}", quote(full.split_once('/').map_or("", |x| x.1), "/"));
    let api = |p: &str| net.http(&format!("https://{CODEBERG}/api/v1/{p}"), auth.as_deref(), None);
    let r: CbRepo = parse("Codeberg repo", api(&base)?.json)?;
    let branch = r.default_branch.clone();

    // combined commit status; a commit the forge does not know (empty repo) has no checks
    let ci = |sha: &str| -> Res<CheckState> {
        match api(&format!("{base}/commits/{}/status", quote(sha, ""))) {
            Ok(x) => Ok(codeberg_state(&parse::<CbStatus>("commit status", x.json)?.state)),
            Err(ForgeError::NotFound(_)) => Ok(CheckState::NoChecks),
            Err(e) => Err(e),
        }
    };

    let rels: Vec<CbRelease> = if r.has_releases {
        parse("Codeberg releases", api(&format!("{base}/releases?limit=1&draft=false"))?.json)?
    } else {
        vec![]
    };
    let rel = rels.into_iter().next();
    let unreleased = rel.as_ref().and_then(|rel| {
        // count only (compare times out on big repos): commits on branch, not on the tag
        let p = format!(
            "{base}/commits?sha={}&not={}&limit=1&stat=false&verification=false&files=false",
            quote(&branch, "/"),
            quote(&rel.tag_name, "/")
        );
        api(&p).ok()?.total
    });
    let (issues, issue_head): (Vec<CbIssue>, _) = if r.has_issues {
        let x = api(&format!("{base}/issues?state=open&type=issues&limit=30"))?;
        (parse("Codeberg issues", x.json)?, x.total)
    } else {
        (vec![], None)
    };
    let (pulls, pull_head): (Vec<CbPull>, _) = if r.has_pull_requests {
        let x = api(&format!("{base}/pulls?state=open&limit=30"))?;
        (parse("Codeberg pulls", x.json)?, x.total)
    } else {
        (vec![], None)
    };
    let pr_total = pull_head.unwrap_or(0).max(pulls.len() as u64);
    // the list has no check state: one small call per PR, 4 at a time
    let pr_ci = crate::par_map(&pulls, 4, |p| if p.draft { Ok(CheckState::NoChecks) } else { ci(&p.head.sha) });
    let pr_ci = pr_ci.into_iter().collect::<Res<Vec<_>>>()?;
    Ok(Fetched {
        url: r.html_url,
        pushed: r.updated_at.unwrap_or_default(),
        stars: r.stars_count,
        forks: r.forks_count,
        ci: ci(&branch)?,
        branch,
        pr: "PR #",
        unreleased,
        release: rel.map(|x| Release { tag: x.tag_name, when: x.published_at.unwrap_or_default(), pre: x.prerelease }),
        prs: pulls
            .into_iter()
            .zip(pr_ci)
            .map(|(p, ci)| Pr {
                n: p.number.text(),
                title: p.title,
                url: p.html_url,
                draft: p.draft,
                by: p.user.map_or_else(|| "?".into(), |u| u.login),
                ci,
            })
            .collect(),
        pr_total,
        issue_total: issue_head.unwrap_or(issues.len() as u64),
        issues: issues
            .into_iter()
            .map(|i| Issue {
                n: i.number.text(),
                title: i.title,
                url: i.html_url,
                created: i.created_at,
                labels: i.labels.into_iter().map(|l| l.name).collect(),
            })
            .collect(),
        alerts: 0,
        private: r.private,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::forge::Reply;
    use crate::forge::fake::Fake;
    use crate::report::check_with;
    use serde_json::json;
    use std::sync::Mutex;

    fn codeberg_answer(url: &str) -> Reply {
        let u = url.split_once("/api/v1/repos/o/r").unwrap().1;
        let (json, total) = if u.is_empty() {
            (
                json!({"html_url": "https://codeberg.org/o/r", "updated_at": "2026-01-01T00:00:00+01:00",
                    "stars_count": 3, "forks_count": 1, "default_branch": "main", "private": false}),
                None,
            )
        } else if u.starts_with("/releases") {
            (json!([{"tag_name": "v2", "published_at": "2026-01-01T00:00:00+01:00", "prerelease": false}]), None)
        } else if u.starts_with("/commits?sha=main&not=v2") {
            (json!([{}]), Some(4))
        } else if u.starts_with("/issues") {
            (
                json!([{"number": 5, "title": "bug", "created_at": "2026-01-01T00:00:00+01:00",
                     "labels": [{"name": "bug"}]}]),
                Some(12),
            )
        } else if u.starts_with("/pulls") {
            (
                json!([{"number": 6, "title": "fix", "html_url": "https://codeberg.org/o/r/pulls/6", "draft": false,
                     "user": {"login": "x"}, "head": {"sha": "abc"}}]),
                None,
            )
        } else if u == "/commits/abc/status" {
            (json!({"state": "failure"}), None)
        } else {
            (json!({"state": "success"}), None) // the default branch
        };
        Reply { json, total }
    }

    #[test]
    fn codeberg_fills_the_card_and_sends_only_its_own_token() {
        let env = vec![("GITLAB_TOKEN", "gl"), ("CODEBERG_TOKEN", "cb")];
        let net = Fake { sent: Mutex::new(vec![]), answer: codeberg_answer, env };
        let card = check_with(&net, "codeberg.org/o/r");
        let ok = card.ok().unwrap();
        assert_eq!(
            (&ok.ci, ok.stars, ok.issue_total, ok.unreleased, ok.private),
            (&CheckState::Passed, 3, 12, Some(4), false)
        );
        assert!(card.todo.contains(&"PR #6 has failing checks".to_string()), "{:?}", card.todo);
        assert!(card.todo.contains(&"12 open issue(s) to triage".to_string()));
        let sent = net.sent.lock().unwrap();
        assert!(!sent.is_empty() && sent.iter().all(|(_, a)| a.as_deref() == Some("token cb")), "{sent:?}");
        assert_eq!(card.summary_lines()[1].1.as_deref(), Some("https://codeberg.org/o/r/pulls/6"));
    }

    #[test]
    fn status_states() {
        assert_eq!(codeberg_state("warning"), CheckState::Unknown("warning".into()));
        assert_eq!(codeberg_state(""), CheckState::NoChecks);
    }
}
