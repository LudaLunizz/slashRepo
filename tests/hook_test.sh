#!/bin/sh
# The hook opens the window for the right prompts and only those.
set -u
dir=$(mktemp -d); trap 'rm -rf "$dir"' EXIT
printf '#!/bin/sh\ntouch "%s/opened"\n' "$dir" > "$dir/fake"; chmod +x "$dir/fake"
hook=$(cd "$(dirname "$0")/.." && pwd)/bin/repo-status-hook
fail=0
try() { # expected(0/1) prompt
  rm -f "$dir/opened"
  printf '{"prompt":%s}' "$(printf '%s' "$2" | jq -Rs .)" | REPO_STATUS="$dir/fake" sh "$hook"; rc=$?
  sleep 0.3
  got=0; [ -e "$dir/opened" ] && got=1
  [ "$rc" = 0 ] && [ "$got" = "$1" ] || { echo "FAIL: '$2' opened=$got rc=$rc"; fail=1; }
}
try 1 "/repo"; try 1 "repo check"; try 1 "please check the repos"; try 1 "repos check now"
try 0 "fix the bug in repository.py"; try 0 "hello"; try 0 "see /repository"
[ "$fail" = 0 ] && echo "hook tests ok"; exit "$fail"
