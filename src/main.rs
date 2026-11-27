//! repo-status: where each of your repos stands, its open issues/PRs and what to do.
//!
//! Read-only towards the forges (GitHub via `gh`, GitLab and Codeberg over HTTPS; no Claude, no
//! tokens). Opens a window (GTK 4 on Linux, egui on Windows); `repo-status --text` prints the same
//! to the terminal.
// Windows: a windowed program (no console window with the Repo Status window); see src/win.rs.
#![cfg_attr(windows, windows_subsystem = "windows")]

mod agents;
mod config;
mod demo;
mod forge;
#[cfg_attr(windows, path = "gui_win.rs")]
mod gui;
mod hook;
mod install;
mod report;
mod settings;
mod uninstall;
mod view;
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
  repo-status --demo                  open the window with made-up repos (reads and changes nothing)
  repo-status --text [TAB]            print the report (all tabs, or one)
  repo-status --tabs                  list the tabs and their repos
  repo-status --add OWNER/NAME [TAB]  add a repo (default tab: My repos; a new tab name makes the tab)
  repo-status --remove OWNER/NAME [TAB]   remove a repo (from every tab, or one)
  repo-status --remove-tab TAB        remove a tab and its repos from the list
  repo-status --sort TAB mine|stars|forks   order of a tab
  repo-status install [--trigger FILE]    set up /repo for the coding agents on this machine
                                      (--trigger: Linux only)
  repo-status uninstall               undo install (keeps your repo list)
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

/// `cmd.output()` with a deadline: a hung child (a `gh` waiting on a dead network) is killed and
/// reported instead of freezing the check for ever.
pub fn output_within(
    mut cmd: std::process::Command,
    limit: std::time::Duration,
) -> std::io::Result<std::process::Output> {
    use std::io::Read;
    use std::process::Stdio;
    let mut child = cmd.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn()?;
    // drain both pipes on their own threads so a chatty child cannot fill a pipe and stall
    fn drain(mut pipe: impl Read + Send + 'static) -> std::thread::JoinHandle<Vec<u8>> {
        std::thread::spawn(move || {
            let mut buf = Vec::new();
            let _ = pipe.read_to_end(&mut buf);
            buf
        })
    }
    let out = child.stdout.take().map(drain);
    let err = child.stderr.take().map(drain);
    let start = std::time::Instant::now();
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break status;
        }
        if start.elapsed() >= limit {
            let _ = child.kill();
            let _ = child.wait();
            return Err(std::io::Error::new(
                std::io::ErrorKind::TimedOut,
                format!("no answer after {}s", limit.as_secs()),
            ));
        }
        std::thread::sleep(std::time::Duration::from_millis(25));
    };
    let join = |h: Option<std::thread::JoinHandle<Vec<u8>>>| h.and_then(|h| h.join().ok()).unwrap_or_default();
    Ok(std::process::Output { status, stdout: join(out), stderr: join(err) })
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
        return written;
    }
    sync_dir(path);
    written
}

/// Sync the folder that holds `path`, so the rename itself survives a power cut (Unix; Windows
/// has no folder handle for this). Best effort: the new file is already in place.
fn sync_dir(path: &Path) {
    #[cfg(unix)]
    if let Some(dir) = path.parent() {
        let dir = if dir.as_os_str().is_empty() { Path::new(".") } else { dir };
        if let Ok(d) = std::fs::File::open(dir) {
            let _ = d.sync_all();
        }
    }
    #[cfg(not(unix))]
    let _ = path;
}

/// The Unix mode of an existing file (None on Windows or when it is not there).
pub fn mode_of(path: &Path) -> Option<u32> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::metadata(path).ok().map(|m| m.permissions().mode() & 0o777)
    }
    #[cfg(not(unix))]
    {
        let _ = path;
        None
    }
}

/// write_atomic for a file that may be there already and belongs to the user: a symlink stays a
/// symlink (the file it points to is replaced) and the file keeps its mode; `default_mode` is for a
/// new file.
pub fn write_atomic_keep(path: &Path, data: &[u8], default_mode: u32) -> std::io::Result<()> {
    let real = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    write_atomic(&real, data, mode_of(&real).unwrap_or(default_mode))
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

/// Wrong usage: exit code 2, the convention, so a script can tell it from a failed run (1).
fn misuse(msg: &str) -> ExitCode {
    eprintln!("{msg}");
    ExitCode::from(2)
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
        ["--demo"] => {
            demo::enable();
            gui::run()
        }
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
        ["uninstall"] => uninstall::run().map_or_else(|e| fail(&e), |()| ExitCode::SUCCESS),
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
                return misuse("not a repo (owner/name, a GitHub/GitLab/Codeberg link) or not a tab name (no [ ] #)");
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
            _ => misuse(&usage()),
        },
        _ => misuse(&usage()),
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::output_within;
    use std::process::Command;
    use std::time::{Duration, Instant};

    #[test]
    fn output_within_returns_what_a_quick_child_wrote() {
        let mut c = Command::new("sh");
        c.args(["-c", "echo out; echo err >&2; exit 3"]);
        let o = output_within(c, Duration::from_secs(10)).unwrap();
        assert_eq!(o.status.code(), Some(3));
        assert_eq!(o.stdout, b"out\n");
        assert_eq!(o.stderr, b"err\n");
    }

    #[test]
    fn output_within_reads_more_than_a_pipe_holds() {
        let mut c = Command::new("sh");
        c.args(["-c", "head -c 500000 /dev/zero"]);
        assert_eq!(output_within(c, Duration::from_secs(10)).unwrap().stdout.len(), 500_000);
    }

    #[test]
    fn output_within_kills_a_hung_child() {
        let mut c = Command::new("sleep");
        c.arg("30");
        let start = Instant::now();
        let e = output_within(c, Duration::from_millis(300)).unwrap_err();
        assert_eq!(e.kind(), std::io::ErrorKind::TimedOut);
        assert!(start.elapsed() < Duration::from_secs(5));
    }
}
