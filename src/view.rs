//! What the window shows, without any toolkit: both windows (GTK 4 in src/gui.rs, egui in
//! src/gui_win.rs) draw from this, so the wording, colours, order and button states are written
//! once and tested here without a display.
//!
//! Layers: forge (fetch) → report (cards, to-dos) → view (this file) → gui / gui_win (draw only).

use std::collections::{HashMap, HashSet};

use crate::config::{self, Sort, Tab};
use crate::report::{self, Card, CheckState, Fetched, Light};

pub const SORT_LABELS: [&str; 3] = ["My order", "Most stars", "Most forks"];
pub const ADD_HINT: &str = "Add a repo: owner/name, or a GitHub, GitLab or Codeberg link";
pub const BAD_REPO: &str =
    "Type owner/name (GitHub), gitlab.com/group/project, codeberg.org/owner/repo, or paste a link";
pub const NO_REPOS: &str = "No repos in this tab yet. Add one above.";
pub const NO_TABS: &str = "No tabs yet: make one with “+ New tab”.";
pub const CHECKING: &str = "checking…";
pub const NOTHING_TO_DO: &str = "Nothing to do";
pub const DEMO_NOTE: &str = "Demo: made-up repos. Nothing is read from a forge, your list is not touched.";
/// PRs or issues shown on a card before the rest folds away.
pub const FOLD: usize = 3;

pub type Rgb = (u8, u8, u8);

/// The one palette: a calm dark of its own, the same on every desktop theme and on Windows.
/// Text colours keep at least 4.5:1 against CARD (tested below).
pub mod palette {
    use super::Rgb;
    /// the window
    pub const BG: Rgb = (0x15, 0x18, 0x1c);
    /// a card
    pub const CARD: Rgb = (0x1d, 0x21, 0x26);
    /// buttons, the open tab, a row under the pointer
    pub const RAISED: Rgb = (0x27, 0x2c, 0x33);
    /// text fields
    pub const FIELD: Rgb = (0x10, 0x12, 0x15);
    pub const BORDER: Rgb = (0x33, 0x39, 0x41);
    /// the main text: off-white, not the brightest thing on the screen
    pub const TEXT: Rgb = (0xd4, 0xd9, 0xdf);
    /// labels and details
    pub const MUTED: Rgb = (0x97, 0xa1, 0xac);
}

/// Link and private-border blue.
pub const LINK: Rgb = (0x53, 0x9b, 0xf5);

pub fn rgb(l: Light) -> Rgb {
    match l {
        Light::Red => (0xec, 0x60, 0x58),
        Light::Orange => (0xd2, 0x99, 0x22),
        Light::Green => (0x57, 0xab, 0x5a),
        Light::Grey => (0x76, 0x83, 0x90),
        Light::Blue => LINK,
    }
}

/// `over` at `alpha` (0-1) on top of `under`: tinted backgrounds and softened borders.
pub fn mix(over: Rgb, under: Rgb, alpha: f32) -> Rgb {
    let ch = |o: u8, u: u8| (f32::from(o) * alpha + f32::from(u) * (1.0 - alpha)).round() as u8;
    (ch(over.0, under.0), ch(over.1, under.1), ch(over.2, under.2))
}

/// A card's border: the visibility colour, softened so the status dot stays the loudest mark.
pub fn border(visibility: Option<Light>) -> Rgb {
    visibility.map_or(palette::BORDER, |l| mix(rgb(l), palette::CARD, 0.55))
}

/// GTK css and markup colours (the egui window takes the numbers)
#[cfg_attr(windows, allow(dead_code))]
pub fn hex((r, g, b): Rgb) -> String {
    format!("#{r:02x}{g:02x}{b:02x}")
}

/// Developer aid for the README pictures: SLASHREPO_SNAPSHOT_SIZE=940x1010 opens the window at
/// that size (only read together with SLASHREPO_SNAPSHOT).
pub fn snapshot_size() -> Option<(i32, i32)> {
    std::env::var_os("SLASHREPO_SNAPSHOT")?;
    let size = std::env::var("SLASHREPO_SNAPSHOT_SIZE").ok()?;
    let (w, h) = size.split_once('x')?;
    Some((w.parse().ok().filter(|w| (300..=4000).contains(w))?, h.parse().ok().filter(|h| (200..=4000).contains(h))?))
}

