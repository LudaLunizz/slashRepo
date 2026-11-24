//! `repo-status install [--trigger FILE]`: the binary in ~/.local/bin, a desktop entry, then /repo
//! for every coding agent found on this machine (Claude Code, opencode, Codex, Antigravity,
//! Gemini CLI). Safe to run again.

use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use crate::{home, write_atomic};

const CLAUDE_CMD: &str = include_str!("../commands/repo.md");
const OPENCODE_CMD: &str = include_str!("../commands/opencode/repo.md");
const GEMINI_CMD: &str = include_str!("../commands/gemini/repo.toml");
const SKILL: &str = include_str!("../skills/repo/SKILL.md");
const TRIGGER_PATH: &str = include_str!("../systemd/slashrepo-trigger.path");
const TRIGGER_SERVICE: &str = include_str!("../systemd/slashrepo-trigger.service");
const ICON: &str = include_str!("../data/io.github.ludalunizz.SlashRepo.svg");

type Res = Result<(), String>;

fn has(cmd: &str) -> bool {
    std::env::var_os("PATH").is_some_and(|p| std::env::split_paths(&p).any(|d| d.join(cmd).is_file()))
}

fn put(path: &Path, text: &str) -> Res {
    write_atomic(path, text.as_bytes(), 0o644).map_err(|e| format!("{}: {e}", path.display()))
}

/// Quote for sh when needed (the hook command is run by a shell).
fn sh_quote(s: &str) -> String {
    if s.bytes().all(|b| b.is_ascii_alphanumeric() || b"/._-+".contains(&b)) {
        s.into()
    } else {
        format!("'{}'", s.replace('\'', r"'\''"))
    }
}

/// Add `command` to UserPromptSubmit once; an old `repo-status-hook` entry becomes `command`.
pub fn add_hook(settings: &mut Value, command: &str, old: &str) -> Res {
    let root = settings.as_object_mut().ok_or("settings.json is not a JSON object")?;
    let hooks = root.entry("hooks").or_insert_with(|| json!({}));
    let hooks = hooks.as_object_mut().ok_or("settings.json: hooks is not an object")?;
    let list = hooks.entry("UserPromptSubmit").or_insert_with(|| json!([]));
    let list = list.as_array_mut().ok_or("settings.json: hooks.UserPromptSubmit is not a list")?;
    let mut found = false;
    for group in list.iter_mut() {
        for h in group.get_mut("hooks").and_then(Value::as_array_mut).into_iter().flatten() {
            if h.get("command").and_then(Value::as_str) == Some(old) {
                h["command"] = json!(command);
            }
            found |= h.get("command").and_then(Value::as_str) == Some(command);
        }
    }
    if !found {
        list.push(json!({"hooks": [{"type": "command", "command": command, "timeout": 5}]}));
    }
    Ok(())
}

fn claude(bin: &Path, h: &Path) -> Res {
    let dir = h.join(".claude");
    put(&dir.join("commands/repo.md"), CLAUDE_CMD)?;
    let settings = dir.join("settings.json");
    let text = match std::fs::read_to_string(&settings) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => "{}".into(),
        Err(e) => return Err(format!("{}: {e}", settings.display())),
    };
    let backup = dir.join("settings.json.bak-slashrepo");
    if !backup.exists() {
        put(&backup, &text)?;
    }
    let mut v: Value = serde_json::from_str(&text).map_err(|e| format!("{}: {e}", settings.display()))?;
    let old_hook = h.join(".local/bin/repo-status-hook");
    add_hook(&mut v, &format!("{} hook", sh_quote(&bin.to_string_lossy())), &old_hook.to_string_lossy())?;
    let out = serde_json::to_string_pretty(&v).map_err(|e| e.to_string())? + "\n";
    if out != text {
        put(&settings, &out)?; // temp file + rename: a failure never leaves settings.json empty
    }
    // the shell hook of 0.4.x is replaced by `repo-status hook`
    if std::fs::read_to_string(&old_hook).is_ok_and(|t| t.contains("Repo Status")) {
        let _ = std::fs::remove_file(&old_hook);
    }
    println!("Claude Code: /repo  (settings backup: {})", backup.display());
    Ok(())
}

fn trigger(file: &Path, h: &Path) -> Res {
    let f = file.to_string_lossy();
    if !file.is_absolute() || f.contains('\n') {
        return Err("--trigger needs an absolute path without newlines".into());
    }
    let config = std::env::var_os("XDG_CONFIG_HOME").map_or_else(|| h.join(".config"), PathBuf::from);
    let units = config.join("systemd/user");
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
    println!("trigger: writing {f} opens the window (set SLASHREPO_TRIGGER={f} inside the sandbox)");
    Ok(())
}

