//! The coding agents /repo is set up for, in one table that `install` and `uninstall` both walk:
//! how to tell the agent is on this machine, where its /repo file goes (and went before), and how
//! to write it. A new agent is one more row.

use std::path::{Path, PathBuf};

use crate::install::{has, is_ours, put_ours};
use crate::settings::{self, Settings};

const CLAUDE_CMD: &str = include_str!("../commands/repo.md");
const OPENCODE_CMD: &str = include_str!("../commands/opencode/repo.md");
const GEMINI_CMD: &str = include_str!("../commands/gemini/repo.toml");
const SKILL: &str = include_str!("../skills/repo/SKILL.md");

/// One step of `install`: what it did, or why it failed.
pub type Step = Result<String, String>;

pub struct Agent {
    pub name: &'static str,
    /// on this machine: its config folder is there, or the program is on PATH
    found: fn(&Path) -> bool,
    /// the command or skill file of /repo
    pub file: fn(&Path) -> PathBuf,
    /// where older versions of slashRepo put that file; ours there is taken away
    pub old_file: Option<fn(&Path) -> PathBuf>,
    /// write /repo (Claude Code: and the prompt hook)
    install: fn(&Path, &Path) -> Step,
}

impl Agent {
    pub fn found(&self, h: &Path) -> bool {
        (self.found)(h)
    }

    /// Set /repo up in home `h` for the binary `bin`.
    pub fn install(&self, h: &Path, bin: &Path) -> Step {
        (self.install)(h, bin)
    }
}

pub const AGENTS: [Agent; 5] = [
    // the /repo command + a UserPromptSubmit hook that opens the window (0 tokens)
    Agent {
        name: "Claude Code",
        found: |h| h.join(".claude").is_dir() || has("claude"),
        file: |h| h.join(".claude/commands/repo.md"),
        old_file: None,
        install: claude,
    },
    // the command runs repo-status itself (0 tokens)
    Agent {
        name: "opencode",
        found: |h| h.join(".config/opencode").is_dir() || has("opencode"),
        file: |h| h.join(".config/opencode/commands/repo.md"),
        // up to 0.6.1 (opencode reads both `command/` and `commands/`; the second is the current name)
        old_file: Some(|h| h.join(".config/opencode/command/repo.md")),
        install: |h, bin| own_file(h, bin, 1, "/repo"),
    },
    // the command runs repo-status itself (asks once before it runs a shell command)
    Agent {
        name: "Gemini CLI",
        found: |h| h.join(".gemini/settings.json").is_file() || has("gemini"),
        file: |h| h.join(".gemini/commands/repo.toml"),
        old_file: None,
        install: |h, bin| own_file(h, bin, 2, "/repo"),
    },
    // a "repo" skill; the agent runs the one command (the report stays 0 tokens)
    Agent {
        name: "Codex",
        found: |h| h.join(".codex").is_dir() || has("codex"),
        file: |h| h.join(".agents/skills/repo/SKILL.md"),
        old_file: None,
        install: |h, bin| own_file(h, bin, 3, "$repo (or pick it in /skills)"),
    },
    Agent {
        name: "Antigravity",
        found: |h| h.join(".gemini/antigravity").is_dir() || h.join(".gemini/config").is_dir() || has("agy"),
        file: |h| h.join(".gemini/config/skills/repo/SKILL.md"),
        old_file: None,
        install: |h, bin| own_file(h, bin, 4, "/repo"),
    },
];

/// The command `install` puts in the Claude Code hook.
pub fn hook_command(bin: &Path) -> String {
    format!("{} hook", shell_path(bin))
}

/// The 0.4.x shell hook, replaced by `repo-status hook`.
pub fn old_hook(h: &Path) -> PathBuf {
    h.join(".local/bin/repo-status-hook")
}

/// The hook commands that are ours: the current one and the 0.4.x script.
pub fn our_hooks(h: &Path, bin: &Path) -> [String; 2] {
    [hook_command(bin), old_hook(h).to_string_lossy().into_owned()]
}

