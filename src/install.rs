//! `repo-status install [--trigger FILE]`: the binary in ~/.local/bin and a desktop entry (Windows:
//! %LOCALAPPDATA%\Programs\slashRepo and a Start Menu entry), then /repo for every coding agent
//! found on this machine (Claude Code, opencode, Codex, Antigravity, Gemini CLI). Safe to run again.

use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use crate::{home, mode_of, write_atomic, write_atomic_keep};

const CLAUDE_CMD: &str = include_str!("../commands/repo.md");
const OPENCODE_CMD: &str = include_str!("../commands/opencode/repo.md");
const GEMINI_CMD: &str = include_str!("../commands/gemini/repo.toml");
const SKILL: &str = include_str!("../skills/repo/SKILL.md");
const TRIGGER_PATH: &str = include_str!("../systemd/slashrepo-trigger.path");
const TRIGGER_SERVICE: &str = include_str!("../systemd/slashrepo-trigger.service");
const ICON: &str = include_str!("../data/io.github.ludalunizz.SlashRepo.svg");

type Res = Result<(), String>;
/// One step of `install`: what it did, or why it failed.
type Step = Result<String, String>;

/// Where `install` puts the binary.
pub fn bin_path(h: &Path) -> PathBuf {
    if cfg!(windows) {
        local_appdata(h).join("Programs").join("slashRepo").join("repo-status.exe")
    } else {
        h.join(".local/bin/repo-status")
    }
}