fn desktop_exec(bin: &Path) -> String {
    let b = bin.to_string_lossy();
    if b.contains([' ', '"', '\\', '`', '$']) {
        format!("\"{}\"", b.replace('\\', "\\\\").replace('"', "\\\"").replace('`', "\\`").replace('$', "\\$"))
    } else {
        b.into()
    }
}

pub fn run(trigger_file: Option<&Path>) -> Res {
    let h = home();
    let bin = h.join(".local/bin/repo-status");
    let me = std::env::current_exe().map_err(|e| e.to_string())?;
    let same = std::fs::canonicalize(&me).ok() == std::fs::canonicalize(&bin).ok();
    if !same {
        let data = std::fs::read(&me).map_err(|e| format!("{}: {e}", me.display()))?;
        // temp file + rename: works while the old binary is running
        write_atomic(&bin, &data, 0o755).map_err(|e| format!("{}: {e}", bin.display()))?;
    }
    // icon + desktop file named after the app id: GNOME matches the window to it (else a gear)
    put(&h.join(format!(".local/share/icons/hicolor/scalable/apps/{}.svg", crate::APP_ID)), ICON)?;
    put(
        &h.join(format!(".local/share/applications/{}.desktop", crate::APP_ID)),
        &format!(
            "[Desktop Entry]\nType=Application\nName=Repo Status\n\
             Comment=Where each of your repos stands: release, CI, issues, PRs, to-dos\n\
             Exec={}\nIcon={}\nStartupWMClass={}\nCategories=Development;\n",
            desktop_exec(&bin),
            crate::APP_ID,
            crate::APP_ID
        ),
    )?;
    // the old name (up to 0.5.1) would show a second "Repo Status" in the app menu
    let _ = std::fs::remove_file(h.join(".local/share/applications/repo-status.desktop"));
    println!("installed: {} (window, app menu)", bin.display());
    if !has("gh") {
        println!("note: gh is not installed; GitHub repos need it (logged in). GitLab and Codeberg work without.");
    }

    // Claude Code: /repo command + a UserPromptSubmit hook that opens the window (0 tokens).
    if h.join(".claude").is_dir() || has("claude") {
        claude(&bin, &h)?;
    }
    // opencode: the command runs repo-status itself (0 tokens).
    if h.join(".config/opencode").is_dir() || has("opencode") {
        put(&h.join(".config/opencode/command/repo.md"), OPENCODE_CMD)?;
        println!("opencode: /repo");
    }
    // Gemini CLI: the command runs repo-status itself (asks once before it runs a shell command).
    if h.join(".gemini/settings.json").is_file() || has("gemini") {
        put(&h.join(".gemini/commands/repo.toml"), GEMINI_CMD)?;
        println!("Gemini CLI: /repo");
    }
    // Codex and Antigravity: a "repo" skill; the agent runs the one command (the report stays 0 tokens).
    if h.join(".codex").is_dir() || has("codex") {
        put(&h.join(".agents/skills/repo/SKILL.md"), SKILL)?;
        println!("Codex: $repo (or pick it in /skills)");
    }
    if h.join(".gemini/antigravity").is_dir() || h.join(".gemini/config").is_dir() || has("agy") {
        put(&h.join(".gemini/config/skills/repo/SKILL.md"), SKILL)?;
        println!("Antigravity: /repo");
    }
    if let Some(f) = trigger_file {
        trigger(f, &h)?;
    }
    println!("Optional: list repos in ~/.config/slashrepo/repos (see the README)");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hook_once_and_old_one_replaced() {
        let mut v = json!({"theme": "x", "hooks": {"UserPromptSubmit": [
            {"hooks": [{"type": "command", "command": "/h/.local/bin/repo-status-hook", "timeout": 5}]}]}});
        add_hook(&mut v, "/h/.local/bin/repo-status hook", "/h/.local/bin/repo-status-hook").unwrap();
        add_hook(&mut v, "/h/.local/bin/repo-status hook", "/h/.local/bin/repo-status-hook").unwrap();
        let list = v["hooks"]["UserPromptSubmit"].as_array().unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0]["hooks"][0]["command"], "/h/.local/bin/repo-status hook");
        assert_eq!(v["theme"], "x");
        assert_eq!(sh_quote("/home/a b/x"), "'/home/a b/x'");
    }
}
