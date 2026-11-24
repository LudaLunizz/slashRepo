//! One card per repo: what a forge returned, turned into a status line, PR/issue lines and to-dos.

use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::config::{CODEBERG, Sort, Tab};
use crate::forge::{self, Net};

pub struct Release {
    pub tag: String,
    /// ISO time from the forge; shown as an age.
    pub when: String,
    pub pre: bool,
}

pub struct Pr {
    pub n: String,
    pub title: String,
    pub url: Option<String>,
    pub draft: bool,
    pub by: String,
    pub ci: String,
}

pub struct Issue {
    pub n: String,
    pub title: String,
    pub url: Option<String>,
    pub created: String,
    pub labels: Vec<String>,
}

/// What one forge call returns. `ci` uses GitHub's words (SUCCESS, FAILURE, ERROR, PENDING, NONE).
pub struct Fetched {
    pub url: String,
    pub pushed: String,
    pub stars: u64,
    pub forks: u64,
    pub branch: String,
    /// "PR #" or "MR !" (GitLab)
    pub pr: &'static str,
    pub unreleased: Option<u64>,
    pub ci: String,
    pub release: Option<Release>,
    pub prs: Vec<Pr>,
    pub issues: Vec<Issue>,
    pub issue_total: u64,
    pub alerts: u64,
}

pub struct Card {
    pub repo: String,
    pub result: Result<Fetched, String>,
    pub todo: Vec<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Light {
    Grey,
    Green,
    Orange,
    Red,
}

impl Light {
    pub fn as_str(self) -> &'static str {
        match self {
            Light::Grey => "grey",
            Light::Green => "green",
            Light::Orange => "orange",
            Light::Red => "red",
        }
    }
}

const NOTHING: &str = "nothing to do";

fn red(ci: &str) -> bool {
    ci == "FAILURE" || ci == "ERROR"
}

impl Card {
    pub fn ok(&self) -> Option<&Fetched> {
        self.result.as_ref().ok()
    }

    pub fn light(&self) -> Light {
        match &self.result {
            Err(_) => Light::Red,
            Ok(d) if red(&d.ci) || d.alerts > 0 => Light::Red,
            Ok(_) if self.todo == [NOTHING] => Light::Green,
            Ok(_) => Light::Orange,
        }
    }

    pub fn stars(&self) -> Option<u64> {
        self.ok().map(|d| d.stars)
    }

    pub fn forks(&self) -> Option<u64> {
        self.ok().map(|d| d.forks)
    }

    /// The status line, then one line per PR and issue, each with its page (or None).
    pub fn summary_lines(&self) -> Vec<(String, Option<String>)> {
        let d = match &self.result {
            Err(e) => return vec![(format!("ERROR: {e}"), None)],
            Ok(d) => d,
        };
        let rel = match &d.release {
            Some(r) => format!("{}{}, {}", r.tag, if r.pre { " (pre-release)" } else { "" }, age(&r.when)),
            None => "no releases".into(),
        };
        let mut out = vec![(
            format!(
                "release: {rel}   |   {} CI: {}   |   last push {}   |   ★ {}  forks {}",
                d.branch,
                d.ci.to_lowercase(),
                age(&d.pushed),
                d.stars,
                d.forks
            ),
            None,
        )];
        for p in &d.prs {
            let draft = if p.draft { "draft, " } else { "" };
            let text = format!("{}{} {}  [{draft}checks {}, by {}]", d.pr, p.n, p.title, p.ci.to_lowercase(), p.by);
            out.push((text, p.url.clone()));
        }
        for i in &d.issues {
            let lab = if i.labels.is_empty() { String::new() } else { format!(" [{}]", i.labels.join(", ")) };
            out.push((format!("issue #{} {}  ({}){lab}", i.n, i.title, age(&i.created)), i.url.clone()));
        }
        let shown = d.issues.len() as u64;
        if d.issue_total > shown {
            out.push((format!("... and {} more issues", d.issue_total - shown), None));
        }
        out
    }
}

