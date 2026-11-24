//! `repo-status hook`: the Claude Code UserPromptSubmit hook. Opens the window when the prompt says
//! "repo check", "repos check", "check the repos" or a line starts with /repo. Always exits 0.
//!
//! A prompt that is ONLY the command (/repo, "repo check", "please check the repos") is blocked
//! after the window is started: Claude Code then drops it, so nothing reaches the model and the
//! report costs 0 tokens (the reason text is shown to you only). A longer prompt that merely
//! mentions it still opens the window but goes through, so the rest of what you wrote isn't lost.

use std::io::Read;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::LazyLock;

use regex::Regex;

static OPENS: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?im)repos? check|check (up on )?(the |my )?repos\b|^ */repo( |$)").unwrap());
static ONLY: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)^\s*(/repo|(please )?(repos? check|check (up on )?(the |my )?repos)( now| please)?[.!]?)\s*$")
        .unwrap()
});

pub const BLOCK: &str = r#"{"decision":"block","reason":"Repo Status window opened (0 tokens)."}"#;

/// (open the window, block the prompt)
pub fn decide(prompt: &str) -> (bool, bool) {
    let open = OPENS.is_match(prompt);
    (open, open && ONLY.is_match(prompt))
}

/// Start the window detached from the hook (own session), like `setsid -f`.
fn open(window: &Path) {
    let null = || Stdio::null();
    let spawned = Command::new("setsid").arg("-f").arg(window).stdin(null()).stdout(null()).stderr(null()).status();
    if spawned.is_err() {
        use std::os::unix::process::CommandExt;
        let _ = Command::new(window).process_group(0).stdin(null()).stdout(null()).stderr(null()).spawn();
    }
}

pub fn run(window: &Path) {
    let mut input = String::new();
    if std::io::stdin().take(16 << 20).read_to_string(&mut input).is_err() {
        return;
    }
    let prompt = serde_json::from_str::<serde_json::Value>(&input)
        .ok()
        .and_then(|v| v.get("prompt")?.as_str().map(String::from))
        .unwrap_or_default();
    let (open_it, block) = decide(&prompt);
    if open_it {
        open(window);
    }
    if block {
        println!("{BLOCK}");
    }
}

#[cfg(test)]
mod tests {
    use super::decide;

    #[test]
    fn prompts() {
        // bare command: window opens AND the prompt is blocked (0 tokens)
        for p in ["/repo", "  /repo  ", "repo check", "please check the repos", "repos check now", "Repo check!"] {
            assert_eq!(decide(p), (true, true), "{p}");
        }
        // mentioned inside a longer prompt: window opens, prompt goes through
        for p in ["/repo and then fix the tab bug", "do a repo check, then bump the version", "hi\n/repo"] {
            assert_eq!(decide(p), (true, false), "{p}");
        }
        for p in ["fix the bug in repository.py", "hello", "see /repository", ""] {
            assert_eq!(decide(p), (false, false), "{p}");
        }
    }
}
