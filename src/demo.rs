//! `repo-status --demo`: the window with made-up repos. Nothing is read from a forge and nothing
//! is written to your repos file (edits change a list in memory), so it is safe to click around
//! in, and it is how the README pictures are taken.

use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::forge::ForgeError;
use crate::report::{Card, CheckState, Fetched, Issue, Pr, Release};

static ON: AtomicBool = AtomicBool::new(false);
static LINES: Mutex<Vec<String>> = Mutex::new(Vec::new());

const LIST: &str = "\
you/rocket-launcher
you/tiny-parser
you/weather-cli
you/dotfiles

[Watching]
sort = stars
octo-org/big-project
gitlab.com/acme/widgets
codeberg.org/acme/atlas
";

pub fn enable() {
    ON.store(true, Ordering::Relaxed);
    set_lines(LIST.lines().map(|l| format!("{l}\n")).collect());
}

pub fn on() -> bool {
    ON.load(Ordering::Relaxed)
}

/// The demo's repos file (it lives in memory only).
pub fn lines() -> Vec<String> {
    LINES.lock().map(|l| l.clone()).unwrap_or_default()
}

pub fn set_lines(lines: Vec<String>) {
    if let Ok(mut l) = LINES.lock() {
        *l = lines;
    }
}

/// An ISO time `days` days before now (the cards show ages, so the demo never looks stale).
fn ago(days: i64) -> String {
    let now = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64);
    // civil from days (Howard Hinnant)
    let z = (now - days * 86400).div_euclid(86400) + 719468;
    let era = z.div_euclid(146097);
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let (d, m) = (doy - (153 * mp + 2) / 5 + 1, if mp < 10 { mp + 3 } else { mp - 9 });
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}-{m:02}-{d:02}T00:00:00Z")
}

fn page(repo: &str) -> String {
    format!("https://example.com/{repo}")
}

fn pr(repo: &str, n: u32, title: &str, by: &str, ci: CheckState, draft: bool) -> Pr {
    Pr {
        n: n.to_string(),
        title: title.into(),
        url: Some(format!("{}/pull/{n}", page(repo))),
        draft,
        by: by.into(),
        ci,
    }
}

fn issue(repo: &str, n: u32, title: &str, days: i64, labels: &[&str]) -> Issue {
    Issue {
        n: n.to_string(),
        title: title.into(),
        url: Some(format!("{}/issues/{n}", page(repo))),
        created: ago(days),
        labels: labels.iter().map(|l| l.to_string()).collect(),
    }
}

fn release(tag: &str, days: i64) -> Option<Release> {
    Some(Release { tag: tag.into(), when: ago(days), pre: false })
}

/// What a forge "answered" for a demo repo; a repo added by hand in the demo gets a not-found card.
fn fetched(repo: &str) -> Result<Fetched, ForgeError> {
    use CheckState::{Failed, NoChecks, Passed, Pending};
    let mut d = Fetched {
        url: page(repo),
        pushed: ago(0),
        stars: 0,
        forks: 0,
        branch: "main".into(),
        pr: "PR #",
        unreleased: Some(0),
        ci: Passed,
        release: None,
        prs: vec![],
        issues: vec![],
        issue_total: 0,
        pr_total: 0,
        alerts: 0,
        private: false,
    };
    match repo {
        "you/rocket-launcher" => {
            (d.stars, d.forks, d.pushed, d.release) = (312, 40, ago(3), release("v1.4.2", 3));
        }
        "you/tiny-parser" => {
            (d.stars, d.forks, d.release, d.unreleased) = (58, 7, release("v0.9.0", 12), Some(4));
            d.prs = vec![pr(repo, 17, "Support trailing commas", "a-friend", Passed, false)];
            d.issues = vec![issue(repo, 14, "Crash on empty input", 6, &["bug"])];
        }
        "you/weather-cli" => {
            (d.stars, d.forks, d.release, d.ci, d.alerts) = (9, 1, release("v2.0.1", 30), Failed, 1);
            d.prs = vec![pr(repo, 31, "Bump the HTTP client to 3.2", "dependabot", Pending, false)];
        }
        "you/dotfiles" => {
            (d.pushed, d.private, d.ci) = (ago(9), true, NoChecks);
        }
        "octo-org/big-project" => {
            (d.stars, d.forks, d.pushed, d.release) = (47_183, 9_442, ago(0), release("v2.105.0", 6));
            d.branch = "trunk".into();
            d.prs = vec![
                pr(repo, 9847, "Search issues by more than one author", "shion", Passed, false),
                pr(repo, 10253, "Send the head repository id when a PR is made", "jacob-k", NoChecks, true),
                pr(repo, 10273, "Process --jq before --template", "heaths", Failed, false),
                pr(repo, 10275, "Template functions in jq filters", "heaths", Passed, true),
                pr(repo, 10423, "New command: env list", "iamazeem", Pending, false),
                pr(repo, 10730, "Custom SSH and SCP commands from the environment", "cmbrose", Passed, false),
            ];
            d.issues = vec![
                issue(repo, 13502, "Login fails behind a proxy that rewrites headers", 0, &["bug", "auth"]),
                issue(repo, 13498, "Ask before a download overwrites a file", 1, &["enhancement"]),
                issue(repo, 13471, "Docs: the install page misses Fedora 43", 2, &["docs"]),
                issue(repo, 13455, "Tab completion is slow in large repos", 4, &[]),
                issue(repo, 13440, "Colours unreadable on a light terminal", 5, &["accessibility"]),
            ];
            d.issue_total = 918;
        }
        "gitlab.com/acme/widgets" => {
            (d.stars, d.forks, d.pushed, d.release, d.pr) = (1_204, 96, ago(1), release("v17.3.0", 20), "MR !");
            d.prs = vec![pr(repo, 2210, "Dark mode for the settings page", "lin", Passed, false)];
            if let Some(p) = d.prs.first_mut() {
                p.url = Some(format!("{}/-/merge_requests/2210", page(repo)));
            }
        }
        "codeberg.org/acme/atlas" => {
            (d.stars, d.forks, d.pushed, d.release) = (76, 5, ago(15), release("v0.4.0", 44));
        }
        _ => return Err(ForgeError::NotFound("the demo only knows its own made-up repos".into())),
    }
    d.issue_total = d.issue_total.max(d.issues.len() as u64);
    d.pr_total = d.pr_total.max(d.prs.len() as u64);
    Ok(d)
}

pub fn check(repo: &str) -> Card {
    Card::from_result(repo, fetched(repo))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::report::{Light, age_at};

    #[test]
    fn ago_gives_iso_times_the_cards_can_read() {
        let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs() as i64;
        assert_eq!(age_at(&ago(0), now), "today");
        assert_eq!(age_at(&ago(12), now), "12d ago");
        assert_eq!(age_at(&ago(400), now), "400d ago");
    }

    #[test]
    fn the_demo_shows_every_colour_and_touches_no_file() {
        enable();
        let tabs = crate::config::parse(&lines());
        assert_eq!(tabs.iter().map(|t| t.repos.len()).collect::<Vec<_>>(), [4, 3]);
        let light = |r: &str| check(r).light();
        assert_eq!(light("you/rocket-launcher"), Light::Green);
        assert_eq!(light("you/tiny-parser"), Light::Orange);
        assert_eq!(light("you/weather-cli"), Light::Red);
        assert_eq!(light("someone/typed-in"), Light::Red);
        assert!(check("you/dotfiles").ok().is_some_and(|d| d.private));
    }
}
