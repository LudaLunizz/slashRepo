//! The repos file (`~/.config/slashrepo/repos`): one repo per line, `#` starts a comment.
//! A `[Tab name]` line starts a tab; lines before the first one go in "My repos".
//! `sort = stars` (or forks) inside a tab sorts it. Every edit works on the file's lines,
//! so comments and hand-made order survive.

use std::path::PathBuf;
use std::sync::LazyLock;

use regex::Regex;

pub const DEFAULT_TAB: &str = "My repos";
pub const CODEBERG: &str = "codeberg.org";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sort {
    /// The order of the lines in the file.
    Mine,
    Stars,
    Forks,
}

impl Sort {
    pub const ALL: [Sort; 3] = [Sort::Mine, Sort::Stars, Sort::Forks];

    pub fn as_str(self) -> &'static str {
        match self {
            Sort::Mine => "mine",
            Sort::Stars => "stars",
            Sort::Forks => "forks",
        }
    }

    pub fn parse(s: &str) -> Option<Sort> {
        Sort::ALL.into_iter().find(|x| x.as_str() == s)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Tab {
    pub name: String,
    pub repos: Vec<String>,
    pub sort: Sort,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Blank,
    Tab,
    Repo,
    Sort,
}

struct Entry {
    line: usize,
    tab: String,
    kind: Kind,
    value: String,
}

fn content(raw: &str) -> &str {
    raw.split('#').next().unwrap_or("").trim()
}

fn entries(lines: &[String]) -> Vec<Entry> {
    let mut tab = DEFAULT_TAB.to_string();
    let mut out = Vec::with_capacity(lines.len());
    for (i, raw) in lines.iter().enumerate() {
        let line = content(raw);
        let (kind, value) = if line.starts_with('[') && line.ends_with(']') && line.len() >= 2 {
            let name = line[1..line.len() - 1].trim();
            tab = if name.is_empty() { DEFAULT_TAB } else { name }.to_string();
            (Kind::Tab, tab.clone())
        } else if line.replace(' ', "").starts_with("sort=") {
            (Kind::Sort, line.split_once('=').map_or("", |(_, v)| v).trim().to_string())
        } else if line.is_empty() {
            (Kind::Blank, String::new())
        } else {
            (Kind::Repo, line.to_string())
        };
        out.push(Entry { line: i, tab: tab.clone(), kind, value });
    }
    out
}

/// The tabs in file order, duplicates (case-insensitive) dropped.
pub fn parse(lines: &[String]) -> Vec<Tab> {
    let mut tabs: Vec<Tab> = Vec::new();
    for e in entries(lines) {
        if e.kind == Kind::Blank {
            continue;
        }
        let k = match tabs.iter().position(|t| t.name == e.tab) {
            Some(k) => k,
            None => {
                tabs.push(Tab { name: e.tab.clone(), repos: Vec::new(), sort: Sort::Mine });
                tabs.len() - 1
            }
        };
        let t = &mut tabs[k];
        match e.kind {
            Kind::Repo if !contains(&t.repos, &e.value) => t.repos.push(e.value),
            Kind::Sort => {
                if let Some(s) = Sort::parse(&e.value) {
                    t.sort = s;
                }
            }
            _ => {}
        }
    }
    tabs
}

fn contains(repos: &[String], repo: &str) -> bool {
    repos.iter().any(|r| r.eq_ignore_ascii_case(repo))
}

static SCHEME: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^https?://").unwrap());
static GITHUB_HOST: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)^(www\.)?github\.com/").unwrap());
static GITHUB: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^[A-Za-z0-9-]+/[A-Za-z0-9_.-]+$").unwrap());
static CODEBERG_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^codeberg\.org/[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+$").unwrap());
static GITLAB: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^[a-z0-9-]+(\.[a-z0-9-]+)+(/[A-Za-z0-9_.-]+){2,}$").unwrap());
static TAB_NAME: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^[^\[\]#\n]+$").unwrap());

/// owner/name for GitHub (name or link); codeberg.org/owner/repo; host/group/project for GitLab
/// (gitlab.com/... or a link). None if it is none of these.
pub fn repo_name(text: &str) -> Option<String> {
    let t = SCHEME.replace(text.trim(), "");
    let t = t.trim_end_matches('/');
    let t = t.strip_suffix(".git").unwrap_or(t);
    let t = GITHUB_HOST.replace(t, "").into_owned();
    if GITHUB.is_match(&t) {
        return Some(t);
    }
    if t.starts_with(&format!("{CODEBERG}/")) {
        return CODEBERG_RE.is_match(&t).then_some(t);
    }
    if GITLAB.is_match(&t) && !format!("{t}/").contains("/-/") {
        return Some(t);
    }
    None
}

pub fn tab_name(text: &str) -> Option<String> {
    let t = text.trim();
    TAB_NAME.is_match(t).then(|| t.to_string())
}

pub fn add_tab(lines: &[String], tab: &str) -> Vec<String> {
    let mut out = lines.to_vec();
    if parse(lines).iter().any(|t| t.name == tab) {
        return out;
    }
    if !out.is_empty() {
        out.push("\n".into());
    }
    out.push(format!("[{tab}]\n"));
    out
}

pub fn add_repo(lines: &[String], repo: &str, tab: &str) -> Vec<String> {
    if parse(lines).iter().any(|t| t.name == tab && contains(&t.repos, repo)) {
        return lines.to_vec();
    }
    let mut out = add_tab(lines, tab);
    let end = entries(&out).iter().filter(|e| e.tab == tab && e.kind != Kind::Blank).map(|e| e.line).next_back();
    let at = end.map_or(out.len(), |e| e + 1);
    out.insert(at, format!("{repo}\n"));
    out
}

/// Drop repo from tab (None = from every tab).
pub fn remove_repo(lines: &[String], repo: &str, tab: Option<&str>) -> Vec<String> {
    let drop: Vec<usize> = entries(lines)
        .into_iter()
        .filter(|e| e.kind == Kind::Repo && e.value.eq_ignore_ascii_case(repo) && tab.is_none_or(|t| t == e.tab))
        .map(|e| e.line)
        .collect();
    keep(lines, &drop)
}

pub fn remove_tab(lines: &[String], tab: &str) -> Vec<String> {
    let drop: Vec<usize> = entries(lines).into_iter().filter(|e| e.tab == tab).map(|e| e.line).collect();
    keep(lines, &drop)
}

fn keep(lines: &[String], drop: &[usize]) -> Vec<String> {
    lines.iter().enumerate().filter(|(i, _)| !drop.contains(i)).map(|(_, l)| l.clone()).collect()
}

/// Swap repo's line with the repo above (step -1) or below (+1) it in the same tab.
pub fn move_repo(lines: &[String], repo: &str, tab: &str, step: isize) -> Vec<String> {
    let rows: Vec<usize> =
        entries(lines).into_iter().filter(|e| e.tab == tab && e.kind == Kind::Repo).map(|e| e.line).collect();
    let mut out = lines.to_vec();
    let Some(k) = rows.iter().position(|&i| content(&lines[i]).eq_ignore_ascii_case(repo)) else {
        return out;
    };
    let Some(j) = k.checked_add_signed(step).filter(|&j| j < rows.len()) else {
        return out;
    };
    out.swap(rows[k], rows[j]);
    out
}

pub fn set_sort(lines: &[String], tab: &str, sort: Sort) -> Vec<String> {
    let drop: Vec<usize> =
        entries(lines).into_iter().filter(|e| e.tab == tab && e.kind == Kind::Sort).map(|e| e.line).collect();
    let mut out = add_tab(&keep(lines, &drop), tab);
    if sort == Sort::Mine {
        return out;
    }
    let first = entries(&out).into_iter().find(|e| e.tab == tab && e.kind != Kind::Blank);
    let at = match first {
        Some(e) if e.kind == Kind::Tab => e.line + 1,
        Some(e) => e.line,
        None => out.len(),
    };
    out.insert(at, format!("sort = {}\n", sort.as_str()));
    out
}

pub fn path() -> PathBuf {
    crate::home().join(".config/slashrepo/repos")
}

pub fn read_lines() -> Option<Vec<String>> {
    let text = std::fs::read_to_string(path()).ok()?;
    Some(text.lines().map(|l| format!("{l}\n")).collect())
}

/// Your own non-fork, non-archived repos (newest 20): the list when there is no config file.
pub fn own_repos() -> Vec<String> {
    let out = std::process::Command::new("gh")
        .args(["repo", "list", "--source", "--no-archived", "--limit", "20"])
        .args(["--json", "nameWithOwner", "--jq", ".[].nameWithOwner"])
        .output();
    match out {
        Ok(o) => String::from_utf8_lossy(&o.stdout).split_whitespace().map(String::from).collect(),
        Err(_) => Vec::new(),
    }
}

pub fn load_tabs() -> Vec<Tab> {
    match read_lines() {
        Some(lines) => parse(&lines),
        None => vec![Tab { name: DEFAULT_TAB.into(), repos: own_repos(), sort: Sort::Mine }],
    }
}

/// Apply change to the config file; a missing file starts from own_repos().
pub fn edit(change: impl FnOnce(&[String]) -> Vec<String>) -> std::io::Result<()> {
    let lines = read_lines().unwrap_or_else(|| own_repos().into_iter().map(|r| format!("{r}\n")).collect());
    let lines = change(&lines);
    crate::write_atomic(&path(), lines.concat().as_bytes(), 0o644)
}

/// Every repo of every tab once, in file order.
pub fn all_repos(tabs: &[Tab]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for r in tabs.iter().flat_map(|t| &t.repos) {
        if !out.contains(r) {
            out.push(r.clone());
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(lines: &[&str]) -> Vec<String> {
        lines.iter().map(|s| s.to_string()).collect()
    }

    fn tab<'a>(tabs: &'a [Tab], name: &str) -> &'a Tab {
        tabs.iter().find(|t| t.name == name).unwrap()
    }

    fn names(tabs: &[Tab]) -> Vec<&str> {
        tabs.iter().map(|t| t.name.as_str()).collect()
    }

    fn old() -> Vec<String> {
        v(&["me/a  # keep this comment\n", "me/b\n", "\n", "[ArtCraft]\n", "sort = stars\n", "st/x\n", "st/y\n"])
    }

    #[test]
    fn parse_old_files_and_tabs() {
        let t = parse(&v(&["me/a\n", "# note\n", "me/b # x\n"]));
        assert_eq!(t, vec![Tab { name: DEFAULT_TAB.into(), repos: v(&["me/a", "me/b"]), sort: Sort::Mine }]);
        let t = parse(&old());
        assert_eq!(names(&t), [DEFAULT_TAB, "ArtCraft"]);
        assert_eq!(tab(&t, DEFAULT_TAB).repos, v(&["me/a", "me/b"]));
        assert_eq!(tab(&t, "ArtCraft").repos, v(&["st/x", "st/y"]));
        assert_eq!(tab(&t, "ArtCraft").sort, Sort::Stars);
    }

    #[test]
    fn add() {
        let old = old();
        let lines = add_repo(&old, "me/c", DEFAULT_TAB);
        assert_eq!(lines[2], "me/c\n");
        assert_eq!(lines[0], old[0]);
        assert_eq!(add_repo(&lines, "ME/C", DEFAULT_TAB), lines);
        let lines = add_repo(&lines, "new/z", "Friends");
        assert_eq!(tab(&parse(&lines), "Friends").repos, v(&["new/z"]));
        let t = parse(&add_repo(&[], "me/a", DEFAULT_TAB));
        assert_eq!(t, vec![Tab { name: DEFAULT_TAB.into(), repos: v(&["me/a"]), sort: Sort::Mine }]);
    }

    #[test]
    fn remove() {
        let both = add_repo(&old(), "me/a", "ArtCraft");
        let one = parse(&remove_repo(&both, "me/a", Some("ArtCraft")));
        assert!(!tab(&one, "ArtCraft").repos.contains(&"me/a".into()));
        assert!(tab(&one, DEFAULT_TAB).repos.contains(&"me/a".into()));
        assert!(parse(&remove_repo(&both, "me/a", None)).iter().all(|t| !t.repos.contains(&"me/a".into())));
        assert_eq!(names(&parse(&remove_tab(&old(), "ArtCraft"))), [DEFAULT_TAB]);
        assert_eq!(names(&parse(&remove_tab(&old(), DEFAULT_TAB))), ["ArtCraft"]);
    }

    #[test]
    fn move_keeps_comment_and_stops_at_edges() {
        let old = old();
        let moved = move_repo(&old, "me/b", DEFAULT_TAB, -1);
        assert_eq!(moved[..2], [old[1].clone(), old[0].clone()]);
        assert_eq!(move_repo(&old, "me/a", DEFAULT_TAB, -1), old);
        assert_eq!(move_repo(&old, "st/y", "ArtCraft", 1), old);
    }

    #[test]
    fn sort_lines() {
        let old = old();
        assert_eq!(tab(&parse(&set_sort(&old, "ArtCraft", Sort::Forks)), "ArtCraft").sort, Sort::Forks);
        assert_eq!(set_sort(&old, "ArtCraft", Sort::Forks).iter().filter(|l| l.contains("sort")).count(), 1);
        assert_eq!(tab(&parse(&set_sort(&old, "ArtCraft", Sort::Mine)), "ArtCraft").sort, Sort::Mine);
        let t = parse(&set_sort(&old, DEFAULT_TAB, Sort::Stars));
        assert_eq!(tab(&t, DEFAULT_TAB).repos, v(&["me/a", "me/b"]));
        assert_eq!(tab(&t, DEFAULT_TAB).sort, Sort::Stars);
    }

    #[test]
    fn input_checks() {
        assert_eq!(repo_name("https://github.com/storytold/pdfcraft.git/").unwrap(), "storytold/pdfcraft");
        assert_eq!(repo_name(" cli/cli ").unwrap(), "cli/cli");
        for bad in ["cli", "a/b/c", "a b/c", "--x", "a/b;rm"] {
            assert_eq!(repo_name(bad), None, "{bad}");
        }
        assert_eq!(repo_name("https://gitlab.com/gitlab-org/cli.git").unwrap(), "gitlab.com/gitlab-org/cli");
        assert_eq!(repo_name("gitlab.com/group/sub/project").unwrap(), "gitlab.com/group/sub/project");
        assert_eq!(repo_name("https://git.example.org/team/app/").unwrap(), "git.example.org/team/app");
        assert_eq!(repo_name("https://codeberg.org/forgejo/forgejo.git").unwrap(), "codeberg.org/forgejo/forgejo");
        assert_eq!(repo_name("codeberg.org/a/b/c"), None); // Codeberg has no subgroups
        for bad in ["gitlab.com/onlygroup", "https://gitlab.com/a/b/-/issues", "gitlab.com/a b/c"] {
            assert_eq!(repo_name(bad), None, "{bad}");
        }
        assert_eq!(tab_name("  ArtCraft ").unwrap(), "ArtCraft");
        assert_eq!(tab_name("a]b"), None);
        assert_eq!(tab_name(""), None);
    }
}
