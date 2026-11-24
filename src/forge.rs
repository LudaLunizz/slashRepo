//! Fetching one repo from GitHub (through `gh`), GitLab (any host) or Codeberg.
//! All network access goes through [`Net`], so the tests can answer with fixed data.

use std::time::Duration;

use serde_json::{Value, json};

use crate::config::CODEBERG;
use crate::report::{Fetched, Issue, Pr, Release};

pub type Res<T> = Result<T, String>;

/// A response body as JSON plus the one header we read (`X-Total-Count`).
pub struct Reply {
    pub json: Value,
    pub total: Option<u64>,
}

pub trait Net: Sync {
    /// `gh ARGS...`, stdout parsed as JSON.
    fn gh(&self, args: &[&str]) -> Res<Value>;
    /// GET (body None) or POST json to url, with an optional Authorization header.
    fn http(&self, url: &str, auth: Option<&str>, body: Option<&Value>) -> Res<Reply>;
    /// An environment variable (tokens, GITLAB_HOST); empty counts as unset.
    fn env(&self, key: &str) -> Option<String> {
        std::env::var(key).ok().filter(|v| !v.is_empty())
    }
}

pub struct RealNet {
    agent: ureq::Agent,
}

impl RealNet {
    pub fn new() -> Self {
        let config =
            ureq::Agent::config_builder().timeout_global(Some(Duration::from_secs(60))).user_agent("slashRepo");
        // Windows: schannel and the Windows certificate store (no bundled roots in the exe)
        #[cfg(windows)]
        let config = config.tls_config(
            ureq::tls::TlsConfig::builder()
                .provider(ureq::tls::TlsProvider::NativeTls)
                .root_certs(ureq::tls::RootCerts::PlatformVerifier)
                .build(),
        );
        let agent = config.build().new_agent();
        RealNet { agent }
    }
}

/// The same 60 seconds the HTTPS calls get.
const GH_TIMEOUT: Duration = Duration::from_secs(60);

impl Net for RealNet {
    fn gh(&self, args: &[&str]) -> Res<Value> {
        let mut cmd = crate::command("gh");
        cmd.args(args);
        let out = crate::output_within(cmd, GH_TIMEOUT).map_err(|e| format!("gh: {e}"))?;
        if !out.status.success() {
            let err = String::from_utf8_lossy(&out.stderr).trim().to_string();
            return Err(if err.is_empty() { "gh failed".into() } else { err });
        }
        serde_json::from_slice(&out.stdout).map_err(|e| format!("gh output: {e}"))
    }

    fn http(&self, url: &str, auth: Option<&str>, body: Option<&Value>) -> Res<Reply> {
        let resp = match body {
            Some(b) => {
                let mut req = self.agent.post(url).header("Content-Type", "application/json");
                if let Some(a) = auth {
                    req = req.header("Authorization", a);
                }
                req.send(b.to_string())
            }
            None => {
                let mut req = self.agent.get(url);
                if let Some(a) = auth {
                    req = req.header("Authorization", a);
                }
                req.call()
            }
        };
        let mut resp = resp.map_err(|e| e.to_string())?;
        let total = resp.headers().get("x-total-count").and_then(|v| v.to_str().ok()).and_then(|v| v.parse().ok());
        let text = resp.body_mut().read_to_string().map_err(|e| e.to_string())?;
        let json = serde_json::from_str(&text).map_err(|e| format!("bad JSON from {url}: {e}"))?;
        Ok(Reply { json, total })
    }
}

