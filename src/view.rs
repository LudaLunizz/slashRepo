//! What the window shows, without any toolkit: both windows (GTK 4 in src/gui.rs, egui in
//! src/gui_win.rs) draw from this, so the wording, colours, order and button states are written
//! once and tested here without a display.
//!
//! Layers: forge (fetch) → report (cards, to-dos) → view (this file) → gui / gui_win (draw only).

use std::collections::{HashMap, HashSet};

use crate::config::{self, Sort, Tab};
use crate::report::{self, Card, Light};

pub const SORT_LABELS: [&str; 3] = ["My order", "Stars", "Forks"];
pub const ADD_HINT: &str = "owner/name, or a GitHub, GitLab or Codeberg link";
pub const BAD_REPO: &str =
    "Type owner/name (GitHub), gitlab.com/group/project, codeberg.org/owner/repo, or paste a link";
pub const NO_REPOS: &str = "No repos in this tab yet. Add one above.";
/// (Windows: the GTK notebook simply shows no page)
#[cfg_attr(not(windows), allow(dead_code))]
pub const NO_TABS: &str = "No tabs yet: make one with “New tab”.";
pub const CHECKING: &str = "checking…";
/// Link text colour (the same blue as a private repo's border).
pub const LINK: (u8, u8, u8) = (0x53, 0x9b, 0xf5);

/// The one palette.
pub fn rgb(l: Light) -> (u8, u8, u8) {
    match l {
        Light::Red => (0xe5, 0x53, 0x4b),
        Light::Orange => (0xd2, 0x99, 0x22),
        Light::Green => (0x57, 0xab, 0x5a),
        Light::Grey => (0x76, 0x83, 0x90),
        Light::Blue => LINK,
    }
}

/// GTK markup colours (the egui window takes the numbers)
#[cfg_attr(windows, allow(dead_code))]
pub fn hex((r, g, b): (u8, u8, u8)) -> String {
    format!("#{r:02x}{g:02x}{b:02x}")
}

/// "Remove the tab?" question and its detail line.
pub fn remove_tab_question(name: &str, repos: usize) -> (String, String) {
    (format!("Remove the tab “{name}”?"), format!("Its {repos} repo(s) leave the list. Nothing changes on GitHub."))
}

pub fn edit_failed(e: &dyn std::fmt::Display) -> String {
    format!("Could not change {}: {e}", config::path().display())
}

/// One line under a card's status line: a PR or issue (with its page) or a plain note.
#[derive(Debug, PartialEq, Eq)]
pub struct Line {
    pub text: String,
    /// only https pages (see report::openable); a line without one is plain text
    pub url: Option<String>,
}

#[derive(Debug, PartialEq, Eq)]
pub struct CardView {
    pub repo: String,
    pub dot: Light,
    /// the repo's page, for its name
    pub url: String,
    /// None while the repo is being checked
    pub status: Option<String>,
    pub lines: Vec<Line>,
    /// "→ ..." lines
    pub todo: Vec<String>,
    /// ("live" | "private", border colour); None while unknown
    pub visibility: Option<(&'static str, Light)>,
    /// ▲ and ▼: own order only, under stars/forks the position comes from the numbers
    pub can_up: bool,
    pub can_down: bool,
}

#[derive(Debug, PartialEq, Eq)]
pub struct TabView {
    pub name: String,
    /// the worst card's colour
    pub dot: Light,
    pub totals: String,
    /// index into SORT_LABELS / Sort::ALL
    pub sort: usize,
    pub cards: Vec<CardView>,
}

fn card_view(repo: &str, c: Option<&Card>, mine: bool, pos: usize, count: usize) -> CardView {
    let url = c.and_then(Card::ok).map(|d| d.url.clone()).filter(|u| !u.is_empty());
    let mut v = CardView {
        repo: repo.into(),
        dot: c.map_or(Light::Grey, Card::light),
        url: url.unwrap_or_else(|| report::web_url(repo)),
        status: None,
        lines: vec![],
        todo: vec![],
        visibility: report::visibility(c),
        can_up: mine && pos > 0,
        can_down: mine && pos + 1 < count,
    };
    let Some(c) = c else { return v };
    let mut lines = c.summary_lines().into_iter();
    v.status = lines.next().map(|(t, _)| t);
    v.lines = lines
        .map(|(text, url)| match url.filter(|u| report::openable(u)) {
            Some(u) => Line { text, url: Some(u) },
            None => Line { text: format!("   {text}"), url: None },
        })
        .collect();
    v.todo = c.todo.iter().map(|t| format!("→ {t}")).collect();
    v
}

/// One finished round of checks, sent from the worker thread to the window.
pub struct Done {
    pub asked: Vec<String>,
    pub found: HashMap<String, Card>,
}

/// The worker's side: check the repos; a crash still answers (with no cards, see Checks::apply).
pub fn run_checks(asked: Vec<String>) -> Done {
    let found = std::panic::catch_unwind(|| report::check_all(&asked)).unwrap_or_default();
    Done { asked, found }
}

/// What the window knows about the checks: one per window, changed only through these methods.
#[derive(Default)]
pub struct Checks {
    cards: HashMap<String, Card>,
    /// repos being checked right now
    busy: HashSet<String>,
    /// "HH:MM" of the last finished round
    stamp: String,
}

impl Checks {
    /// Refresh: forget every card (the next start() checks them all again).
    pub fn clear(&mut self) {
        self.cards.clear();
    }

