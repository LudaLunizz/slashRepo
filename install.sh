#!/bin/sh
# Install slashRepo: script + hook in ~/.local/bin, /repo command, hook entry in settings.json.
# Safe to run again. Needs: python3 with PyGObject (GTK4) for the window, gh (logged in), jq.
set -eu
here=$(cd "$(dirname "$0")" && pwd)
settings="$HOME/.claude/settings.json"
for c in gh jq python3; do command -v "$c" >/dev/null || { echo "missing: $c" >&2; exit 1; }; done

mkdir -p "$HOME/.local/bin" "$HOME/.claude/commands" "$HOME/.local/share/applications"
install -m 755 "$here/bin/repo-status" "$here/bin/repo-status-hook" "$HOME/.local/bin/"
install -m 644 "$here/commands/repo.md" "$HOME/.claude/commands/repo.md"
cat > "$HOME/.local/share/applications/repo-status.desktop" <<DESK
[Desktop Entry]
Type=Application
Name=Repo Status
Comment=Where each of your repos stands: release, CI, issues, PRs, to-dos
Exec=$HOME/.local/bin/repo-status
Icon=utilities-system-monitor
Categories=Development;
DESK

[ -f "$settings" ] || echo '{}' > "$settings"
[ -e "$settings.bak-slashrepo" ] || cp "$settings" "$settings.bak-slashrepo"
hook="$HOME/.local/bin/repo-status-hook"
# Write to a temp file first: a jq failure must not leave settings.json empty.
jq --arg c "$hook" '
  .hooks.UserPromptSubmit = ((.hooks.UserPromptSubmit // [])
    | if any(.[]; .hooks[]?.command == $c) then . else . + [{hooks: [{type: "command", command: $c, timeout: 5}]}] end)
' "$settings" > "$settings.new" && mv "$settings.new" "$settings"
echo "installed. Settings backup: $settings.bak-slashrepo"
echo "Optional: list repos one owner/name per line in ~/.config/slashrepo/repos"