/// The command and skill files `install` writes for each agent (name, file).
pub fn agent_files(h: &Path) -> [(&'static str, PathBuf); 5] {
    [
        ("Claude Code", h.join(".claude/commands/repo.md")),
        ("opencode", h.join(".config/opencode/commands/repo.md")),
        ("Gemini CLI", h.join(".gemini/commands/repo.toml")),
        ("Codex", h.join(".agents/skills/repo/SKILL.md")),
        ("Antigravity", h.join(".gemini/config/skills/repo/SKILL.md")),
    ]
}

pub fn claude_settings(h: &Path) -> PathBuf {
    h.join(".claude/settings.json")
}

/// The copy of settings.json made before `install` or `uninstall` changes it.
pub fn settings_backup(h: &Path) -> PathBuf {
    h.join(".claude/settings.json.bak-slashrepo")
}

/// Where slashRepo up to 0.6.1 put the opencode command (opencode reads both `command/` and
/// `commands/`; the second is the current name).
pub fn old_opencode_file(h: &Path) -> PathBuf {
    h.join(".config/opencode/command/repo.md")
}

/// The opencode step: the command in `commands/`, and an older copy of ours in `command/` taken
/// away, so /repo is not listed twice. A `command/repo.md` of the user's own stays.
fn opencode(h: &Path, path: &Path, text: &str) -> Step {
    let said = agent_file(path, text, "/repo")?;
    let old = old_opencode_file(h);
    match std::fs::read_to_string(&old) {
        Ok(t) if is_ours(&t) => {
            std::fs::remove_file(&old).map_err(|e| format!("{}: {e}", old.display()))?;
            Ok(format!("{said} (moved from {})", old.display()))
        }
        _ => Ok(said),
    }
}

/// The 0.4.x shell hook, replaced by `repo-status hook`.
pub fn old_hook(h: &Path) -> PathBuf {
    h.join(".local/bin/repo-status-hook")
}

/// The command `install` puts in the Claude Code hook.
pub fn hook_command(bin: &Path) -> String {
    format!("{} hook", shell_path(bin))
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
fn has(cmd: &str) -> bool {
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

enum Put {
    Written,
    /// a file of the user's own is there: left alone
    Kept,
}

/// put, but never over a file we did not write. (A file that is not text is not ours either.)
fn put_ours(path: &Path, text: &str) -> Result<Put, String> {
    match std::fs::read_to_string(path) {
        Ok(old) if !is_ours(&old) => return Ok(Put::Kept),
        Err(e) if e.kind() == std::io::ErrorKind::InvalidData => return Ok(Put::Kept),
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => return Err(format!("{}: {e}", path.display())),
        _ => {}
    }
    put(path, text).map(|()| Put::Written)
}

/// One agent's command or skill file; says so when it leaves the user's own file alone.
fn agent_file(path: &Path, text: &str, done: &str) -> Step {
    match put_ours(path, text)? {
        Put::Written => Ok(done.into()),
        Put::Kept => Ok(format!("kept your own {} (it does not call repo-status), so no /repo here", path.display())),
    }
}

/// The binary as the agents' shells should call it. Unix: quoted for sh when needed. Windows:
/// forward slashes, in double quotes when needed: the same text then works in Git Bash (Claude
/// Code's hook shell) and cmd.
fn shell_path(bin: &Path) -> String {
    let b = bin.to_string_lossy();
    if cfg!(windows) {
        let b = b.replace('\\', "/");
        if b.bytes().all(|c| c.is_ascii_alphanumeric() || b"/._-+:".contains(&c)) {
            b
        } else {
            format!("\"{}\"", b.replace('$', "\\$").replace('`', "\\`"))
        }
    } else {
        sh_quote(&b)
    }
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

/// /repo + the prompt hook. settings.json is read and checked before anything is written, so a
/// broken one changes nothing; it keeps its mode and symlink, and a backup of the state before this
/// change sits next to it.
fn claude(bin: &Path, h: &Path) -> Step {
    let cmd = if cfg!(windows) {
        CLAUDE_CMD.replace("~/.local/bin/repo-status", "repo-status.exe")
    } else {
        CLAUDE_CMD.into()
    };
    let settings = claude_settings(h);
    let (text, existed) = match std::fs::read_to_string(&settings) {
        Ok(t) => (t, true),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => ("{}".to_string(), false),
        Err(e) => return Err(format!("{}: {e}", settings.display())),
    };
    let mut v: Value = serde_json::from_str(&text).map_err(|e| format!("{}: {e}", settings.display()))?;
    let old = old_hook(h);
    add_hook(&mut v, &hook_command(bin), &old.to_string_lossy())?;
    let out = serde_json::to_string_pretty(&v).map_err(|e| e.to_string())? + "\n";

    let mut said = String::from("/repo");
    let command_file = &agent_files(h)[0].1;
    if let Put::Kept = put_ours(command_file, &cmd)? {
        said = format!(
            "kept your own {} (it does not call repo-status); the hook still opens the window",
            command_file.display()
        );
    }
    if out != text {
        if existed {
            let backup = settings_backup(h);
            let mode = mode_of(&settings).unwrap_or(0o644);
            write_atomic(&backup, text.as_bytes(), mode).map_err(|e| format!("{}: {e}", backup.display()))?;
            said += &format!("  (settings backup: {})", backup.display());
        }
        // temp file + rename: a failure never leaves settings.json empty
        write_atomic_keep(&settings, out.as_bytes(), 0o644).map_err(|e| format!("{}: {e}", settings.display()))?;
    }
    // the shell hook of 0.4.x is replaced by `repo-status hook`
    if std::fs::read_to_string(&old).is_ok_and(|t| t.contains("Repo Status")) {
        let _ = std::fs::remove_file(&old);
    }
    Ok(said)
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

/// What opencode, Gemini CLI, Codex and Antigravity run for /repo on Windows: `repo-status open`
/// (starts the window, prints one line).
fn windows_agent_files(bin: &Path) -> (String, String, String) {
    let call = format!("{} open", shell_path(bin));
    let reply = "The line above comes from this command. Reply with exactly that one line and nothing else. Do not run anything.";
    let opencode =
        format!("---\ndescription: Open the Repo Status window (releases, CI, PRs, issues)\n---\n!`{call}`\n{reply}\n");
    let gemini = format!(
        "description = \"Open the Repo Status window (releases, CI, PRs, issues)\"\nprompt = \"\"\"\n!{{{call}}}\n{reply}\n\"\"\"\n"
    );
    // PowerShell (Codex's shell on Windows): single quotes, nothing expands inside
    let ps = format!("& '{}' open", bin.to_string_lossy().replace('\'', "''"));
    let skill = SKILL.replace(
        "setsid -f ~/.local/bin/repo-status >/dev/null 2>&1 </dev/null",
        &format!("{call}\n\nIn PowerShell:\n\n    {ps}"),
    );
    (opencode, gemini, skill)
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
    let (opencode_cmd, gemini_cmd, skill) =
        if cfg!(windows) { windows_agent_files(&bin) } else { (OPENCODE_CMD.into(), GEMINI_CMD.into(), SKILL.into()) };

    // Every agent is a step of its own: one that fails does not stop the others.
    let files = agent_files(&h);
    let mut steps: Vec<(&str, Step)> = Vec::new();
    // Claude Code: /repo command + a UserPromptSubmit hook that opens the window (0 tokens).
    if h.join(".claude").is_dir() || has("claude") {
        steps.push(("Claude Code", claude(&bin, &h)));
    }
    // opencode: the command runs repo-status itself (0 tokens).
    if h.join(".config/opencode").is_dir() || has("opencode") {
        steps.push(("opencode", opencode(&h, &files[1].1, &opencode_cmd)));
    }
    // Gemini CLI: the command runs repo-status itself (asks once before it runs a shell command).
    if h.join(".gemini/settings.json").is_file() || has("gemini") {
        steps.push(("Gemini CLI", agent_file(&files[2].1, &gemini_cmd, "/repo")));
    }
    // Codex and Antigravity: a "repo" skill; the agent runs the one command (the report stays 0 tokens).
    if h.join(".codex").is_dir() || has("codex") {
        steps.push(("Codex", agent_file(&files[3].1, &skill, "$repo (or pick it in /skills)")));
    }
    if h.join(".gemini/antigravity").is_dir() || h.join(".gemini/config").is_dir() || has("agy") {
        steps.push(("Antigravity", agent_file(&files[4].1, &skill, "/repo")));
    }
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
