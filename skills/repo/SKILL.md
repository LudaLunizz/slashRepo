---
name: repo
description: Open the Repo Status window (slashRepo) when the user types /repo or $repo or asks for a "repo check". Not for questions about one repository's code.
---
Run exactly this one shell command and do not read any output or report:

    setsid -f ~/.local/bin/repo-status >/dev/null 2>&1 </dev/null

It opens a desktop window that checks GitHub, GitLab and Codeberg by itself, so it needs the desktop and
network access. If your tool runs commands in a sandbox without network, ask to run this one outside it.

Then reply with one short line: "Repo Status window opened." Do not summarise repos and do not run
`repo-status --text`: the point is that the report costs no tokens.
