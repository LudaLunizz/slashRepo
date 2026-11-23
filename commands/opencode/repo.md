---
description: Open the Repo Status window (releases, CI, PRs, issues)
---
!`p="${REPO_STATUS:-$HOME/.local/bin/repo-status}"; if [ -x "$p" ]; then setsid -f "$p" >/dev/null 2>&1 </dev/null; echo "Repo Status window opened."; else echo "repo-status is not reachable from here (not installed, or a sandbox hides it)."; fi`
The line above comes from this command. Reply with exactly that one line and nothing else. Do not run anything.