    pub fn busy(&self) -> bool {
        !self.busy.is_empty()
    }

    fn waiting(&self, tabs: &[Tab]) -> Vec<String> {
        config::all_repos(tabs).into_iter().filter(|r| !self.cards.contains_key(r) && !self.busy.contains(r)).collect()
    }

    /// A check runs, or a repo still waits for one.
    pub fn checking(&self, tabs: &[Tab]) -> bool {
        self.busy() || !self.waiting(tabs).is_empty()
    }

    /// The repos to check now (each only once); they count as busy until apply().
    pub fn start(&mut self, tabs: &[Tab]) -> Vec<String> {
        let todo = self.waiting(tabs);
        self.busy.extend(todo.iter().cloned());
        todo
    }

    /// A finished round: every asked repo gets a card, a crashed check an error card, so no
    /// card stays "checking…" for ever.
    pub fn apply(&mut self, done: Done, stamp: String) {
        let Done { asked, mut found } = done;
        for r in asked {
            self.busy.remove(&r);
            found.entry(r.clone()).or_insert_with(|| Card::crashed(&r));
        }
        self.cards.extend(found);
        self.stamp = stamp;
    }

    pub fn tab(&self, tab: &Tab) -> TabView {
        let repos = report::ordered(&tab.repos, tab.sort, &self.cards);
        let mine = tab.sort == Sort::Mine;
        let cards: Vec<CardView> =
            repos.iter().enumerate().map(|(pos, r)| card_view(r, self.cards.get(r), mine, pos, repos.len())).collect();
        TabView {
            name: tab.name.clone(),
            dot: cards.iter().map(|c| c.dot).max().unwrap_or(Light::Grey),
            totals: report::totals_line(&tab.repos, &self.cards),
            sort: Sort::ALL.iter().position(|s| *s == tab.sort).unwrap_or(0),
            cards,
        }
    }