/// "Remove the tab?" question and its detail line.
pub fn remove_tab_question(name: &str, repos: usize) -> (String, String) {
    (
        format!("Remove the tab “{name}”?"),
        format!("Its {} leave{} the list. Nothing changes on the forge.", count(repos, "repo"), s_if(repos == 1)),
    )
}

pub fn edit_failed(e: &dyn std::fmt::Display) -> String {
    format!("Could not change {}: {e}", config::path().display())
}

fn s_if(yes: bool) -> &'static str {
    if yes { "s" } else { "" }
}

/// "1 repo", "3 repos"
fn count(n: usize, word: &str) -> String {
    format!("{n} {word}{}", s_if(n != 1))
}

/// The button under a folded list.
pub fn fold_text(hidden: usize, open: bool) -> String {
    if open { "Show fewer".into() } else { format!("Show {hidden} more") }
}

/// "owner/" and "name" (or "host/group/" and "project"): the name is the part to read first.
pub fn split_name(repo: &str) -> (&str, &str) {
    repo.rfind('/').map_or(("", repo), |i| repo.split_at(i + 1))
}

/// The word for a check state on a card.
pub fn ci_word(ci: &CheckState) -> &str {
    match ci {
        CheckState::Passed => "passing",
        CheckState::Failed => "failing",
        CheckState::Errored => "could not run",
        CheckState::Pending => "running",
        CheckState::NoChecks => "none",
        CheckState::Unknown(w) => w,
    }
}

pub fn ci_tone(ci: &CheckState) -> Light {
    match ci {
        CheckState::Passed => Light::Green,
        CheckState::Failed | CheckState::Errored => Light::Red,
        CheckState::Pending | CheckState::Unknown(_) => Light::Orange,
        CheckState::NoChecks => Light::Grey,
    }
}

/// One labelled fact on a card: "release" "v1.4.2 · 3d ago".
#[derive(Debug, PartialEq, Eq)]
pub struct Fact {
    pub label: String,
    pub value: String,
    /// Grey: plain text; a colour: the value is drawn in it, with a dot
    pub tone: Light,
}

fn fact(label: &str, value: impl Into<String>) -> Fact {
    Fact { label: label.into(), value: value.into(), tone: Light::Grey }
}

/// One PR or issue.
#[derive(Debug, PartialEq, Eq)]
pub struct Row {
    /// "PR #17", "MR !4", "#14"
    pub id: String,
    pub title: String,
    /// quiet details at the end: "checks passing · by a-friend", "6d ago"
    pub meta: String,
    pub labels: Vec<String>,
    /// a PR's checks; None for an issue (no dot)
    pub tone: Option<Light>,
    /// only https pages (see report::openable); a row without one is plain text
    pub url: Option<String>,
}

/// The PRs or the issues of a card.
#[derive(Debug, PartialEq, Eq)]
pub struct Group {
    /// "Pull requests · 2"
    pub title: String,
    pub rows: Vec<Row>,
    /// what the forge has beyond the rows: "915 more on the forge"
    pub more: Option<String>,
}

