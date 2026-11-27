//! One card per repo: what a forge returned, turned into a status line, PR/issue lines and to-dos.

use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::config::{Sort, Tab};
use crate::forge::{self, Forge, ForgeError, Net};

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
    pub ci: CheckState,
}

pub struct Issue {
    pub n: String,
    pub title: String,
    pub url: Option<String>,
    pub created: String,
    pub labels: Vec<String>,
}

/// Where the checks on a commit stand, read once from each forge's own words (src/forge.rs).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CheckState {
    Passed,
    Failed,
    /// the check could not run (GitHub/Codeberg "error")
    Errored,
    Pending,
    /// nothing reports on the commit
    NoChecks,
    /// a state this program does not know yet, lower case; shown as is, never taken as green
    Unknown(String),
}

impl CheckState {
    pub fn unknown(word: &str) -> Self {
        CheckState::Unknown(word.to_lowercase())
    }

    /// The word on the card ("checks failure", "main CI: success").
    pub fn word(&self) -> &str {
        match self {
            CheckState::Passed => "success",
            CheckState::Failed => "failure",
            CheckState::Errored => "error",
            CheckState::Pending => "pending",
            CheckState::NoChecks => "none",
            CheckState::Unknown(w) => w,
        }
    }

    pub fn red(&self) -> bool {
        matches!(self, CheckState::Failed | CheckState::Errored)
    }

    /// Ready for review as far as checks go: passed, or nothing to wait for.
    pub fn clear(&self) -> bool {
        matches!(self, CheckState::Passed | CheckState::NoChecks)
    }
}

/// What one forge call returns.
pub struct Fetched {
    pub url: String,
    pub pushed: String,
    pub stars: u64,
    pub forks: u64,
    pub branch: String,
    /// "PR #" or "MR !" (GitLab)
    pub pr: &'static str,
    pub unreleased: Option<u64>,
    pub ci: CheckState,
    pub release: Option<Release>,
    pub prs: Vec<Pr>,
    /// open PRs (MRs) on the forge; more than `prs` when the list was cut
    pub pr_total: u64,
    pub issues: Vec<Issue>,
    pub issue_total: u64,
    pub alerts: u64,
    /// not public (GitHub private, GitLab private or internal, Codeberg private)
    pub private: bool,
}

pub struct Card {
    pub repo: String,
    pub result: Result<Fetched, ForgeError>,
    pub todo: Vec<String>,
}

/// Public or private, for the card's border and corner label; None while unknown (failed check).
pub fn visibility(c: Option<&Card>) -> Option<(&'static str, Light)> {
    let d = c?.ok()?;
    Some(if d.private { ("private", Light::Blue) } else { ("live", Light::Green) })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Light {
    Grey,
    /// only for the private-repo border, never a status
    Blue,
    Green,
    Orange,
    Red,
}

impl Light {
    pub fn as_str(self) -> &'static str {
        match self {
            Light::Grey => "grey",
            Light::Blue => "blue",
            Light::Green => "green",
            Light::Orange => "orange",
            Light::Red => "red",
        }
    }
}

const NOTHING: &str = "nothing to do";

impl Card {
    /// A card for a check that never came back (the worker crashed).
    pub fn crashed(repo: &str) -> Card {
        let err = ForgeError::Internal;
        Card { repo: repo.into(), todo: vec![err.fix(Forge::of(repo))], result: Err(err) }
    }

    /// A forge's answer (or its error) with the to-dos that follow from it.
    pub fn from_result(repo: &str, result: Result<Fetched, ForgeError>) -> Card {
        let todo = match &result {
            // one broken repo must not hide the others
            Err(e) => vec![e.fix(Forge::of(repo))],
            Ok(d) => todo(d),
        };
        Card { repo: repo.into(), todo, result }
    }

    /// Nothing asks for you on this repo.
    pub fn clear(&self) -> bool {
        self.result.is_ok() && self.todo == [NOTHING]
    }

