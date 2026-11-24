//! `repo-status uninstall`: undo `install`. Takes out the hook entry in Claude Code's settings.json
//! (backup first, everything else in the file stays), the command and skill files that are ours, the
//! desktop or Start Menu entry and icon, the systemd units of `--trigger` and the binary. It keeps
//! your repos list, the trigger file and any file that is not ours. Safe to run again.

use std::path::Path;

use serde_json::Value;

use crate::install::{
    agent_files, bin_path, claude_settings, desktop_files, home_var, hook_command, is_ours, old_hook, settings_backup,
    start_menu_link, units_dir,
};
use crate::{home, mode_of, write_atomic, write_atomic_keep};

/// What a step did (None: nothing there to do), or why it failed.
type Step = Result<Option<String>, String>;

/// Remove every UserPromptSubmit hook whose command is in `commands`. A group, the list or the
/// `hooks` object that this empties goes too; nothing else is touched. Returns how many were removed.
pub fn remove_hook(settings: &mut Value, commands: &[String]) -> usize {
    let mut removed = 0;
    let mut hooks_empty = false;
    if let Some(hooks) = settings.get_mut("hooks").and_then(Value::as_object_mut) {
        let mut list_empty = false;
        if let Some(list) = hooks.get_mut("UserPromptSubmit").and_then(Value::as_array_mut) {
            let mut emptied = Vec::new();
            for (i, group) in list.iter_mut().enumerate() {
                let Some(inner) = group.get_mut("hooks").and_then(Value::as_array_mut) else { continue };
                let before = inner.len();
                inner.retain(|h| {
                    !h.get("command").and_then(Value::as_str).is_some_and(|c| commands.iter().any(|x| x == c))
                });
                removed += before - inner.len();
                if before > 0 && inner.is_empty() {
                    emptied.push(i);
                }
            }
            for i in emptied.into_iter().rev() {
                list.remove(i);
            }
            list_empty = removed > 0 && list.is_empty();
        }
        if list_empty {
            hooks.remove("UserPromptSubmit");
        }
        hooks_empty = removed > 0 && hooks.is_empty();
    }
    if hooks_empty && let Some(root) = settings.as_object_mut() {
        root.remove("hooks");
    }
    removed
}

fn hook_step(bin: &Path, h: &Path) -> Step {
    let settings = claude_settings(h);
    let text = match std::fs::read_to_string(&settings) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(format!("{}: {e}", settings.display())),
    };
    let mut v: Value = serde_json::from_str(&text).map_err(|e| format!("{}: {e}", settings.display()))?;
    let ours = [hook_command(bin), old_hook(h).to_string_lossy().into_owned()];
    if remove_hook(&mut v, &ours) == 0 {
        return Ok(None);
    }
    let out = serde_json::to_string_pretty(&v).map_err(|e| e.to_string())? + "\n";
    let backup = settings_backup(h);
    write_atomic(&backup, text.as_bytes(), mode_of(&settings).unwrap_or(0o644))
        .map_err(|e| format!("{}: {e}", backup.display()))?;
    write_atomic_keep(&settings, out.as_bytes(), 0o644).map_err(|e| format!("{}: {e}", settings.display()))?;
    Ok(Some(format!("hook removed from {} (backup: {})", settings.display(), backup.display())))
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
    for (name, file) in agent_files(&h) {
        steps.push((name.into(), file_step(&file)));
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn only_our_hook_goes_and_empty_shells_with_it() {
        let ours = vec!["/b/repo-status hook".to_string()];
        // ours next to another hook in the same group: the other stays
        let mut v = json!({"theme": "x", "hooks": {
            "UserPromptSubmit": [{"hooks": [{"type": "command", "command": "/b/repo-status hook"},
                                            {"type": "command", "command": "echo hi"}]}],
            "Stop": [{"hooks": [{"type": "command", "command": "/b/repo-status hook"}]}]}});
        assert_eq!(remove_hook(&mut v, &ours), 1);
        assert_eq!(v["hooks"]["UserPromptSubmit"][0]["hooks"], json!([{"type": "command", "command": "echo hi"}]));
        assert!(v["hooks"]["Stop"][0]["hooks"][0]["command"].is_string(), "other events are not ours to touch");
        assert_eq!(v["theme"], "x");
        // ours alone: group, list and the hooks object go, the rest of the file stays
        let mut v = json!({"theme": "x", "hooks": {"UserPromptSubmit": [
            {"hooks": [{"type": "command", "command": "/b/repo-status hook"}]}]}});
        assert_eq!(remove_hook(&mut v, &ours), 1);
        assert_eq!(v, json!({"theme": "x"}));
        // nothing of ours: nothing changes, not even an empty list that was already there
        let mut v = json!({"hooks": {"UserPromptSubmit": []}});
        assert_eq!(remove_hook(&mut v, &ours), 0);
        assert_eq!(v, json!({"hooks": {"UserPromptSubmit": []}}));
    }
}