#[derive(Debug, PartialEq, Eq)]
pub struct CardView {
    pub repo: String,
    pub dot: Light,
    /// the repo's page, for its name
    pub url: String,
    /// still being checked: no facts yet
    pub checking: bool,
    /// the check failed: why
    pub error: Option<String>,
    pub facts: Vec<Fact>,
    pub groups: Vec<Group>,
    /// what to do next, most urgent first; empty while checking and when `clear`
    pub todo: Vec<String>,
    /// nothing asks for you
    pub clear: bool,
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

fn facts(d: &Fetched) -> Vec<Fact> {
    let release = match &d.release {
        Some(r) => format!("{} · {}{}", r.tag, report::age(&r.when), if r.pre { " · pre-release" } else { "" }),
        None => "none yet".into(),
    };
    let mut out = vec![
        fact("release", release),
        Fact { label: format!("CI on {}", d.branch), value: ci_word(&d.ci).into(), tone: ci_tone(&d.ci) },
        fact("last push", report::age(&d.pushed)),
        fact("★", d.stars.to_string()),
        fact("forks", d.forks.to_string()),
    ];
    if d.alerts > 0 {
        out.push(Fact { label: "security alerts".into(), value: d.alerts.to_string(), tone: Light::Red });
    }
    out
}

fn groups(d: &Fetched) -> Vec<Group> {
    let link = |u: &Option<String>| u.clone().filter(|u| report::openable(u));
    let mut out = Vec::new();
    if !d.prs.is_empty() {
        let rows = d.prs.iter().map(|p| {
            let checks = match &p.ci {
                CheckState::NoChecks => "no checks".to_string(),
                ci => format!("checks {}", ci_word(ci)),
            };
            Row {
                id: format!("{}{}", d.pr, p.n),
                title: p.title.clone(),
                meta: format!("{}{checks} · by {}", if p.draft { "draft · " } else { "" }, p.by),
                labels: vec![],
                tone: Some(ci_tone(&p.ci)),
                url: link(&p.url),
            }
        });
        let name = if d.pr.starts_with("MR") { "Merge requests" } else { "Pull requests" };
        out.push(Group { title: format!("{name} · {}", d.prs.len()), rows: rows.collect(), more: None });
    }
    if !d.issues.is_empty() {
        let rows = d.issues.iter().map(|i| Row {
            id: format!("#{}", i.n),
            title: i.title.clone(),
            meta: report::age(&i.created),
            labels: i.labels.clone(),
            tone: None,
            url: link(&i.url),
        });
        let beyond = d.issue_total.saturating_sub(d.issues.len() as u64);
        out.push(Group {
            title: format!("Issues · {}", d.issue_total.max(d.issues.len() as u64)),
            rows: rows.collect(),
            more: (beyond > 0).then(|| format!("{beyond} more on the forge")),
        });
    }
    out
}

fn card_view(repo: &str, c: Option<&Card>, mine: bool, pos: usize, count: usize) -> CardView {
    let url = c.and_then(Card::ok).map(|d| d.url.clone()).filter(|u| !u.is_empty());
    let mut v = CardView {
        repo: repo.into(),
        dot: c.map_or(Light::Grey, Card::light),
        url: url.unwrap_or_else(|| report::web_url(repo)),
        checking: c.is_none(),
        error: None,
        facts: vec![],
        groups: vec![],
        todo: vec![],
        clear: false,
        visibility: report::visibility(c),
        can_up: mine && pos > 0,
        can_down: mine && pos + 1 < count,
    };
    let Some(c) = c else { return v };
    match &c.result {
        Err(e) => v.error = Some(format!("Could not check: {e}")),
        Ok(d) => (v.facts, v.groups) = (facts(d), groups(d)),
    }
    v.clear = c.clear();
    if !v.clear {
        v.todo = c.todo.clone();
    }
    v
}

/// The count pills at the top of the window, over every repo of every tab (each once).
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Summary {
    pub red: usize,
    pub orange: usize,
    pub green: usize,
    pub checking: usize,
}

impl Summary {
    /// The pills to draw, worst first; a count of 0 gets none.
    pub fn pills(&self) -> Vec<(String, Light)> {
        [
            (self.red, format!("{} need{} a fix", self.red, s_if(self.red == 1)), Light::Red),
            (self.orange, format!("{} to do", self.orange), Light::Orange),
            (self.green, format!("{} clear", self.green), Light::Green),
            (self.checking, format!("{} checking…", self.checking), Light::Grey),
        ]
        .into_iter()
        .filter(|(n, ..)| *n > 0)
        .map(|(_, text, light)| (text, light))
        .collect()
    }
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

    pub fn summary(&self, tabs: &[Tab]) -> Summary {
        let mut s = Summary::default();
        for r in config::all_repos(tabs) {
            match self.cards.get(&r).map(Card::light) {
                Some(Light::Red) => s.red += 1,
                Some(Light::Orange) => s.orange += 1,
                Some(Light::Green) => s.green += 1,
                _ => s.checking += 1,
            }
        }
        s
    }

