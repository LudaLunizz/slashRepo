//! `repo-status uninstall`: undo `install`. Takes out the hook entry in Claude Code's settings.json
//! (backup first, everything else in the file stays), the command and skill files that are ours, the
//! desktop or Start Menu entry and icon, the systemd units of `--trigger` and the binary. It keeps
//! your repos list, the trigger file and any file that is not ours. Safe to run again.

use std::path::Path;

use crate::agents::{AGENTS, our_hooks};
use crate::home;
use crate::install::{bin_path, desktop_files, home_var, is_ours, start_menu_link, units_dir};
use crate::settings::{self, Settings};

/// What a step did (None: nothing there to do), or why it failed.
type Step = Result<Option<String>, String>;

fn hook_step(bin: &Path, h: &Path) -> Step {
    let mut s = Settings::read(h)?;
    if settings::remove_hook(&mut s.value, &our_hooks(h, bin)) == 0 {
        return Ok(None);
    }
    let backup = s.save()?.map_or_else(String::new, |b| format!(" (backup: {})", b.display()));
    Ok(Some(format!("hook removed from {}{backup}", s.path().display())))
}

/// Remove a file if it is ours; a file of the user's own stays.
fn file_step(path: &Path) -> Step {
    match std::fs::read_to_string(path) {
        Ok(t) if is_ours(&t) => {
            std::fs::remove_file(path).map_err(|e| format!("{}: {e}", path.display()))?;
            Ok(Some(format!("removed {}", path.display())))
        }
        Ok(_) => Ok(Some(format!("kept {} (not ours)", path.display()))),
        Err(e) if e.kind() == std::io::ErrorKind::InvalidData => {
            Ok(Some(format!("kept {} (not ours)", path.display())))
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(format!("{}: {e}", path.display())),
    }
}

/// Remove a file we made (named after the app); a missing one is fine.
fn remove(path: &Path) -> Result<bool, String> {
    match std::fs::remove_file(path) {
        Ok(()) => Ok(true),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(format!("{}: {e}", path.display())),
    }
}

fn desktop_step(h: &Path) -> Step {
    let files = if cfg!(windows) { vec![start_menu_link(h)] } else { desktop_files(h).to_vec() };
    let mut gone = Vec::new();
    for f in &files {
        if remove(f)? {
            gone.push(f.display().to_string());
        }
    }
    Ok((!gone.is_empty()).then(|| format!("removed {}", gone.join(", "))))
}

/// The systemd units of `--trigger` (Linux). The trigger file itself is left: it may be yours.
fn trigger_step(h: &Path) -> Step {
    let dir = units_dir(h);
    let units = ["slashrepo-trigger.path", "slashrepo-trigger.service"].map(|u| dir.join(u));
    if !units.iter().any(|u| u.exists()) {
        return Ok(None);
    }
    let systemctl =
        |args: &[&str]| std::process::Command::new("systemctl").args(args).status().is_ok_and(|s| s.success());
    let stopped = systemctl(&["--user", "disable", "--now", "slashrepo-trigger.path"]);
    for u in &units {
        remove(u)?;
    }
    let reloaded = systemctl(&["--user", "daemon-reload"]);
    let mut said = format!("removed the units in {}", dir.display());
    if !(stopped && reloaded) {
        said += " (systemctl did not confirm: run `systemctl --user disable --now slashrepo-trigger.path` yourself)";
    }
    Ok(Some(said))
}

/// The binary, last. Windows cannot delete a running exe but can rename it: then it goes aside.
fn binary_step(bin: &Path) -> Step {
    if cfg!(windows) {
        let _ = std::fs::remove_file(bin.with_extension("exe.old"));
    }
    match std::fs::remove_file(bin) {
        Ok(()) => Ok(Some(format!("removed {}", bin.display()))),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(_) if cfg!(windows) => {
            let old = bin.with_extension("exe.old");
            std::fs::rename(bin, &old).map_err(|e| format!("{}: {e}", bin.display()))?;
            Ok(Some(format!("moved {} to {} (it is running: delete that file later)", bin.display(), old.display())))
        }
        Err(e) => Err(format!("{}: {e}", bin.display())),
    }
}

pub fn run() -> Result<(), String> {
    if std::env::var_os(home_var()).is_none_or(|v| v.is_empty()) {
        return Err(format!("{} is not set: there is no home folder to look in", home_var()));
    }
    let h = home();
    let bin = bin_path(&h);
    let mut steps: Vec<(String, Step)> = vec![("Claude Code hook".into(), hook_step(&bin, &h))];
    for a in &AGENTS {
        steps.push((a.name.into(), file_step(&(a.file)(&h))));
        if let Some(old) = a.old_file {
            steps.push((format!("{} (old folder)", a.name), file_step(&old(&h))));
        }
    }
    steps.push((if cfg!(windows) { "Start Menu" } else { "app menu" }.into(), desktop_step(&h)));
    if !cfg!(windows) {
        steps.push(("trigger".into(), trigger_step(&h)));
    }
    steps.push(("program".into(), binary_step(&bin)));

    let (mut failed, mut did) = (0, 0);
    for (name, step) in &steps {
        match step {
            Ok(Some(said)) => {
                did += 1;
                println!("{name}: {said}");
            }
            Ok(None) => {}
            Err(e) => {
                failed += 1;
                println!("{name}: FAILED: {e}");
            }
        }
    }
    if did + failed == 0 {
        println!("nothing to remove: slashRepo is not installed here");
    }
    println!(
        "kept: your repos list {} and any settings backup (settings.json.bak-slashrepo)",
        crate::config::path().display()
    );
    if failed > 0 {
        return Err(format!("{failed} of {} steps failed (the others are done)", steps.len()));
    }
    Ok(())
}
