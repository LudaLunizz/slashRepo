//! repo-status: where each of your repos stands, its open issues/PRs and what to do.
//!
//! Read-only towards the forges (GitHub via `gh`, GitLab and Codeberg over HTTPS; no Claude, no
//! tokens). Opens a window (GTK 4 on Linux, egui on Windows); `repo-status --text` prints the same
//! to the terminal.
// Windows: a windowed program (no console window with the Repo Status window); see src/win.rs.
#![cfg_attr(windows, windows_subsystem = "windows")]

mod config;
mod forge;
#[cfg_attr(windows, path = "gui_win.rs")]
mod gui;
mod hook;
mod install;
mod report;
#[cfg(windows)]
mod win;

use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::atomic::{AtomicUsize, Ordering};

use config::{DEFAULT_TAB, Sort, repo_name, tab_name};

/// GTK application id; also the name of the installed icon and desktop file.
pub const APP_ID: &str = "io.github.ludalunizz.SlashRepo";

const USAGE: &str = "usage:
  repo-status                         open the window
  repo-status open                    open the window in the background and return
  repo-status --text [TAB]            print the report (all tabs, or one)
  repo-status --tabs                  list the tabs and their repos
  repo-status --add OWNER/NAME [TAB]  add a repo (default tab: My repos; a new tab name makes the tab)
  repo-status --remove OWNER/NAME [TAB]   remove a repo (from every tab, or one)
  repo-status --remove-tab TAB        remove a tab and its repos from the list
  repo-status --sort TAB mine|stars|forks   order of a tab
  repo-status install [--trigger FILE]    set up /repo for the coding agents on this machine
                                      (--trigger: Linux only)
  repo-status hook                    the Claude Code prompt hook (reads the prompt on stdin)
  repo-status --help | --version";

fn usage() -> String {
    format!("{USAGE}\nThe list lives in {}; you may also edit it by hand.", config::path().display())
}

/// Your home folder: $HOME, on Windows %USERPROFILE%.
pub fn home() -> PathBuf {
    let var = if cfg!(windows) { "USERPROFILE" } else { "HOME" };
    std::env::var_os(var).filter(|v| !v.is_empty()).map_or_else(|| PathBuf::from("/"), PathBuf::from)
}

/// A command that never flashes a console window on Windows (the window runs `gh` without one).
pub fn command(program: impl AsRef<std::ffi::OsStr>) -> std::process::Command {
    #[allow(unused_mut)]
    let mut c = std::process::Command::new(program);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        c.creation_flags(windows_sys::Win32::System::Threading::CREATE_NO_WINDOW);
    }
    c
}

/// Write via a temp file + rename, so a crash never leaves half a file. `mode` is the Unix file
/// mode (Windows has none).
pub fn write_atomic(path: &Path, data: &[u8], mode: u32) -> std::io::Result<()> {
    use std::io::Write;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(format!(".tmp{}", std::process::id()));
    let tmp = path.with_file_name(name);
    let _ = std::fs::remove_file(&tmp);
    let written = (|| {
        let mut opts = std::fs::OpenOptions::new();
        opts.write(true).create_new(true);
        #[cfg(unix)]
        std::os::unix::fs::OpenOptionsExt::mode(&mut opts, mode);
        #[cfg(not(unix))]
        let _ = mode;
        let mut f = opts.open(&tmp)?;
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
    #[cfg(windows)]
    if !args.is_empty() {
        win::attach_parent_console();
    }
    match args.as_slice() {
        [] => gui::run(),
        ["open"] => match std::env::current_exe() {
            Ok(me) if hook::open(&me) => {
                println!("Repo Status window opened.");
                ExitCode::SUCCESS
            }
            _ => fail("could not start the Repo Status window"),
        },
        ["--help" | "-h"] => {
            println!("{}", usage());
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
                .unwrap_or_else(|| install::bin_path(&home()));
            hook::run(&window);
            ExitCode::SUCCESS // a hook must never stand in the way of the prompt
        }
        ["install"] => install::run(None).map_or_else(|e| fail(&e), |()| ExitCode::SUCCESS),
        ["install", "--trigger", f] => {
            install::run(Some(Path::new(f))).map_or_else(|e| fail(&e), |()| ExitCode::SUCCESS)
        }
        ["--text", rest @ ..] if rest.len() <= 1 => match config::load_tabs() {
            Ok(tabs) => {
                print!("{}", report::text_mode(&tabs, rest.first().copied()));
                ExitCode::SUCCESS
            }
            Err(e) => fail(&e),
        },
        ["--tabs"] => {
            let tabs = match config::load_tabs() {
                Ok(tabs) => tabs,
                Err(e) => return fail(&e),
            };
            for tab in tabs {
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
            _ => fail(&usage()),
        },
        _ => fail(&usage()),
    }
}