    /// The line beside the pills: time of the last check, counts and totals over all tabs.
    pub fn status_line(&self, tabs: &[Tab]) -> String {
        let all = config::all_repos(tabs);
        format!(
            "{}{} in {} · all tabs: {}",
            if self.stamp.is_empty() { String::new() } else { format!("checked {} · ", self.stamp) },
            count(all.len(), "repo"),
            count(tabs.len(), "tab"),
            report::totals_line(&all, &self.cards)
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::forge::ForgeError;
    use crate::report::{Issue, Pr};

    fn tab(name: &str, sort: Sort, repos: &[&str]) -> Tab {
        Tab { name: name.into(), sort, repos: repos.iter().map(|r| r.to_string()).collect() }
    }

    fn issue(n: &str, url: &str, labels: &[&str]) -> Issue {
        Issue {
            n: n.into(),
            title: format!("issue {n}"),
            url: Some(url.into()),
            created: String::new(),
            labels: labels.iter().map(|l| l.to_string()).collect(),
        }
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
            issues: vec![issue("1", "https://h/1", &["bug"]), issue("2", "file:///etc/passwd", &[])],
            issue_total: 3,
            alerts: 0,
            private: true,
        }
    }

    fn done(cards: Vec<(&str, Fetched)>, asked: &[&str]) -> Done {
        let found = cards.into_iter().map(|(r, d)| (r.to_string(), Card::from_result(r, Ok(d)))).collect();
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
            "2 repos in 2 tabs · all tabs: ★ 0 stars · 0 forks in 0 repos (2 not counted: checking or failed)"
        );
        assert_eq!(ch.summary(&tabs).pills(), [("2 checking…".to_string(), Light::Grey)]);
        // o/b's check crashed: it still gets a card
        ch.apply(done(vec![("o/a", fetched(4))], &["o/a", "o/b"]), "09:30".into());
        assert!(!ch.checking(&tabs));
        assert_eq!(
            ch.status_line(&tabs),
            "checked 09:30 · 2 repos in 2 tabs · all tabs: ★ 4 stars · 0 forks in 1 repo (1 not counted: checking or failed)"
        );
        assert_eq!(
            ch.summary(&tabs).pills(),
            [("1 needs a fix".to_string(), Light::Red), ("1 to do".to_string(), Light::Orange)]
        );
        let v = ch.tab(&tabs[0]);
        assert_eq!(v.cards[1].error.as_deref(), Some("Could not check: internal error while reading the answer"));
        assert_eq!(v.cards[1].todo, ["press Refresh to try again"]);
        assert_eq!(v.dot, Light::Red);
        ch.clear();
        assert_eq!(ch.start(&tabs), ["o/a", "o/b"], "Refresh checks everything again");
    }

    #[test]
    fn card_facts_rows_and_buttons() {
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
        let said: Vec<String> = a.facts.iter().map(|f| format!("{} {}", f.label, f.value)).collect();
        assert_eq!(said, ["release none yet", "CI on main passing", "last push ?", "★ 1", "forks 0"]);
        assert_eq!(a.facts[1].tone, Light::Green);
        let [issues] = a.groups.as_slice() else { panic!("one group: {:?}", a.groups) };
        assert_eq!((issues.title.as_str(), issues.more.as_deref()), ("Issues · 3", Some("1 more on the forge")));
        let first = &issues.rows[0];
        assert_eq!((first.id.as_str(), first.meta.as_str(), first.tone), ("#1", "?", None));
        assert_eq!(
            (first.url.as_deref(), first.labels.as_slice()),
            (Some("https://h/1"), ["bug".to_string()].as_slice())
        );
        // a non-https page is shown as plain text, never as a link
        assert_eq!(issues.rows[1].url, None);
        assert_eq!((a.clear, a.todo.as_slice()), (false, ["3 open issue(s) to triage".to_string()].as_slice()));
        // still checking: grey, a guessed page, nothing else
        let b = &v.cards[1];
        assert_eq!((b.dot, b.checking, b.url.as_str()), (Light::Grey, true, "https://github.com/o/b"));
        assert!(b.facts.is_empty() && b.todo.is_empty() && !b.clear);
        assert_eq!(v.dot, Light::Orange);
    }