    pub fn ok(&self) -> Option<&Fetched> {
        self.result.as_ref().ok()
    }

    pub fn light(&self) -> Light {
        match &self.result {
            Err(_) => Light::Red,
            Ok(d) if d.ci.red() || d.alerts > 0 => Light::Red,
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
                d.ci.word(),
                age(&d.pushed),
                d.stars,
                d.forks
            ),
            None,
        )];
        for p in &d.prs {
            let draft = if p.draft { "draft, " } else { "" };
            let text = format!("{}{} {}  [{draft}checks {}, by {}]", d.pr, p.n, p.title, p.ci.word(), p.by);
            out.push((text, p.url.clone()));
        }
        let shown_prs = d.prs.len() as u64;
        if d.pr_total > shown_prs {
            let what = if d.pr.starts_with("MR") { "merge requests" } else { "pull requests" };
            out.push((format!("... and {} more {what}", d.pr_total - shown_prs), None));
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
    if d.ci.red() {
        todo.push(format!("CI is red on {}: fix first", d.branch));
    } else if let CheckState::Unknown(w) = &d.ci {
        todo.push(format!("CI on {} says \"{w}\", a state slashRepo does not know: look on the forge", d.branch));
    }
    if d.alerts > 0 {
        todo.push(format!("{} open Dependabot security alert(s)", d.alerts));
    }
    for p in d.prs.iter().filter(|p| !p.draft) {
        if p.ci.red() {
            todo.push(format!("{}{} has failing checks", d.pr, p.n));
        } else if p.ci.clear() {
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

pub fn check_with(net: &dyn Net, full: &str) -> Card {
    let forge = Forge::of(full);
    // a panic while reading one repo's answer becomes that repo's error card, not a dead window or
    // a crashed `--text` run
    let got = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| forge.fetch(net, full)))
        .unwrap_or(Err(ForgeError::Internal));
    Card::from_result(full, got)
}

/// Check every repo, 8 at a time.
pub fn check_all(repos: &[String]) -> HashMap<String, Card> {
    if crate::demo::on() {
        return repos.iter().map(|r| (r.clone(), crate::demo::check(r))).collect();
    }
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

/// Stars and forks added up over repos (each repo once). Repos without an answer yet, or whose
/// check failed, are left out and counted in `missing`.
pub fn totals_line(repos: &[String], cards: &HashMap<String, Card>) -> String {
    let (mut stars, mut forks, mut seen, mut missing) = (0u64, 0u64, Vec::new(), 0usize);
    for r in repos {
        if seen.contains(&r) {
            continue;
        }
        seen.push(r);
        match cards.get(r).and_then(Card::ok) {
            Some(d) => {
                stars += d.stars;
                forks += d.forks;
            }
            None => missing += 1,
        }
    }
    let counted = seen.len() - missing;
    let mut out = format!("★ {stars} stars · {forks} forks in {counted} repo{}", if counted == 1 { "" } else { "s" });
    if missing > 0 {
        out += &format!(" ({missing} not counted: checking or failed)");
    }
    out
}

/// The repo's page: GitLab/Codeberg lines carry their host, GitHub lines are owner/name.
/// Only https links are opened from the window: a forge answer (or a hand-edited host) must not
/// be able to hand the desktop a `file:` or `javascript:` address.
pub fn openable(url: &str) -> bool {
    url.get(..8).is_some_and(|p| p.eq_ignore_ascii_case("https://")) && !url.contains(char::is_whitespace)
}

pub fn web_url(repo: &str) -> String {
    match forge::host(repo) {
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
        // a time ahead of this clock (clock skew) is "today", never "-1d ago"
        Some(t) => match (now - t).div_euclid(86400) {
            ..=0 => "today".into(),
            d => format!("{d}d ago"),
        },
    }
}

pub fn age(iso: &str) -> String {
    let now = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64);
    age_at(iso, now)
}

/// `repo-status --text [TAB]`
pub fn text_mode(tabs: &[Tab], only: Option<&str>) -> String {
    let cards = check_all(&crate::config::all_repos(tabs));
    let mut out = String::new();
    for tab in tabs.iter().filter(|t| only.is_none_or(|o| o == t.name)) {
        out += &format!("\n##### {}\n   {}\n", tab.name, totals_line(&tab.repos, &cards));
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

    #[test]
    fn visibility_word_and_colour() {
        let (_, mut c) = card("o/r", 0, 0);
        assert_eq!(visibility(Some(&c)), Some(("live", Light::Green)));
        if let Ok(d) = &mut c.result {
            d.private = true;
        }
        assert_eq!(visibility(Some(&c)), Some(("private", Light::Blue)));
        c.result = Err(ForgeError::Unreachable("down".into()));
        assert_eq!(visibility(Some(&c)), None);
        assert_eq!(visibility(None), None);
    }

    fn card(repo: &str, stars: u64, forks: u64) -> (String, Card) {
        let d = Fetched {
            url: String::new(),
            pushed: String::new(),
            stars,
            forks,
            branch: "main".into(),
            pr: "PR #",
            unreleased: None,
            ci: CheckState::Passed,
            release: None,
            prs: vec![],
            issues: vec![],
            issue_total: 0,
            pr_total: 0,
            alerts: 0,
            private: false,
        };
        (repo.into(), Card { repo: repo.into(), todo: todo(&d), result: Ok(d) })
    }

    #[test]
    fn totals_add_up_once_and_skip_missing() {
        let mut cards: HashMap<String, Card> = [card("a/x", 5, 2), card("a/y", 3, 1)].into_iter().collect();
        let v = |l: &[&str]| l.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(totals_line(&v(&["a/x", "a/y", "a/x"]), &cards), "★ 8 stars · 3 forks in 2 repos");
        assert_eq!(
            totals_line(&v(&["a/x", "a/z"]), &cards),
            "★ 5 stars · 2 forks in 1 repo (1 not counted: checking or failed)"
        );
        cards.insert("a/e".into(), Card { repo: "a/e".into(), result: Err(ForgeError::Internal), todo: vec![] });
        assert_eq!(
            totals_line(&v(&["a/e"]), &cards),
            "★ 0 stars · 0 forks in 0 repos (1 not counted: checking or failed)"
        );
        assert_eq!(totals_line(&[], &cards), "★ 0 stars · 0 forks in 0 repos");
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
        assert_eq!(age_at("2026-03-01T12:00:30Z", now), "today");
        assert_eq!(age_at("2026-03-03T12:00:00Z", now), "today");
    }

    /// A net whose every call panics, to prove one broken repo only costs its own card.
    struct Panics;
    impl Net for Panics {
        fn gh(&self, _: &[&str]) -> forge::Res<serde_json::Value> {
            panic!("boom");
        }
        fn http(&self, _: &str, _: Option<&str>, _: Option<&serde_json::Value>) -> forge::Res<forge::Reply> {
            panic!("boom");
        }
    }

    #[test]
    fn only_https_links_open() {
        assert!(openable("https://github.com/me/app/pull/3"));
        assert!(openable("HTTPS://codeberg.org/me/app"));
        for bad in ["http://github.com/x", "file:///etc/passwd", "javascript:alert(1)", "", "https://a b", "ssh://x"] {
            assert!(!openable(bad), "{bad}");
        }
    }

    #[test]
    fn a_panic_while_reading_becomes_an_error_card() {
        for repo in ["me/app", "codeberg.org/me/app", "gitlab.com/me/app"] {
            let card = check_with(&Panics, repo);
            assert_eq!(card.repo, repo);
            assert_eq!(card.result.as_ref().err(), Some(&ForgeError::Internal), "{repo}");
            assert_eq!(card.todo, ["press Refresh to try again"]);
        }
    }
}
