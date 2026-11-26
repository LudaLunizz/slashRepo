//! `repo-status install [--trigger FILE]`: the binary in ~/.local/bin and a desktop entry (Windows:
//! %LOCALAPPDATA%\Programs\slashRepo and a Start Menu entry), then /repo for every coding agent
//! found on this machine (Claude Code, opencode, Codex, Antigravity, Gemini CLI). Safe to run again.

use std::path::{Path, PathBuf};

use crate::agents::{AGENTS, Step};
use crate::{home, write_atomic};

const TRIGGER_PATH: &str = include_str!("../systemd/slashrepo-trigger.path");
const TRIGGER_SERVICE: &str = include_str!("../systemd/slashrepo-trigger.service");
const ICON: &str = include_str!("../data/io.github.ludalunizz.SlashRepo.svg");

type Res = Result<(), String>;

/// Where `install` puts the binary.
pub fn bin_path(h: &Path) -> PathBuf {
    if cfg!(windows) {
        local_appdata(h).join("Programs").join("slashRepo").join("repo-status.exe")
    } else {
        h.join(".local/bin/repo-status")
    }
}

/// Where the systemd units of `--trigger` go (Linux).
pub fn units_dir(h: &Path) -> PathBuf {
    std::env::var_os("XDG_CONFIG_HOME").map_or_else(|| h.join(".config"), PathBuf::from).join("systemd/user")
}

/// The Start Menu entry (Windows).
pub fn start_menu_link(h: &Path) -> PathBuf {
    let appdata = std::env::var_os("APPDATA").filter(|v| !v.is_empty());
    appdata
        .map_or_else(|| h.join("AppData/Roaming"), PathBuf::from)
        .join("Microsoft/Windows/Start Menu/Programs/Repo Status.lnk")
}

/// The desktop file and the icon (Linux).
pub fn desktop_files(h: &Path) -> [PathBuf; 2] {
    [
        h.join(format!(".local/share/applications/{}.desktop", crate::APP_ID)),
        h.join(format!(".local/share/icons/hicolor/scalable/apps/{}.svg", crate::APP_ID)),
    ]
}

fn local_appdata(h: &Path) -> PathBuf {
    std::env::var_os("LOCALAPPDATA").filter(|v| !v.is_empty()).map_or_else(|| h.join("AppData/Local"), PathBuf::from)
}

/// cmd is on PATH (on Windows also as cmd.exe, .cmd, .bat or .ps1, the way npm installs them).
pub fn has(cmd: &str) -> bool {
    let names: Vec<String> = if cfg!(windows) {
        ["", ".exe", ".cmd", ".bat", ".ps1"].iter().map(|x| format!("{cmd}{x}")).collect()
    } else {
        vec![cmd.into()]
    };
    std::env::var_os("PATH")
        .is_some_and(|p| std::env::split_paths(&p).any(|d| names.iter().any(|n| d.join(n).is_file())))
}

fn put(path: &Path, text: &str) -> Res {
    write_atomic(path, text.as_bytes(), 0o644).map_err(|e| format!("{}: {e}", path.display()))
}

/// A command or skill file we wrote calls `repo-status`; one that does not is somebody else's.
pub fn is_ours(text: &str) -> bool {
    text.contains("repo-status")
}

/// put, but never over a file we did not write (a file that is not text is not ours either).
/// false: the user's own file is there and was left alone.
pub fn put_ours(path: &Path, text: &str) -> Result<bool, String> {
    match std::fs::read_to_string(path) {
        Ok(old) if !is_ours(&old) => return Ok(false),
        Err(e) if e.kind() == std::io::ErrorKind::InvalidData => return Ok(false),
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => return Err(format!("{}: {e}", path.display())),
        _ => {}
    }
    put(path, text).map(|()| true)
}

fn trigger(file: &Path, h: &Path) -> Step {
    if cfg!(windows) {
        return Err("--trigger sets up a systemd unit: Linux only".into());
    }
    let f = file.to_string_lossy();
    if !file.is_absolute() || f.contains('\n') {
        return Err("--trigger needs an absolute path without newlines".into());
    }
    let units = units_dir(h);
    if !file.exists() {
        put(file, "")?;
    }
    put(&units.join("slashrepo-trigger.service"), TRIGGER_SERVICE)?;
    // systemd expands % specifiers in unit files
    put(&units.join("slashrepo-trigger.path"), &TRIGGER_PATH.replace("@TRIGGER@", &f.replace('%', "%%")))?;
    for args in [&["--user", "daemon-reload"][..], &["--user", "enable", "--now", "slashrepo-trigger.path"]] {
        let ok = std::process::Command::new("systemctl").args(args).status().is_ok_and(|s| s.success());
        if !ok {
            return Err(format!("systemctl {} failed", args.join(" ")));
        }
    }
    Ok(format!("writing {f} opens the window (set SLASHREPO_TRIGGER={f} inside the sandbox)"))
}

fn desktop_exec(bin: &Path) -> String {
    let b = bin.to_string_lossy();
    if b.contains([' ', '"', '\\', '`', '$']) {
        format!("\"{}\"", b.replace('\\', "\\\\").replace('"', "\\\"").replace('`', "\\`").replace('$', "\\$"))
    } else {
        b.into()
    }
}