fn todo(d: &Fetched) -> Vec<String> {
    let mut todo = Vec::new();
    if red(&d.ci) {
        todo.push(format!("CI is red on {}: fix first", d.branch));
    }
    if d.alerts > 0 {
        todo.push(format!("{} open Dependabot security alert(s)", d.alerts));
    }
    for p in d.prs.iter().filter(|p| !p.draft) {
        if red(&p.ci) {
            todo.push(format!("{}{} has failing checks", d.pr, p.n));
        } else if p.ci == "SUCCESS" || p.ci == "NONE" {
            todo.push(format!("{}{} is green: review / merge", d.pr, p.n));
        }
    }
    if let (Some(n @ 1..), Some(rel)) = (d.unreleased, &d.release) {
        todo.push(format!("{n} commit(s) on {} since {}: release when ready", d.branch, rel.tag));
    }
    if !d.issues.is_empty() {
        todo.push(format!("{} open issue(s) to triage", d.issue_total));
    }
    if todo.is_empty() {
        todo.push(NOTHING.into());
    }
    todo
}

/// GitHub owners have no dots, hosts do.
fn host(full: &str) -> Option<&str> {
    let first = full.split('/').next().unwrap_or("");
    first.contains('.').then_some(first)
}

pub fn check_with(net: &dyn Net, full: &str) -> Card {
    let (got, hint) = match host(full) {
        None => (forge::github(net, full), "gh call failed: check `gh auth status`"),
        Some(CODEBERG) => {
            (forge::codeberg(net, full), "Codeberg call failed: check the name, or CODEBERG_TOKEN for a private repo")
        }
        Some(_) => {
            (forge::gitlab(net, full), "GitLab call failed: check the name, or GITLAB_TOKEN for a private project")
        }
    };
    match got {
        // one broken repo must not hide the others
        Err(e) => Card { repo: full.into(), result: Err(e), todo: vec![hint.into()] },
        Ok(d) => Card { repo: full.into(), todo: todo(&d), result: Ok(d) },
    }
}

/// Check every repo, 8 at a time.
pub fn check_all(repos: &[String]) -> HashMap<String, Card> {
    let net = forge::RealNet::new();
    let cards = crate::par_map(repos, 8, |r| check_with(&net, r));
    repos.iter().cloned().zip(cards).collect()
}

/// repos in the tab's order; stars/forks sort high to low, unknown (not checked yet) last.
pub fn ordered(repos: &[String], sort: Sort, cards: &HashMap<String, Card>) -> Vec<String> {
    let mut out = repos.to_vec();
    let key = |r: &String| -> i64 {
        let c = cards.get(r);
        let n = match sort {
            Sort::Stars => c.and_then(Card::stars),
            _ => c.and_then(Card::forks),
        };
        n.map_or(-1, |n| n as i64)
    };
    if sort != Sort::Mine {
        out.sort_by_key(|r| std::cmp::Reverse(key(r)));
    }
    out
}

/// The repo's page: GitLab/Codeberg lines carry their host, GitHub lines are owner/name.
pub fn web_url(repo: &str) -> String {
    match host(repo) {
        Some(_) => format!("https://{repo}"),
        None => format!("https://github.com/{repo}"),
    }
}

/// Seconds since 1970 for an ISO time like 2026-01-01T00:00:00(.123)(Z|+01:00).
fn unix_time(iso: &str) -> Option<i64> {
    let b = iso.as_bytes();
    let n = |a: usize, z: usize| -> Option<i64> { iso.get(a..z)?.parse().ok() };
    if b.len() < 19 || b[4] != b'-' || b[7] != b'-' || !matches!(b[10], b'T' | b't' | b' ') {
        return None;
    }
    let (y, m, d, hh, mm, ss) = (n(0, 4)?, n(5, 7)?, n(8, 10)?, n(11, 13)?, n(14, 16)?, n(17, 19)?);
    let mut rest = &iso[19..];
    if let Some(frac) = rest.strip_prefix('.') {
        rest = frac.trim_start_matches(|c: char| c.is_ascii_digit());
    }
    let offset = match rest {
        "" | "Z" | "z" => 0,
        _ => {
            let sign = match rest.as_bytes()[0] {
                b'+' => 1,
                b'-' => -1,
                _ => return None,
            };
            let (h, mi) = rest[1..].split_once(':')?;
            sign * (h.parse::<i64>().ok()? * 3600 + mi.parse::<i64>().ok()? * 60)
        }
    };
    // days from civil (Howard Hinnant)
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * ((m + 9) % 12) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146097 + doe - 719468;
    Some(days * 86400 + hh * 3600 + mm * 60 + ss - offset)
}