    /// The line at the top: progress, time of the last check, counts and totals over all tabs.
    pub fn status_line(&self, tabs: &[Tab]) -> String {
        let all = config::all_repos(tabs);
        format!(
            "{}{}{} repos in {} tab(s) — all tabs: {}",
            if self.checking(tabs) { "checking… " } else { "" },
            if self.stamp.is_empty() { String::new() } else { format!("checked {} — ", self.stamp) },
            all.len(),
            tabs.len(),
            report::totals_line(&all, &self.cards)
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::forge::ForgeError;
    use crate::report::{CheckState, Fetched, Issue};

    fn tab(name: &str, sort: Sort, repos: &[&str]) -> Tab {
        Tab { name: name.into(), sort, repos: repos.iter().map(|r| r.to_string()).collect() }
    }

    fn fetched(stars: u64) -> Fetched {
        Fetched {
            url: "https://github.com/o/a".into(),
            pushed: String::new(),
            stars,
            forks: 0,
            branch: "main".into(),
            pr: "PR #",
            unreleased: None,
            ci: CheckState::Passed,
            release: None,
            prs: vec![],
            issues: vec![
                Issue {
                    n: "1".into(),
                    title: "x".into(),
                    url: Some("https://h/1".into()),
                    created: String::new(),
                    labels: vec![],
                },
                Issue {
                    n: "2".into(),
                    title: "y".into(),
                    url: Some("file:///etc/passwd".into()),
                    created: String::new(),
                    labels: vec![],
                },
            ],
            issue_total: 3,
            alerts: 0,
            private: true,
        }
    }

    fn done(cards: Vec<(&str, Fetched)>, asked: &[&str]) -> Done {
        let found = cards
            .into_iter()
            .map(|(r, d)| (r.to_string(), Card { repo: r.into(), todo: vec!["do it".into()], result: Ok(d) }))
            .collect();
        Done { asked: asked.iter().map(|r| r.to_string()).collect(), found }
    }

    #[test]
    fn a_round_starts_once_and_every_asked_repo_gets_a_card() {
        let tabs = [tab("A", Sort::Mine, &["o/a", "o/b"]), tab("B", Sort::Mine, &["o/a"])];
        let mut ch = Checks::default();
        assert!(ch.checking(&tabs));
        assert_eq!(ch.start(&tabs), ["o/a", "o/b"]);
        assert!(ch.start(&tabs).is_empty(), "busy repos are not started twice");
        assert_eq!(
            ch.status_line(&tabs),
            "checking… 2 repos in 2 tab(s) — all tabs: ★ 0 stars · 0 forks in 0 repos (2 not counted: checking or failed)"
        );
        // o/b's check crashed: it still gets a card
        ch.apply(done(vec![("o/a", fetched(4))], &["o/a", "o/b"]), "09:30".into());
        assert!(!ch.checking(&tabs));
        assert_eq!(
            ch.status_line(&tabs),
            "checked 09:30 — 2 repos in 2 tab(s) — all tabs: ★ 4 stars · 0 forks in 1 repo (1 not counted: checking or failed)"
        );
        let v = ch.tab(&tabs[0]);
        assert_eq!(v.cards[1].status.as_deref(), Some("ERROR: internal error while reading the answer"));
        assert_eq!(v.cards[1].todo, ["→ press Refresh to try again"]);
        assert_eq!(v.dot, Light::Red);
        ch.clear();
        assert_eq!(ch.start(&tabs), ["o/a", "o/b"], "Refresh checks everything again");
    }

    #[test]
    fn card_lines_links_and_buttons() {
        let tabs = [tab("A", Sort::Mine, &["o/a", "o/b", "o/c"])];
        let mut ch = Checks::default();
        ch.start(&tabs);
        ch.apply(done(vec![("o/a", fetched(1))], &["o/a"]), String::new());
        let v = ch.tab(&tabs[0]);
        let a = &v.cards[0];
        assert_eq!((a.can_up, a.can_down), (false, true));
        assert_eq!((v.cards[1].can_up, v.cards[1].can_down), (true, true));
        assert_eq!((v.cards[2].can_up, v.cards[2].can_down), (true, false));
        assert_eq!(a.url, "https://github.com/o/a");
        assert_eq!(a.visibility, Some(("private", Light::Blue)));
        assert_eq!(a.lines[0], Line { text: "issue #1 x  (?)".into(), url: Some("https://h/1".into()) });
        // a non-https page is shown as plain text, never as a link
        assert_eq!(a.lines[1], Line { text: "   issue #2 y  (?)".into(), url: None });
        assert_eq!(a.lines[2], Line { text: "   ... and 1 more issues".into(), url: None });
        assert_eq!(a.todo, ["→ do it"]);
        // still checking: grey, a guessed page, no lines
        let b = &v.cards[1];
        assert_eq!((b.dot, b.status.as_deref(), b.url.as_str()), (Light::Grey, None, "https://github.com/o/b"));
        assert_eq!(v.dot, Light::Orange);
    }

    #[test]
    fn sorted_tabs_have_no_move_buttons() {
        let tabs = [tab("A", Sort::Stars, &["o/a", "o/b"])];
        let mut ch = Checks::default();
        ch.start(&tabs);
        ch.apply(done(vec![("o/a", fetched(1)), ("o/b", fetched(9))], &["o/a", "o/b"]), String::new());
        let v = ch.tab(&tabs[0]);
        assert_eq!(v.sort, 1);
        assert_eq!(v.cards.iter().map(|c| c.repo.as_str()).collect::<Vec<_>>(), ["o/b", "o/a"]);
        assert!(v.cards.iter().all(|c| !c.can_up && !c.can_down));
    }

    #[test]
    fn error_card_and_palette() {
        let tabs = [tab("A", Sort::Mine, &["gitlab.com/g/p"])];
        let mut ch = Checks::default();
        ch.start(&tabs);
        let err = ForgeError::Auth("HTTP 401".into());
        let card = Card { repo: "gitlab.com/g/p".into(), todo: vec!["set it".into()], result: Err(err) };
        ch.apply(
            Done { asked: vec!["gitlab.com/g/p".into()], found: [("gitlab.com/g/p".into(), card)].into() },
            String::new(),
        );
        let c = &ch.tab(&tabs[0]).cards[0];
        assert_eq!(c.status.as_deref(), Some("ERROR: access refused: HTTP 401"));
        assert_eq!((c.dot, c.visibility, c.url.as_str()), (Light::Red, None, "https://gitlab.com/g/p"));
        assert_eq!(hex(rgb(Light::Red)), "#e5534b");
        assert_eq!(hex(LINK), hex(rgb(Light::Blue)));
        assert_eq!(remove_tab_question("X", 2).0, "Remove the tab “X”?");
    }
}