/// Percent-encode everything except unreserved characters and the ones in `safe`.
pub fn quote(s: &str, safe: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || b"_.-~".contains(&b) || safe.as_bytes().contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

fn st(v: &Value, ptr: &str) -> String {
    v.pointer(ptr).and_then(Value::as_str).unwrap_or_default().to_string()
}

fn num(v: &Value, ptr: &str) -> u64 {
    v.pointer(ptr).and_then(Value::as_u64).unwrap_or(0)
}

/// Issue and MR numbers: GitHub/Codeberg send numbers, GitLab's GraphQL sends strings.
fn id(v: &Value, ptr: &str) -> String {
    match v.pointer(ptr) {
        Some(Value::String(s)) => s.clone(),
        Some(n @ Value::Number(_)) => n.to_string(),
        _ => "?".into(),
    }
}

fn opt_str(v: &Value, ptr: &str) -> Option<String> {
    v.pointer(ptr).and_then(Value::as_str).map(String::from)
}

fn list<'a>(v: &'a Value, ptr: &str) -> &'a [Value] {
    v.pointer(ptr).and_then(Value::as_array).map_or(&[], Vec::as_slice)
}

fn labels(v: &Value, ptr: &str, field: &str) -> Vec<String> {
    list(v, ptr).iter().filter_map(|l| l.get(field)?.as_str().map(String::from)).collect()
}

const GITHUB_QUERY: &str = "query($o:String!,$n:String!){ repository(owner:$o,name:$n){
  url pushedAt stargazerCount forkCount
  defaultBranchRef{ name target{ ... on Commit{ statusCheckRollup{state} } } }
  releases(first:1, orderBy:{field:CREATED_AT, direction:DESC}){ nodes{tagName publishedAt isPrerelease} }
  issues(states:OPEN, first:30, orderBy:{field:CREATED_AT, direction:DESC}){ totalCount nodes{number title url createdAt labels(first:3){nodes{name}}} }
  pullRequests(states:OPEN, first:30){ totalCount nodes{number title url isDraft reviewDecision author{login}
    commits(last:1){nodes{commit{statusCheckRollup{state}}}} } }
  vulnerabilityAlerts(states:OPEN){totalCount}
}}";

fn state_or_none(v: &Value, ptr: &str) -> String {
    opt_str(v, ptr).unwrap_or_else(|| "NONE".into())
}

pub fn github(net: &dyn Net, full: &str) -> Res<Fetched> {
    let (owner, name) = full.split_once('/').ok_or("not owner/name")?;
    let q = format!("query={GITHUB_QUERY}");
    let (o, n) = (format!("o={owner}"), format!("n={name}"));
    let got = net.gh(&["api", "graphql", "-f", &q, "-F", &o, "-F", &n])?;
    let r = got.pointer("/data/repository").filter(|r| !r.is_null()).ok_or("repository not found")?;
    let branch = st(r, "/defaultBranchRef/name");
    let rel = list(r, "/releases/nodes").first();
    let unreleased = rel.and_then(|rel| {
        let path = format!("repos/{full}/compare/{}...{branch}", st(rel, "/tagName"));
        net.gh(&["api", &path]).ok()?.get("ahead_by")?.as_u64()
    });
    Ok(Fetched {
        url: st(r, "/url"),
        pushed: st(r, "/pushedAt"),
        stars: num(r, "/stargazerCount"),
        forks: num(r, "/forkCount"),
        ci: state_or_none(r, "/defaultBranchRef/target/statusCheckRollup/state"),
        branch,
        pr: "PR #",
        unreleased,
        release: rel.map(|x| Release {
            tag: st(x, "/tagName"),
            when: st(x, "/publishedAt"),
            pre: x.get("isPrerelease").and_then(Value::as_bool).unwrap_or(false),
        }),
        prs: list(r, "/pullRequests/nodes")
            .iter()
            .map(|p| Pr {
                n: id(p, "/number"),
                title: st(p, "/title"),
                url: opt_str(p, "/url"),
                draft: p.get("isDraft").and_then(Value::as_bool).unwrap_or(false),
                by: opt_str(p, "/author/login").unwrap_or_else(|| "?".into()),
                ci: state_or_none(p, "/commits/nodes/0/commit/statusCheckRollup/state"),
            })
            .collect(),
        issues: list(r, "/issues/nodes")
            .iter()
            .map(|i| Issue {
                n: id(i, "/number"),
                title: st(i, "/title"),
                url: opt_str(i, "/url"),
                created: st(i, "/createdAt"),
                labels: labels(i, "/labels/nodes", "name"),
            })
            .collect(),
        issue_total: num(r, "/issues/totalCount"),
        alerts: num(r, "/vulnerabilityAlerts/totalCount"),
    })
}