pub fn age_at(iso: &str, now: i64) -> String {
    match unix_time(iso) {
        None => "?".into(),
        Some(t) => match (now - t).div_euclid(86400) {
            0 => "today".into(),
            d => format!("{d}d ago"),
        },
    }
}

fn age(iso: &str) -> String {
    let now = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64);
    age_at(iso, now)
}

/// `repo-status --text [TAB]`
pub fn text_mode(tabs: &[Tab], only: Option<&str>) -> String {
    let cards = check_all(&crate::config::all_repos(tabs));
    let mut out = String::new();
    for tab in tabs.iter().filter(|t| only.is_none_or(|o| o == t.name)) {
        out += &format!("\n##### {}\n", tab.name);
        for repo in ordered(&tab.repos, tab.sort, &cards) {
            let c = &cards[&repo];
            out += &format!("\n== {}  [{}]\n", c.repo, c.light().as_str());
            for (line, _) in c.summary_lines() {
                out += &format!("   {line}\n");
            }
            for t in &c.todo {
                out += &format!("   TODO: {t}\n");
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn card(repo: &str, stars: u64, forks: u64) -> (String, Card) {
        let d = Fetched {
            url: String::new(),
            pushed: String::new(),
            stars,
            forks,
            branch: "main".into(),
            pr: "PR #",
            unreleased: None,
            ci: "SUCCESS".into(),
            release: None,
            prs: vec![],
            issues: vec![],
            issue_total: 0,
            alerts: 0,
        };
        (repo.into(), Card { repo: repo.into(), todo: todo(&d), result: Ok(d) })
    }

    #[test]
    fn order_by_stars_and_forks() {
        let cards: HashMap<_, _> = [card("a/1", 5, 9), card("a/2", 50, 1)].into_iter().collect();
        let all = ["a/1", "a/2", "a/3"].map(String::from);
        assert_eq!(ordered(&all, Sort::Stars, &cards), ["a/2", "a/1", "a/3"]);
        assert_eq!(ordered(&all, Sort::Forks, &cards), ["a/1", "a/2", "a/3"]);
        let two = ["a/2", "a/1"].map(String::from);
        assert_eq!(ordered(&two, Sort::Mine, &cards), ["a/2", "a/1"]);
        assert_eq!(cards["a/1"].light(), Light::Green);
    }

    #[test]
    fn web_urls() {
        assert_eq!(web_url("cli/cli"), "https://github.com/cli/cli");
        assert_eq!(web_url("gitlab.com/g/sub/p"), "https://gitlab.com/g/sub/p");
        assert_eq!(web_url("codeberg.org/o/r"), "https://codeberg.org/o/r");
    }

    #[test]
    fn ages() {
        let now = unix_time("2026-03-01T12:00:00Z").unwrap();
        assert_eq!(age_at("2026-03-01T01:00:00Z", now), "today");
        assert_eq!(age_at("2026-02-28T12:00:00.5Z", now), "1d ago");
        assert_eq!(age_at("2025-03-01T12:00:00+00:00", now), "365d ago");
        assert_eq!(age_at("2026-03-01T12:30:00+01:00", now), "today");
        assert_eq!(age_at("2026-02-27T13:00:00-02:00", now), "1d ago");
        assert_eq!(unix_time("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(unix_time("2000-03-01T00:00:00Z"), Some(951868800));
        assert_eq!(age_at("garbage", now), "?");
    }
}
