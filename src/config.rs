//! The repos file (`~/.config/slashrepo/repos`, on Windows `%APPDATA%\slashRepo\repos`): one repo per line, `#` starts a comment.
//! A `[Tab name]` line starts a tab; lines before the first one go in "My repos".
//! `sort = stars` (or forks) inside a tab sorts it. Every edit works on the file's lines,
//! so comments and hand-made order survive.

use std::io::{Error, ErrorKind};
use std::path::{Path, PathBuf};
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

/// The tabs in file order, duplicate repos (case-insensitive) dropped. A repo written with other
/// capitals in a later tab takes the first spelling, so it is checked and counted once.
pub fn parse(lines: &[String]) -> Vec<Tab> {
    let mut tabs: Vec<Tab> = Vec::new();
    let mut spelling: std::collections::HashMap<String, String> = std::collections::HashMap::new();
    for mut e in entries(lines) {
        if e.kind == Kind::Repo {
            e.value = spelling.entry(e.value.to_ascii_lowercase()).or_insert_with(|| e.value.clone()).clone();
        }
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

static SCHEME: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)^(https?|ssh|git)://").unwrap());
/// a clone address in scp form: git@github.com:owner/name.git
static SCP: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^[A-Za-z0-9._-]+@([A-Za-z0-9.-]+\.[A-Za-z]+):([^/].*)$").unwrap());
/// user@ and :port in front of the path (ssh://git@host:2222/...)
static USER_PORT: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^([^/@]*@)?([^/:]+)(:[0-9]+)?").unwrap());
static GITHUB_HOST: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^(www\.)?github\.com/").unwrap());
static GITHUB: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^[A-Za-z0-9-]+/[A-Za-z0-9_.-]+$").unwrap());
static CODEBERG_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^codeberg\.org/[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+$").unwrap());
static GITLAB: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^[a-z0-9-]+(\.[a-z0-9-]+)+(/[A-Za-z0-9_.-]+){2,}$").unwrap());
static TAB_NAME: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^[^\[\]#\n]+$").unwrap());

/// The pages of a Codeberg repo (codeberg.org/owner/repo/PAGE/...): a link to one is the repo.
const CODEBERG_PAGES: [&str; 16] = [
    "src",
    "issues",
    "pulls",
    "releases",
    "tags",
    "branches",
    "commits",
    "commit",
    "compare",
    "wiki",
    "actions",
    "activity",
    "milestones",
    "labels",
    "projects",
    "settings",
];

/// owner/name for GitHub (name, link or clone address); codeberg.org/owner/repo;
/// host/group/project for GitLab (gitlab.com/... or a link). A link to a page of the repo (an
/// issue, a branch) is the repo; hosts are written in lower case. None if it is none of these.
pub fn repo_name(text: &str) -> Option<String> {
    let t = text.trim();
    let t = match SCP.captures(t) {
        Some(c) if !t.contains("://") => format!("{}/{}", &c[1], &c[2]),
        _ => SCHEME.replace(t, "").into_owned(),
    };
    // a page's ?query or #anchor is not part of the repo
    let t = t.split(['?', '#']).next().unwrap_or("");
    let t = USER_PORT.replace(t, "$2").into_owned();
    // the host (the first part, when it has a dot) in lower case: GitHub.com is github.com
    let t = match t.split_once('/') {
        Some((first, rest)) if first.contains('.') => format!("{}/{rest}", first.to_ascii_lowercase()),
        _ => t,
    };
    let t = t.trim_end_matches('/');
    let t = t.split("/-/").next().unwrap_or(t); // a GitLab page: gitlab.com/g/p/-/issues/3
    let t = t.strip_suffix(".git").unwrap_or(t);
    if let Some(m) = GITHUB_HOST.find(t) {
        // GitHub has no subgroups: whatever follows owner/name is a page of the repo
        let rest: Vec<&str> = t[m.end()..].splitn(3, '/').take(2).collect();
        let t = rest.join("/");
        let t = t.strip_suffix(".git").unwrap_or(&t);
        return GITHUB.is_match(t).then(|| t.to_string());
    }
    if GITHUB.is_match(t) {
        return Some(t.to_string());
    }
    if let Some(path) = t.strip_prefix(&format!("{CODEBERG}/")) {
        let parts: Vec<&str> = path.split('/').collect();
        let t = match parts.get(2) {
            Some(page) if CODEBERG_PAGES.contains(page) => format!("{CODEBERG}/{}/{}", parts[0], parts[1]),
            _ => t.to_string(),
        };
        return CODEBERG_RE.is_match(&t).then_some(t);
    }
    GITLAB.is_match(t).then(|| t.to_string())
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

/// $XDG_CONFIG_HOME/slashrepo/repos (default ~/.config/slashrepo/repos); on Windows %APPDATA%\slashRepo\repos.
pub fn path() -> PathBuf {
    if cfg!(windows) {
        let appdata = std::env::var_os("APPDATA").filter(|v| !v.is_empty());
        return appdata.map_or_else(|| crate::home().join("AppData/Roaming"), PathBuf::from).join("slashRepo/repos");
    }
    config_home(&crate::home()).join("slashrepo/repos")
}

/// The file's lines; `Ok(None)` only when there is no file. Any other failure (no permission,
/// not UTF-8, a directory) is an error, so an edit never mistakes a file it cannot read for a
/// missing one and writes over it.
fn read_lines_at(p: &Path) -> std::io::Result<Option<Vec<String>>> {
    match std::fs::read_to_string(p) {
        // a byte order mark (Windows Notepad) is not part of the first line
        Ok(text) => Ok(Some(text.trim_start_matches('\u{feff}').lines().map(|l| format!("{l}\n")).collect())),
        Err(e) if e.kind() == ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e),
    }
}

pub fn read_lines() -> std::io::Result<Option<Vec<String>>> {
    if crate::demo::on() {
        return Ok(Some(crate::demo::lines()));
    }
    read_lines_at(&path())
}

/// The answer of `gh repo list`, asked once per run: the window reads the list on every redraw
/// (on its main thread), so without this each tab switch would wait for `gh` again.
static OWN_REPOS: std::sync::Mutex<Option<Vec<String>>> = std::sync::Mutex::new(None);

/// Your own non-fork, non-archived repos (newest 20): the list when there is no config file.
/// Asked once and kept (see [`forget_own_repos`]).
pub fn own_repos() -> Vec<String> {
    kept_or(&OWN_REPOS, ask_own_repos)
}

fn kept_or(kept: &std::sync::Mutex<Option<Vec<String>>>, ask: impl FnOnce() -> Vec<String>) -> Vec<String> {
    kept.lock().unwrap_or_else(std::sync::PoisonError::into_inner).get_or_insert_with(ask).clone()
}

/// Refresh: the next own_repos() asks `gh` again (you may have logged in meanwhile).
pub fn forget_own_repos() {
    *OWN_REPOS.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = None;
}

/// A hung `gh` gets the same 60 seconds as the checks (an edit holds the lock meanwhile), and a
/// failed one (not logged in) gives no list rather than whatever it printed.
fn ask_own_repos() -> Vec<String> {
    let mut cmd = crate::command("gh");
    cmd.args(["repo", "list", "--source", "--no-archived", "--limit", "20"]).args([
        "--json",
        "nameWithOwner",
        "--jq",
        ".[].nameWithOwner",
    ]);
    match crate::output_within(cmd, std::time::Duration::from_secs(60)) {
        Ok(o) if o.status.success() => own_repo_lines(&String::from_utf8_lossy(&o.stdout)),
        _ => Vec::new(),
    }
}

/// gh's lines, each one only if it is a repo name.
fn own_repo_lines(out: &str) -> Vec<String> {
    out.split_whitespace().filter_map(repo_name).collect()
}

/// $XDG_CONFIG_HOME when it is an absolute path (the spec says to ignore a relative one), else
/// ~/.config. Unix only; the repos file and the systemd units both live under it.
pub fn config_home(home: &Path) -> PathBuf {
    config_home_from(std::env::var_os("XDG_CONFIG_HOME"), home)
}

fn config_home_from(xdg: Option<std::ffi::OsString>, home: &Path) -> PathBuf {
    xdg.map(PathBuf::from).filter(|p| p.is_absolute()).unwrap_or_else(|| home.join(".config"))
}

pub fn load_tabs() -> Result<Vec<Tab>, String> {
    match read_lines() {
        Ok(Some(lines)) => Ok(parse(&lines)),
        Ok(None) => Ok(vec![Tab { name: DEFAULT_TAB.into(), repos: own_repos(), sort: Sort::Mine }]),
        Err(e) => Err(format!("Could not read {}: {e}", path().display())),
    }
}

/// The lock file next to the repos file: held for a whole edit, so two edits at once (two
/// `--add` runs, a window button and a CLI call) take turns instead of one losing its change.
/// The repos file itself is replaced on every write, so it cannot carry the lock.
pub fn lock_path() -> PathBuf {
    let p = path();
    p.with_file_name(".repos.lock")
}

/// Apply change to the config file; a missing file starts from own_repos(). A file that exists
/// but cannot be read stops the edit and is left as it is.
pub fn edit(change: impl FnOnce(&[String]) -> Vec<String>) -> std::io::Result<()> {
    if crate::demo::on() {
        // the demo's list lives in memory: your repos file is never touched
        crate::demo::set_lines(change(&crate::demo::lines()));
        return Ok(());
    }
    let lock = lock_path();
    if let Some(dir) = lock.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let lock = std::fs::OpenOptions::new().create(true).truncate(false).write(true).open(&lock)?;
    lock.lock()?; // released when `lock` is dropped (also if change() panics)
    let lines = match read_lines() {
        Ok(Some(lines)) => lines,
        Ok(None) => own_repos().into_iter().map(|r| format!("{r}\n")).collect(),
        Err(e) => return Err(Error::new(e.kind(), format!("could not read it, left untouched ({e})"))),
    };
    let new = change(&lines);
    if new == lines && path().exists() {
        return Ok(()); // nothing changed: the file (and its time stamp) stays as it is
    }
    crate::write_atomic_keep(&path(), new.concat().as_bytes(), 0o644)
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
    fn only_a_missing_file_counts_as_missing() {
        let dir = std::env::temp_dir().join(format!("slashrepo-config-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        // no file: the one case that starts from the generated list
        assert_eq!(read_lines_at(&dir.join("none")).unwrap(), None);
        // a readable file, comments and all
        let good = dir.join("good");
        std::fs::write(&good, "me/a # x\n\n").unwrap();
        assert_eq!(read_lines_at(&good).unwrap(), Some(v(&["me/a # x\n", "\n"])));
        // a file that is there but not UTF-8 (a Latin-1 comment) is an error, not "missing"
        let latin = dir.join("latin");
        std::fs::write(&latin, b"me/a # caf\xe9\n").unwrap();
        assert!(read_lines_at(&latin).is_err());
        // so is anything else that cannot be read as a file
        assert!(read_lines_at(&dir).is_err());
        // a BOM in front of the first line is not part of it
        let bom = dir.join("bom");
        std::fs::write(&bom, "\u{feff}[Work]\r\nme/a\r\n").unwrap();
        let lines = read_lines_at(&bom).unwrap().unwrap();
        assert_eq!(lines, v(&["[Work]\n", "me/a\n"]));
        assert_eq!(parse(&lines)[0].name, "Work");
        let _ = std::fs::remove_dir_all(&dir);
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
        // one repo, two spellings in two tabs: one name, so one check
        let t = parse(&v(&["Me/App\n", "[B]\n", "me/app\n", "me/other\n"]));
        assert_eq!(tab(&t, "B").repos, v(&["Me/App", "me/other"]));
        assert_eq!(all_repos(&t), v(&["Me/App", "me/other"]));
    }

    #[test]
    fn gh_is_asked_once_until_forgotten() {
        let kept = std::sync::Mutex::new(None);
        let asked = std::cell::Cell::new(0);
        let ask = || {
            asked.set(asked.get() + 1);
            v(&["me/a"])
        };
        assert_eq!(kept_or(&kept, ask), v(&["me/a"]));
        assert_eq!(kept_or(&kept, ask), v(&["me/a"]));
        assert_eq!(asked.get(), 1, "a redraw must not run gh again");
        *kept.lock().unwrap() = None; // what forget_own_repos() does to the real one
        kept_or(&kept, ask);
        assert_eq!(asked.get(), 2);
    }

    #[test]
    fn own_repo_lines_keep_only_repo_names() {
        assert_eq!(own_repo_lines("me/a\nme/b\n"), v(&["me/a", "me/b"]));
        assert_eq!(own_repo_lines("To get started run: gh auth login"), Vec::<String>::new());
    }

    #[cfg(unix)]
    #[test]
    fn config_home_ignores_a_relative_xdg_config_home() {
        let h = Path::new("/h");
        assert_eq!(config_home_from(Some("rel/cfg".into()), h), Path::new("/h/.config"));
        assert_eq!(config_home_from(Some("/x/cfg".into()), h), Path::new("/x/cfg"));
        assert_eq!(config_home_from(None, h), Path::new("/h/.config"));
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
        for bad in ["gitlab.com/onlygroup", "https://gitlab.com/-/issues", "gitlab.com/a b/c", "codeberg.org/a"] {
            assert_eq!(repo_name(bad), None, "{bad}");
        }
        // capitals in the scheme or host: still the same repo, host in lower case
        for (text, want) in [
            ("HTTPS://github.com/o/r", "o/r"),
            ("https://GitHub.com/Me/App", "Me/App"),
            ("Codeberg.org/o/r", "codeberg.org/o/r"),
            ("https://GitLab.com/Group/Project", "gitlab.com/Group/Project"),
        ] {
            assert_eq!(repo_name(text).as_deref(), Some(want), "{text}");
        }
        // a link to a page of the repo, and clone addresses
        for (text, want) in [
            ("https://github.com/o/r/issues/3", "o/r"),
            ("https://github.com/o/r/tree/main/src", "o/r"),
            ("https://github.com/o/r?tab=readme-ov-file#install", "o/r"),
            ("https://gitlab.com/g/sub/p/-/merge_requests/1", "gitlab.com/g/sub/p"),
            ("https://codeberg.org/o/r/src/branch/main", "codeberg.org/o/r"),
            ("https://codeberg.org/o/r/issues/12", "codeberg.org/o/r"),
            ("git@github.com:o/r.git", "o/r"),
            ("ssh://git@github.com/o/r.git", "o/r"),
            ("git@gitlab.com:g/sub/p.git", "gitlab.com/g/sub/p"),
            ("ssh://git@codeberg.org:2222/o/r.git", "codeberg.org/o/r"),
        ] {
            assert_eq!(repo_name(text).as_deref(), Some(want), "{text}");
        }
        // a GitHub link with only an owner is no repo
        assert_eq!(repo_name("https://github.com/o"), None);
        assert_eq!(tab_name("  ArtCraft ").unwrap(), "ArtCraft");
        assert_eq!(tab_name("a]b"), None);
        assert_eq!(tab_name(""), None);
    }
}