    #[test]
    fn pull_request_rows_say_their_checks_and_a_clear_card_has_no_to_do() {
        let mut d = fetched(0);
        (d.issues, d.issue_total, d.pr) = (vec![], 0, "MR !");
        d.prs = vec![
            Pr { n: "7".into(), title: "t".into(), url: None, draft: true, by: "ann".into(), ci: CheckState::NoChecks },
            Pr { n: "8".into(), title: "u".into(), url: None, draft: true, by: "bo".into(), ci: CheckState::Failed },
        ];
        let v = card_view("gitlab.com/g/p", Some(&Card::from_result("gitlab.com/g/p", Ok(d))), true, 0, 1);
        let [mrs] = v.groups.as_slice() else { panic!("one group: {:?}", v.groups) };
        assert_eq!(mrs.title, "Merge requests · 2");
        assert_eq!((mrs.rows[0].id.as_str(), mrs.rows[0].meta.as_str()), ("MR !7", "draft · no checks · by ann"));
        assert_eq!((mrs.rows[0].tone, mrs.rows[1].tone), (Some(Light::Grey), Some(Light::Red)));
        assert_eq!(mrs.rows[1].meta, "draft · checks failing · by bo");
        // drafts ask for nothing: the card is clear and draws "Nothing to do" itself
        assert!(v.clear && v.todo.is_empty());
        assert_eq!(v.dot, Light::Green);
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
    fn error_card_and_small_words() {
        let tabs = [tab("A", Sort::Mine, &["gitlab.com/g/p"])];
        let mut ch = Checks::default();
        ch.start(&tabs);
        let card = Card::from_result("gitlab.com/g/p", Err(ForgeError::Auth("HTTP 401".into())));
        ch.apply(
            Done { asked: vec!["gitlab.com/g/p".into()], found: [("gitlab.com/g/p".into(), card)].into() },
            String::new(),
        );
        let c = &ch.tab(&tabs[0]).cards[0];
        assert_eq!(c.error.as_deref(), Some("Could not check: access refused: HTTP 401"));
        assert_eq!((c.dot, c.visibility, c.url.as_str()), (Light::Red, None, "https://gitlab.com/g/p"));
        assert!(!c.clear && c.todo.len() == 1);
        assert_eq!(remove_tab_question("X", 2).0, "Remove the tab “X”?");
        assert_eq!(remove_tab_question("X", 1).1, "Its 1 repo leaves the list. Nothing changes on the forge.");
        assert_eq!(split_name("gitlab.com/g/p"), ("gitlab.com/g/", "p"));
        assert_eq!(split_name("solo"), ("", "solo"));
        assert_eq!((fold_text(4, false), fold_text(4, true)), ("Show 4 more".into(), "Show fewer".into()));
    }

    /// WCAG contrast of a text colour on a background.
    fn contrast(a: Rgb, b: Rgb) -> f32 {
        let lum = |(r, g, b): Rgb| {
            let ch = |c: u8| {
                let c = f32::from(c) / 255.0;
                if c <= 0.03928 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
            };
            0.2126 * ch(r) + 0.7152 * ch(g) + 0.0722 * ch(b)
        };
        let (hi, lo) = (lum(a).max(lum(b)), lum(a).min(lum(b)));
        (hi + 0.05) / (lo + 0.05)
    }

    #[test]
    fn the_palette_is_readable_and_not_glaring() {
        use palette::{BG, CARD, MUTED, RAISED, TEXT};
        assert_eq!(hex(rgb(Light::Red)), "#ec6058");
        assert_eq!(hex(LINK), hex(rgb(Light::Blue)));
        let status = [rgb(Light::Red), rgb(Light::Orange), rgb(Light::Green)];
        // text: 4.5:1 where it is read (window and cards); on a hovered row the status words
        // may drop to 4:1, plain text never below 4.5
        for (bg, least) in [(BG, 4.5), (CARD, 4.5), (RAISED, 4.0)] {
            for text in [TEXT, MUTED, LINK].into_iter().chain(status) {
                let need = if status.contains(&text) { least } else { 4.5 };
                assert!(contrast(text, bg) >= need, "{} on {}: {:.2}", hex(text), hex(bg), contrast(text, bg));
            }
        }
        // no pure white text: the main text stays under 14:1 on a card
        assert!(contrast(TEXT, CARD) < 14.0);
        assert_eq!(border(None), palette::BORDER);
        assert_eq!(mix((255, 255, 255), (0, 0, 0), 0.5), (128, 128, 128));
    }
}
