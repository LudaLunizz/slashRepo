#!/bin/sh
# Install slashRepo: the window in ~/.local/bin, then /repo for every coding agent found on this
# machine (Claude Code, opencode, Codex, Antigravity, Gemini CLI). Safe to run again.
# Needs: python3 with PyGObject (GTK4) for the window, gh (logged in) for GitHub repos, jq for Claude Code.
set -eu
here=$(cd "$(dirname "$0")" && pwd)
# --trigger FILE: also install a systemd user unit that opens the window when FILE is written
# (for agents in a sandbox, see the README). FILE must be a path the sandbox can write.
trigger=""
if [ "${1:-}" = "--trigger" ]; then
  case "${2:-}" in /*) trigger=$2 ;; *) echo "usage: $0 [--trigger /absolute/path/to/file]" >&2; exit 1 ;; esac
fi
for c in gh python3; do command -v "$c" >/dev/null || { echo "missing: $c" >&2; exit 1; }; done
has() { command -v "$1" >/dev/null 2>&1; }

mkdir -p "$HOME/.local/bin" "$HOME/.local/share/applications"
install -m 755 "$here/bin/repo-status" "$here/bin/repo-status-hook" "$HOME/.local/bin/"
cat > "$HOME/.local/share/applications/repo-status.desktop" <<DESK
[Desktop Entry]
Type=Application
Name=Repo Status
Comment=Where each of your repos stands: release, CI, issues, PRs, to-dos
Exec=$HOME/.local/bin/repo-status
Icon=utilities-system-monitor
Categories=Development;
DESK
echo "installed: repo-status (window, app menu)"

# Claude Code: /repo command + a UserPromptSubmit hook that opens the window (0 tokens).
if [ -d "$HOME/.claude" ] || has claude; then
  has jq || { echo "missing: jq (needed for Claude Code)" >&2; exit 1; }
  settings="$HOME/.claude/settings.json"
  mkdir -p "$HOME/.claude/commands"
  install -m 644 "$here/commands/repo.md" "$HOME/.claude/commands/repo.md"
  [ -f "$settings" ] || echo '{}' > "$settings"
  [ -e "$settings.bak-slashrepo" ] || cp "$settings" "$settings.bak-slashrepo"
  hook="$HOME/.local/bin/repo-status-hook"
  # Write to a temp file first: a jq failure must not leave settings.json empty.
  jq --arg c "$hook" '
    .hooks.UserPromptSubmit = ((.hooks.UserPromptSubmit // [])
      | if any(.[]; .hooks[]?.command == $c) then . else . + [{hooks: [{type: "command", command: $c, timeout: 5}]}] end)
  ' "$settings" > "$settings.new" && mv "$settings.new" "$settings"
  echo "Claude Code: /repo  (settings backup: $settings.bak-slashrepo)"
fi

# opencode: the command runs repo-status itself (0 tokens).
if [ -d "$HOME/.config/opencode" ] || has opencode; then
  mkdir -p "$HOME/.config/opencode/command"
  install -m 644 "$here/commands/opencode/repo.md" "$HOME/.config/opencode/command/repo.md"
  echo "opencode: /repo"
fi

# Gemini CLI: the command runs repo-status itself (asks once before it runs a shell command).
if [ -f "$HOME/.gemini/settings.json" ] || has gemini; then
  mkdir -p "$HOME/.gemini/commands"
  install -m 644 "$here/commands/gemini/repo.toml" "$HOME/.gemini/commands/repo.toml"
  echo "Gemini CLI: /repo"
fi

# Codex and Antigravity: a "repo" skill; the agent runs the one command (the report stays 0 tokens).
skill() { mkdir -p "$1/repo" && install -m 644 "$here/skills/repo/SKILL.md" "$1/repo/SKILL.md"; }
if [ -d "$HOME/.codex" ] || has codex; then
  skill "$HOME/.agents/skills"
  echo "Codex: \$repo (or pick it in /skills)"
fi
if [ -d "$HOME/.gemini/antigravity" ] || [ -d "$HOME/.gemini/config" ] || has agy; then
  skill "$HOME/.gemini/config/skills"
  echo "Antigravity: /repo"
fi

if [ -n "$trigger" ]; then
  units="${XDG_CONFIG_HOME:-$HOME/.config}/systemd/user"
  mkdir -p "$units" "$(dirname "$trigger")"
  [ -e "$trigger" ] || : > "$trigger"
  install -m 644 "$here/systemd/slashrepo-trigger.service" "$units/"
  # | as sed delimiter: the path has slashes; refuse paths with | or newlines
  case "$trigger" in *"|"*|*"
"*) echo "trigger path may not contain | or a newline" >&2; exit 1 ;; esac
  sed "s|@TRIGGER@|$trigger|" "$here/systemd/slashrepo-trigger.path" > "$units/slashrepo-trigger.path"
  systemctl --user daemon-reload && systemctl --user enable --now slashrepo-trigger.path
  echo "trigger: writing $trigger opens the window (set SLASHREPO_TRIGGER=$trigger inside the sandbox)"
fi

echo "Optional: list repos in ~/.config/slashrepo/repos (see the README)"