const GITLAB_QUERY: &str = "query($p:ID!){ project(fullPath:$p){
  webUrl lastActivityAt starCount forksCount repository{rootRef}
  releases(first:1, sort:RELEASED_AT_DESC){ nodes{tagName releasedAt upcomingRelease} }
  issues(state:opened, first:30, sort:CREATED_DESC){ count nodes{iid title webUrl createdAt labels(first:3){nodes{title}}} }
  mergeRequests(state:opened, first:30){ nodes{iid title webUrl draft author{username} headPipeline{status}} }
}}";

/// GitLab pipeline states -> the GitHub words the rest of the program uses.
fn pipeline(status: &str) -> String {
    match status {
        "success" => "SUCCESS",
        "failed" => "FAILURE",
        "running" | "pending" | "created" | "preparing" | "waiting_for_resource" => "PENDING",
        _ => "NONE",
    }
    .into()
}

/// GITLAB_TOKEN is only sent to GITLAB_HOST (default gitlab.com), never to other hosts in the list.
fn gitlab_auth(net: &dyn Net, host: &str) -> Option<String> {
    let token = net.env("GITLAB_TOKEN")?;
    let want = net.env("GITLAB_HOST").unwrap_or_else(|| "gitlab.com".into());
    let want = want.strip_prefix("https://").unwrap_or(&want).trim_end_matches('/');
    (host == want).then(|| format!("Bearer {token}"))
}

pub fn gitlab(net: &dyn Net, full: &str) -> Res<Fetched> {
    let (host, path) = full.split_once('/').ok_or("not host/group/project")?;
    let auth = gitlab_auth(net, host);
    let api = |p: &str, body: Option<&Value>| net.http(&format!("https://{host}/api/{p}"), auth.as_deref(), body);
    let got = api("graphql", Some(&json!({"query": GITLAB_QUERY, "variables": {"p": path}})))?.json;
    let Some(r) = got.pointer("/data/project").filter(|r| !r.is_null()) else {
        let errs: Vec<String> = list(&got, "/errors").iter().map(|e| st(e, "/message")).collect();
        let errs = errs.join("; ");
        return Err(if errs.is_empty() { "project not found (private? set GITLAB_TOKEN)".into() } else { errs });
    };
    let branch = opt_str(r, "/repository/rootRef").filter(|b| !b.is_empty()).unwrap_or_else(|| "main".into());
    let pid = quote(path, "");
    let runs = api(&format!("v4/projects/{pid}/pipelines?ref={}&per_page=1", quote(&branch, "/")), None)?.json;
    let rel = list(r, "/releases/nodes").first();
    let unreleased = rel.and_then(|rel| {
        let p = format!(
            "v4/projects/{pid}/repository/compare?straight=true&from={}&to={}",
            quote(&st(rel, "/tagName"), "/"),
            quote(&branch, "/")
        );
        Some(api(&p, None).ok()?.json.get("commits")?.as_array()?.len() as u64)
    });
    Ok(Fetched {
        url: st(r, "/webUrl"),
        pushed: st(r, "/lastActivityAt"),
        stars: num(r, "/starCount"),
        forks: num(r, "/forksCount"),
        ci: list(&runs, "").first().map_or_else(|| "NONE".into(), |x| pipeline(&st(x, "/status"))),
        branch,
        pr: "MR !",
        unreleased,
        release: rel.map(|x| Release {
            tag: st(x, "/tagName"),
            when: st(x, "/releasedAt"),
            pre: x.get("upcomingRelease").and_then(Value::as_bool).unwrap_or(false),
        }),
        prs: list(r, "/mergeRequests/nodes")
            .iter()
            .map(|m| Pr {
                n: id(m, "/iid"),
                title: st(m, "/title"),
                url: opt_str(m, "/webUrl"),
                draft: m.get("draft").and_then(Value::as_bool).unwrap_or(false),
                by: opt_str(m, "/author/username").unwrap_or_else(|| "?".into()),
                ci: pipeline(&st(m, "/headPipeline/status").to_lowercase()),
            })
            .collect(),
        issues: list(r, "/issues/nodes")
            .iter()
            .map(|i| Issue {
                n: id(i, "/iid"),
                title: st(i, "/title"),
                url: opt_str(i, "/webUrl"),
                created: st(i, "/createdAt"),
                labels: labels(i, "/labels/nodes", "title"),
            })
            .collect(),
        issue_total: num(r, "/issues/count"),
        alerts: 0,
    })
}

