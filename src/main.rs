//! repo-status: where each of your repos stands, its open issues/PRs and what to do.
//!
//! Read-only towards the forges (GitHub via `gh`, GitLab and Codeberg over HTTPS; no Claude, no
//! tokens). Opens a GTK4 window; `repo-status --text` prints the same to the terminal.

mod config;
mod forge;
mod gui;
mod hook;
mod install;
mod report;

use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::atomic::{AtomicUsize, Ordering};

use config::{DEFAULT_TAB, Sort, repo_name, tab_name};

const USAGE: &str = "usage:
  repo-status                         open the window
  repo-status --text [TAB]            print the report (all tabs, or one)
  repo-status --tabs                  list the tabs and their repos
  repo-status --add OWNER/NAME [TAB]  add a repo (default tab: My repos; a new tab name makes the tab)
  repo-status --remove OWNER/NAME [TAB]   remove a repo (from every tab, or one)
  repo-status --remove-tab TAB        remove a tab and its repos from the list
  repo-status --sort TAB mine|stars|forks   order of a tab
  repo-status install [--trigger FILE]    set up /repo for the coding agents on this machine
  repo-status hook                    the Claude Code prompt hook (reads the prompt on stdin)
  repo-status --help | --version
The list lives in ~/.config/slashrepo/repos; you may also edit it by hand.";

pub fn home() -> PathBuf {
    std::env::var_os("HOME").map_or_else(|| PathBuf::from("/"), PathBuf::from)
}

/// Write via a temp file + rename, so a crash never leaves half a file.
pub fn write_atomic(path: &Path, data: &[u8], mode: u32) -> std::io::Result<()> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(format!(".tmp{}", std::process::id()));
    let tmp = path.with_file_name(name);
    let _ = std::fs::remove_file(&tmp);
    let written = (|| {
        let mut f = std::fs::OpenOptions::new().write(true).create_new(true).mode(mode).open(&tmp)?;
        f.write_all(data)?;
        f.sync_all()?;
        std::fs::rename(&tmp, path)
    })();
    if written.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    written
}

/// f over items on at most `threads` threads; results in item order.
pub fn par_map<T: Sync, R: Send>(items: &[T], threads: usize, f: impl Fn(&T) -> R + Sync) -> Vec<R> {
    let next = AtomicUsize::new(0);
    let mut out: Vec<(usize, R)> = std::thread::scope(|s| {
        let workers: Vec<_> = (0..threads.min(items.len()))
            .map(|_| {
                s.spawn(|| {
                    let mut done = Vec::new();
                    loop {
                        let i = next.fetch_add(1, Ordering::Relaxed);
                        let Some(item) = items.get(i) else { break };
                        done.push((i, f(item)));
                    }
                    done
                })
            })
            .collect();
        workers.into_iter().flat_map(|w| w.join().expect("worker panicked")).collect()
    });
    out.sort_by_key(|(i, _)| *i);
    out.into_iter().map(|(_, r)| r).collect()
}

fn fail(msg: &str) -> ExitCode {
    eprintln!("{msg}");
    ExitCode::FAILURE
}

fn edited(r: std::io::Result<()>) -> ExitCode {
    match r {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => fail(&format!("{}: {e}", config::path().display())),
    }
}

fn main() -> ExitCode {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    let args: Vec<&str> = argv.iter().map(String::as_str).collect();
    match args.as_slice() {
        [] => gui::run(),
        ["--help" | "-h"] => {
            println!("{USAGE}");
            ExitCode::SUCCESS
        }
        ["--version"] => {
            println!("repo-status {}", env!("CARGO_PKG_VERSION"));
            ExitCode::SUCCESS
        }
        ["hook"] => {
            let window = std::env::var_os("REPO_STATUS")
                .map(PathBuf::from)
                .or_else(|| std::env::current_exe().ok())
                .unwrap_or_else(|| home().join(".local/bin/repo-status"));
            hook::run(&window);
            ExitCode::SUCCESS // a hook must never stand in the way of the prompt
        }
        ["install"] => install::run(None).map_or_else(|e| fail(&e), |()| ExitCode::SUCCESS),
        ["install", "--trigger", f] => {
            install::run(Some(Path::new(f))).map_or_else(|e| fail(&e), |()| ExitCode::SUCCESS)
        }
        ["--text", rest @ ..] if rest.len() <= 1 => {
            print!("{}", report::text_mode(&config::load_tabs(), rest.first().copied()));
            ExitCode::SUCCESS
        }
        ["--tabs"] => {
            for tab in config::load_tabs() {
                let sort =
                    if tab.sort == Sort::Mine { String::new() } else { format!("  sort = {}", tab.sort.as_str()) };
                println!("[{}]{sort}", tab.name);
                for r in &tab.repos {
                    println!("  {r}");
                }
            }
            ExitCode::SUCCESS
        }
        [cmd @ ("--add" | "--remove"), repo, rest @ ..] if rest.len() <= 1 => {
            let tab = match rest.first() {
                Some(t) => tab_name(t),
                None => (*cmd == "--add").then(|| DEFAULT_TAB.to_string()),
            };
            let (Some(repo), true) = (repo_name(repo), rest.is_empty() || tab.is_some()) else {
                return fail("not a repo (owner/name, a GitHub/GitLab/Codeberg link) or not a tab name (no [ ] #)");
            };
            if *cmd == "--add" {
                let tab = tab.unwrap_or_else(|| DEFAULT_TAB.into());
                edited(config::edit(|l| config::add_repo(l, &repo, &tab)))
            } else {
                edited(config::edit(|l| config::remove_repo(l, &repo, tab.as_deref())))
            }
        }
        ["--remove-tab", tab] => edited(config::edit(|l| config::remove_tab(l, tab.trim()))),
        ["--sort", tab, sort] => match (tab_name(tab), Sort::parse(sort)) {
            (Some(tab), Some(sort)) => edited(config::edit(|l| config::set_sort(l, &tab, sort))),
            _ => fail(USAGE),
        },
        _ => fail(USAGE),
    }
}