/// The text of AGENTS[i]'s file. On Windows opencode, Gemini CLI, Codex and Antigravity run
/// `repo-status open` (starts the window, prints one line).
fn text(i: usize, bin: &Path) -> String {
    if !cfg!(windows) {
        return [CLAUDE_CMD, OPENCODE_CMD, GEMINI_CMD, SKILL, SKILL][i].into();
    }
    let call = format!("{} open", shell_path(bin));
    let reply = "The line above comes from this command. Reply with exactly that one line and nothing else. Do not run anything.";
    match i {
        0 => CLAUDE_CMD.replace("~/.local/bin/repo-status", "repo-status.exe"),
        1 => format!(
            "---\ndescription: Open the Repo Status window (releases, CI, PRs, issues)\n---\n!`{call}`\n{reply}\n"
        ),
        2 => format!(
            "description = \"Open the Repo Status window (releases, CI, PRs, issues)\"\nprompt = \"\"\"\n!{{{call}}}\n{reply}\n\"\"\"\n"
        ),
        _ => {
            // PowerShell (Codex's shell on Windows): single quotes, nothing expands inside
            let ps = format!("& '{}' open", bin.to_string_lossy().replace('\'', "''"));
            SKILL.replace(
                "setsid -f ~/.local/bin/repo-status >/dev/null 2>&1 </dev/null",
                &format!("{call}\n\nIn PowerShell:\n\n    {ps}"),
            )
        }
    }
}

/// An agent whose /repo is one file of ours: written, or left alone when the user has their own.
/// An older copy of ours in the agent's old place goes, so /repo is not listed twice; a file of the
/// user's own there stays.
fn own_file(h: &Path, bin: &Path, i: usize, done: &str) -> Step {
    let a = &AGENTS[i];
    let path = (a.file)(h);
    if !put_ours(&path, &text(i, bin))? {
        return Ok(format!("kept your own {} (it does not call repo-status), so no /repo here", path.display()));
    }
    let Some(old) = a.old_file.map(|f| f(h)) else { return Ok(done.into()) };
    match std::fs::read_to_string(&old) {
        Ok(t) if is_ours(&t) => {
            std::fs::remove_file(&old).map_err(|e| format!("{}: {e}", old.display()))?;
            Ok(format!("{done} (moved from {})", old.display()))
        }
        _ => Ok(done.into()),
    }
}

/// /repo + the prompt hook. settings.json is read and changed in memory before anything is written,
/// so a broken one changes nothing.
fn claude(h: &Path, bin: &Path) -> Step {
    let mut s = Settings::read(h)?;
    let [command, old] = our_hooks(h, bin);
    settings::add_hook(&mut s.value, &command, &old)?;
    let file = (AGENTS[0].file)(h);
    let mut said = String::from("/repo");
    if !put_ours(&file, &text(0, bin))? {
        said =
            format!("kept your own {} (it does not call repo-status); the hook still opens the window", file.display());
    }
    if let Some(backup) = s.save()? {
        said += &format!("  (settings backup: {})", backup.display());
    }
    // the shell hook of 0.4.x is replaced by `repo-status hook`
    if std::fs::read_to_string(old_hook(h)).is_ok_and(|t| t.contains("Repo Status")) {
        let _ = std::fs::remove_file(old_hook(h));
    }
    Ok(said)
}

/// The binary as the agents' shells should call it. Unix: quoted for sh when needed. Windows:
/// forward slashes, in double quotes when needed: the same text then works in Git Bash (Claude
/// Code's hook shell) and cmd.
pub fn shell_path(bin: &Path) -> String {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn table_rows_are_distinct_and_texts_call_us() {
        let h = Path::new("/h");
        let bin = Path::new("/h/.local/bin/repo-status");
        let files: Vec<PathBuf> = AGENTS.iter().map(|a| (a.file)(h)).collect();
        for (i, f) in files.iter().enumerate() {
            assert!(f.starts_with(h));
            assert!(!files[..i].contains(f), "two agents share {}", f.display());
            assert!(is_ours(&text(i, bin)), "{}'s file must call repo-status", AGENTS[i].name);
        }
        assert_eq!(AGENTS[0].name, "Claude Code", "claude() writes AGENTS[0]'s file");
        assert_eq!(sh_quote("/home/a b/x"), "'/home/a b/x'");
        // the old hook is a joined path: `\` before `.local` on Windows
        let old = h.join(".local/bin/repo-status-hook").to_string_lossy().into_owned();
        assert_eq!(our_hooks(h, bin), [format!("{} hook", shell_path(bin)), old]);
    }
}