/// Only Codeberg itself; self-hosted Forgejo/Gitea would need a host list.
pub fn codeberg(net: &dyn Net, full: &str) -> Res<Fetched> {
    // CODEBERG_TOKEN only ever goes to codeberg.org.
    let auth = net.env("CODEBERG_TOKEN").map(|t| format!("token {t}"));
    let base = format!("repos/{}", quote(full.split_once('/').map_or("", |x| x.1), "/"));
    let api = |p: &str| net.http(&format!("https://{CODEBERG}/api/v1/{p}"), auth.as_deref(), None);
    let r = api(&base)?.json;
    let branch = st(&r, "/default_branch");
    let has = |k: &str| r.get(k).and_then(Value::as_bool).unwrap_or(true);

    // combined commit status; NONE when no CI reports on it
    let ci = |sha: &str| -> String {
        let s = api(&format!("{base}/commits/{}/status", quote(sha, ""))).map(|x| st(&x.json, "/state"));
        match s.as_deref() {
            Ok("success") => "SUCCESS",
            Ok("failure") => "FAILURE",
            Ok("error") => "ERROR",
            Ok("pending") => "PENDING",
            _ => "NONE",
        }
        .into()
    };

    // a repo can switch releases, issues or PRs off; asking anyway gives a 404
    let rels = if has("has_releases") { api(&format!("{base}/releases?limit=1&draft=false"))?.json } else { json!([]) };
    let rel = list(&rels, "").first().cloned();
    let unreleased = rel.as_ref().and_then(|rel| {
        // count only (compare times out on big repos): commits on branch, not on the tag
        let p = format!(
            "{base}/commits?sha={}&not={}&limit=1&stat=false&verification=false&files=false",
            quote(&branch, "/"),
            quote(&st(rel, "/tag_name"), "/")
        );
        api(&p).ok()?.total
    });
    let (issues, issue_head) = if has("has_issues") {
        let x = api(&format!("{base}/issues?state=open&type=issues&limit=30"))?;
        (x.json, x.total)
    } else {
        (json!([]), None)
    };
    let pulls =
        if has("has_pull_requests") { api(&format!("{base}/pulls?state=open&limit=30"))?.json } else { json!([]) };
    let pulls = list(&pulls, "");
    // the list has no check state: one small call per PR, 4 at a time
    let pr_ci = crate::par_map(pulls, 4, |p| {
        if p.get("draft").and_then(Value::as_bool).unwrap_or(false) { "NONE".into() } else { ci(&st(p, "/head/sha")) }
    });
    let issues = list(&issues, "");
    Ok(Fetched {
        url: st(&r, "/html_url"),
        pushed: st(&r, "/updated_at"),
        stars: num(&r, "/stars_count"),
        forks: num(&r, "/forks_count"),
        ci: ci(&branch),
        branch,
        pr: "PR #",
        unreleased,
        release: rel.map(|x| Release {
            tag: st(&x, "/tag_name"),
            when: st(&x, "/published_at"),
            pre: x.get("prerelease").and_then(Value::as_bool).unwrap_or(false),
        }),
        prs: pulls
            .iter()
            .zip(pr_ci)
            .map(|(p, c)| Pr {
                n: id(p, "/number"),
                title: st(p, "/title"),
                url: opt_str(p, "/html_url"),
                draft: p.get("draft").and_then(Value::as_bool).unwrap_or(false),
                by: opt_str(p, "/user/login").unwrap_or_else(|| "?".into()),
                ci: c,
            })
            .collect(),
        issue_total: issue_head.unwrap_or(issues.len() as u64),
        issues: issues
            .iter()
            .map(|i| Issue {
                n: id(i, "/number"),
                title: st(i, "/title"),
                url: opt_str(i, "/html_url"),
                created: st(i, "/created_at"),
                labels: labels(i, "/labels", "name"),
            })
            .collect(),
        alerts: 0,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::report::{Light, check_with};
    use std::sync::Mutex;

    /// Answers by URL and records (url, Authorization) of every call.
    struct Fake {
        sent: Mutex<Vec<(String, Option<String>)>>,
        answer: fn(&str) -> Reply,
        env: Vec<(&'static str, &'static str)>,
    }

    impl Net for Fake {
        fn gh(&self, _: &[&str]) -> Res<Value> {
            Err("no gh in tests".into())
        }
        fn http(&self, url: &str, auth: Option<&str>, _: Option<&Value>) -> Res<Reply> {
            self.sent.lock().unwrap().push((url.into(), auth.map(String::from)));
            Ok((self.answer)(url))
        }
        fn env(&self, key: &str) -> Option<String> {
            self.env.iter().find(|(k, _)| *k == key).map(|(_, v)| v.to_string())
        }
    }

    fn reply(json: Value) -> Reply {
        Reply { json, total: None }
    }

    fn gitlab_answer(url: &str) -> Reply {
        if url.ends_with("/graphql") {
            return reply(json!({"data": {"project": {
                "webUrl": "https://gitlab.com/g/p", "lastActivityAt": "2026-01-01T00:00:00Z", "starCount": 7,
                "forksCount": 2, "repository": {"rootRef": "main"},
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

    fn codeberg_answer(url: &str) -> Reply {
        let u = url.split_once("/api/v1/repos/o/r").unwrap().1;
        let (json, total) = if u.is_empty() {
            (
                json!({"html_url": "https://codeberg.org/o/r", "updated_at": "2026-01-01T00:00:00+01:00",
                    "stars_count": 3, "forks_count": 1, "default_branch": "main"}),
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
    fn gitlab_fills_the_card_and_sends_its_token_only_to_gitlab_host() {
        let net = Fake { sent: Mutex::new(vec![]), answer: gitlab_answer, env: vec![("GITLAB_TOKEN", "secret")] };
        let card = check_with(&net, "gitlab.com/g/p");
        let ok = card.ok().unwrap();
        assert_eq!((ok.ci.as_str(), ok.stars, ok.unreleased), ("SUCCESS", 7, Some(2)));
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
    fn codeberg_fills_the_card_and_sends_only_its_own_token() {
        let env = vec![("GITLAB_TOKEN", "gl"), ("CODEBERG_TOKEN", "cb")];
        let net = Fake { sent: Mutex::new(vec![]), answer: codeberg_answer, env };
        let card = check_with(&net, "codeberg.org/o/r");
        let ok = card.ok().unwrap();
        assert_eq!((ok.ci.as_str(), ok.stars, ok.issue_total, ok.unreleased), ("SUCCESS", 3, 12, Some(4)));
        assert!(card.todo.contains(&"PR #6 has failing checks".to_string()), "{:?}", card.todo);
        assert!(card.todo.contains(&"12 open issue(s) to triage".to_string()));
        let sent = net.sent.lock().unwrap();
        assert!(!sent.is_empty() && sent.iter().all(|(_, a)| a.as_deref() == Some("token cb")), "{sent:?}");
        assert_eq!(card.summary_lines()[1].1.as_deref(), Some("https://codeberg.org/o/r/pulls/6"));
    }

    #[test]
    fn quoting() {
        assert_eq!(quote("g/sub/p", ""), "g%2Fsub%2Fp");
        assert_eq!(quote("feature/x y", "/"), "feature/x%20y");
    }
}
