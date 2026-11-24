#!/bin/sh
# The hook opens the window for the right prompts and only those.
set -u
dir=$(mktemp -d); trap 'rm -rf "$dir"' EXIT
printf '#!/bin/sh\ntouch "%s/opened"\n' "$dir" > "$dir/fake"; chmod +x "$dir/fake"
hook=$(cd "$(dirname "$0")/.." && pwd)/bin/repo-status-hook
fail=0
try() { # expected opened(0/1) expected blocked(0/1) prompt
  rm -f "$dir/opened"
  out=$(printf '{"prompt":%s}' "$(printf '%s' "$3" | jq -Rs .)" | REPO_STATUS="$dir/fake" sh "$hook"); rc=$?
  sleep 0.3
  got=0; [ -e "$dir/opened" ] && got=1
  blk=0; [ "$(printf '%s' "$out" | jq -r '.decision // empty' 2>/dev/null)" = block ] && blk=1
  [ "$rc" = 0 ] && [ "$got" = "$1" ] && [ "$blk" = "$2" ] || { echo "FAIL: '$3' opened=$got blocked=$blk rc=$rc"; fail=1; }
}
# bare command: window opens AND the prompt is blocked (0 tokens)
try 1 1 "/repo"; try 1 1 "  /repo  "; try 1 1 "repo check"; try 1 1 "please check the repos"; try 1 1 "repos check now"
# mentioned inside a longer prompt: window opens, prompt goes through
try 1 0 "/repo and then fix the tab bug"; try 1 0 "do a repo check, then bump the version"
try 0 0 "fix the bug in repository.py"; try 0 0 "hello"; try 0 0 "see /repository"
[ "$fail" = 0 ] && echo "hook tests ok"; exit "$fail"
