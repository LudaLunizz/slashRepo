//! Claude Code's settings.json, the one file of the user's that both `install` and `uninstall`
//! change: read and checked before anything is written (a broken file changes nothing), written
//! back only when it changed (temp file + rename, mode and symlink kept), with a backup of the
//! state before the change next to it.

use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use crate::{mode_of, write_atomic, write_atomic_keep};

pub fn path(h: &Path) -> PathBuf {
    h.join(".claude/settings.json")
}

/// The copy of settings.json made before `install` or `uninstall` changes it.
pub fn backup_path(h: &Path) -> PathBuf {
    h.join(".claude/settings.json.bak-slashrepo")
}

pub struct Settings {
    path: PathBuf,
    backup: PathBuf,
    /// the file as read; None when there was none
    before: Option<String>,
    pub value: Value,
}

impl Settings {
    /// A missing file reads as `{}`; one that cannot be read or parsed is an error.
    pub fn read(h: &Path) -> Result<Settings, String> {
        let (path, backup) = (path(h), backup_path(h));
        let before = match std::fs::read_to_string(&path) {
            Ok(t) => Some(t),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
            Err(e) => return Err(format!("{}: {e}", path.display())),
        };
        let value =
            serde_json::from_str(before.as_deref().unwrap_or("{}")).map_err(|e| format!("{}: {e}", path.display()))?;
        Ok(Settings { path, backup, before, value })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Write the change, if there is one. Returns the backup made of the old file (None: nothing
    /// written, or there was no old file).
    pub fn save(&self) -> Result<Option<PathBuf>, String> {
        let out = serde_json::to_string_pretty(&self.value).map_err(|e| e.to_string())? + "\n";
        if self.before.as_deref() == Some(out.as_str()) {
            return Ok(None);
        }
        let mut backup = None;
        if let Some(old) = &self.before {
            let mode = mode_of(&self.path).unwrap_or(0o644);
            write_atomic(&self.backup, old.as_bytes(), mode).map_err(|e| format!("{}: {e}", self.backup.display()))?;
            backup = Some(self.backup.clone());
        }
        // temp file + rename: a failure never leaves settings.json empty
        write_atomic_keep(&self.path, out.as_bytes(), 0o644).map_err(|e| format!("{}: {e}", self.path.display()))?;
        Ok(backup)
    }
}

/// The prompt-hook event /repo uses.
const EVENT: &str = "UserPromptSubmit";

fn command_of(h: &Value) -> Option<&str> {
    h.get("command").and_then(Value::as_str)
}

/// Add `command` to UserPromptSubmit once; an entry with an `old` command becomes `command`.
pub fn add_hook(settings: &mut Value, command: &str, old: &str) -> Result<(), String> {
    let root = settings.as_object_mut().ok_or("settings.json is not a JSON object")?;
    let hooks = root.entry("hooks").or_insert_with(|| json!({}));
    let hooks = hooks.as_object_mut().ok_or("settings.json: hooks is not an object")?;
    let list = hooks.entry(EVENT).or_insert_with(|| json!([]));
    let list = list.as_array_mut().ok_or("settings.json: hooks.UserPromptSubmit is not a list")?;
    let mut found = false;
    for group in list.iter_mut() {
        for h in group.get_mut("hooks").and_then(Value::as_array_mut).into_iter().flatten() {
            if command_of(h) == Some(old) {
                h["command"] = json!(command);
            }
            found |= command_of(h) == Some(command);
        }
    }
    if !found {
        list.push(json!({"hooks": [{"type": "command", "command": command, "timeout": 5}]}));
    }
    Ok(())
}

/// Remove every UserPromptSubmit hook whose command is in `commands`. A group, the list or the
/// `hooks` object that this empties goes too; nothing else is touched. Returns how many were removed.
pub fn remove_hook(settings: &mut Value, commands: &[String]) -> usize {
    let mut removed = 0;
    let mut hooks_empty = false;
    if let Some(hooks) = settings.get_mut("hooks").and_then(Value::as_object_mut) {
        let mut list_empty = false;
        if let Some(list) = hooks.get_mut(EVENT).and_then(Value::as_array_mut) {
            let mut emptied = Vec::new();
            for (i, group) in list.iter_mut().enumerate() {
                let Some(inner) = group.get_mut("hooks").and_then(Value::as_array_mut) else { continue };
                let before = inner.len();
                inner.retain(|h| !command_of(h).is_some_and(|c| commands.iter().any(|x| x == c)));
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
            hooks.remove(EVENT);
        }
        hooks_empty = removed > 0 && hooks.is_empty();
    }
    if hooks_empty && let Some(root) = settings.as_object_mut() {
        root.remove("hooks");
    }
    removed
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
        assert!(add_hook(&mut json!([]), "c", "o").is_err());
        assert!(add_hook(&mut json!({"hooks": {"UserPromptSubmit": {}}}), "c", "o").is_err());
    }

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

    #[test]
    fn add_then_remove_gives_the_file_back() {
        let start = json!({"theme": "x", "hooks": {"Stop": [{"hooks": [{"type": "command", "command": "a"}]}]}});
        let mut v = start.clone();
        add_hook(&mut v, "/b/repo-status hook", "/b/repo-status-hook").unwrap();
        assert_ne!(v, start);
        assert_eq!(remove_hook(&mut v, &["/b/repo-status hook".into()]), 1);
        assert_eq!(v, start);
    }
}