/// Copy this binary to `bin` (temp file + rename, so a running old one keeps working).
fn copy_self(bin: &Path) -> Res {
    let me = std::env::current_exe().map_err(|e| e.to_string())?;
    if std::fs::canonicalize(&me).ok() == std::fs::canonicalize(bin).ok() {
        return Ok(());
    }
    let data = std::fs::read(&me).map_err(|e| format!("{}: {e}", me.display()))?;
    if cfg!(windows) && bin.exists() {
        // Windows can't replace a running exe, but it can rename it: move the old one aside.
        let old = bin.with_extension("exe.old");
        let _ = std::fs::remove_file(&old); // still running from the last update: rename fails below
        std::fs::rename(bin, &old)
            .map_err(|e| format!("{}: {e} (close the Repo Status window first)", bin.display()))?;
    }
    write_atomic(bin, &data, 0o755).map_err(|e| format!("{}: {e}", bin.display()))
}

/// Linux: icon + desktop file named after the app id; GNOME matches the window to it (else a gear).
fn desktop_entry(bin: &Path, h: &Path) -> Res {
    let [desktop, icon] = desktop_files(h);
    put(&icon, ICON)?;
    put(
        &desktop,
        &format!(
            "[Desktop Entry]\nType=Application\nName=Repo Status\n\
             Comment=Where each of your repos stands: release, CI, issues, PRs, to-dos\n\
             Exec={}\nIcon={}\nStartupWMClass={}\nCategories=Development;\n",
            desktop_exec(bin),
            crate::APP_ID,
            crate::APP_ID
        ),
    )?;
    // the old name (up to 0.5.1) would show a second "Repo Status" in the app menu
    let _ = std::fs::remove_file(h.join(".local/share/applications/repo-status.desktop"));
    Ok(())
}

/// Windows: "Repo Status" in the Start Menu (a shortcut made by PowerShell; paths go in through the
/// environment, so nothing needs quoting). The icon is inside the exe.
fn start_menu_entry(bin: &Path, h: &Path) -> Res {
    let lnk = start_menu_link(h);
    let programs = lnk.parent().unwrap_or(h);
    std::fs::create_dir_all(programs).map_err(|e| format!("{}: {e}", programs.display()))?;
    let script = "$s = (New-Object -ComObject WScript.Shell).CreateShortcut($env:SLASHREPO_LNK); \
                  $s.TargetPath = $env:SLASHREPO_EXE; $s.Description = 'Where each of your repos stands'; $s.Save()";
    let ok = crate::command("powershell")
        .args(["-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-Command", script])
        .env("SLASHREPO_LNK", &lnk)
        .env("SLASHREPO_EXE", bin)
        .stdin(std::process::Stdio::null())
        .output()
        .is_ok_and(|o| o.status.success());
    if ok && lnk.exists() { Ok(()) } else { Err(format!("could not make the Start Menu entry {}", lnk.display())) }
}

/// The environment variable `home()` reads.
pub fn home_var() -> &'static str {
    if cfg!(windows) { "USERPROFILE" } else { "HOME" }
}

pub fn run(trigger_file: Option<&Path>) -> Res {
    if std::env::var_os(home_var()).is_none_or(|v| v.is_empty()) {
        return Err(format!("{} is not set: there is no home folder to install into", home_var()));
    }
    let h = home();
    let bin = bin_path(&h);
    copy_self(&bin)?;
    if cfg!(windows) {
        if let Err(e) = start_menu_entry(&bin, &h) {
            println!("note: {e}");
        }
        println!("installed: {} (window, Start Menu)", bin.display());
        if let Some(dir) = bin.parent() {
            let on_path = std::env::var_os("PATH").is_some_and(|p| std::env::split_paths(&p).any(|d| d == dir));
            if !on_path {
                println!("note: to type repo-status in a terminal, add {} to your PATH", dir.display());
            }
        }
    } else {
        desktop_entry(&bin, &h)?;
        println!("installed: {} (window, app menu)", bin.display());
    }
    if !has("gh") {
        println!("note: gh is not installed; GitHub repos need it (logged in). GitLab and Codeberg work without.");
    }
    // Every agent found is a step of its own: one that fails does not stop the others.
    let mut steps: Vec<(&str, Step)> =
        AGENTS.iter().filter(|a| a.found(&h)).map(|a| (a.name, a.install(&h, &bin))).collect();
    if let Some(f) = trigger_file {
        steps.push(("trigger", trigger(f, &h)));
    }
    let mut failed = 0;
    for (name, result) in &steps {
        match result {
            Ok(said) => println!("{name}: {said}"),
            Err(e) => {
                failed += 1;
                println!("{name}: FAILED: {e}");
            }
        }
    }
    println!("Optional: list repos in {} (see the README)", crate::config::path().display());
    if failed > 0 {
        return Err(format!(
            "{failed} of {} steps failed (the others are done; fix the cause and run install again)",
            steps.len()
        ));
    }
    Ok(())
}
