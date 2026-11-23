#!/bin/sh
# install.sh in a throwaway HOME: /repo lands for each agent that is present, and only for those.
set -u
here=$(cd "$(dirname "$0")/.." && pwd)
h=$(mktemp -d); trap 'rm -rf "$h"' EXIT
fail=0
need() { [ -e "$h/$1" ] || { echo "FAIL: missing $1"; fail=1; }; }
none() { [ ! -e "$h/$1" ] || { echo "FAIL: should not exist: $1"; fail=1; }; }
# PATH without the real agent CLIs, so only the folders below decide (gh, jq, python3 linked in).
mkdir "$h/bin"; for c in gh jq python3; do ln -s "$(command -v "$c")" "$h/bin/$c"; done
P=$h/bin:/bin
mkdir -p "$h/.claude" "$h/.config/opencode" "$h/.codex" "$h/.gemini/antigravity"
echo '{"theme":"x"}' > "$h/.claude/settings.json"; echo '{}' > "$h/.gemini/settings.json"
HOME=$h PATH=$P sh "$here/install.sh" >/dev/null || { echo "FAIL: install.sh exited $?"; fail=1; }
HOME=$h PATH=$P sh "$here/install.sh" >/dev/null || fail=1   # twice: changes nothing
for f in .local/bin/repo-status .claude/commands/repo.md .config/opencode/command/repo.md \
         .gemini/commands/repo.toml .agents/skills/repo/SKILL.md .gemini/config/skills/repo/SKILL.md; do need "$f"; done
[ "$(jq '.hooks.UserPromptSubmit | length' "$h/.claude/settings.json")" = 1 ] || { echo "FAIL: hook not once"; fail=1; }
[ "$(jq -r .theme "$h/.claude/settings.json")" = x ] || { echo "FAIL: settings lost"; fail=1; }
# A machine with none of the agents gets only the window.
h2=$h/bare; mkdir "$h2"
HOME=$h2 PATH=$P sh "$here/install.sh" >/dev/null || fail=1
[ -e "$h2/.local/bin/repo-status" ] || { echo "FAIL: bare: no window"; fail=1; }
for d in .claude .config/opencode .gemini .agents .codex; do [ ! -e "$h2/$d" ] || { echo "FAIL: bare: made $d"; fail=1; }; done
[ "$fail" = 0 ] && echo "install tests ok"; exit "$fail"
