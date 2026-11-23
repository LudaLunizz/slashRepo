# slashRepo

Type `/repo` (or say "repo check") in Claude Code and a **Repo Status** window opens: for each of your
GitHub repos the latest release, CI on the default branch, open PRs and issues, Dependabot alerts,
commits since the last release, and a to-do list. A green/orange/red dot says if anything needs you.

It is read-only and costs no tokens: the window is a Python GTK4 script that talks to GitHub through `gh`.
A Claude Code `UserPromptSubmit` hook opens it, and Claude only answers "Repo Status window opened."

## Install

Needs `gh` (logged in), `jq`, `python3` with PyGObject + GTK 4 (Linux).

```sh
git clone https://github.com/LudaLunizz/slashRepo
cd slashRepo && ./install.sh
```

This copies `repo-status` and `repo-status-hook` to `~/.local/bin`, `repo.md` to `~/.claude/commands`,
adds a desktop entry, and adds the hook to `~/.claude/settings.json` (backup: `settings.json.bak-slashrepo`).
Running it again changes nothing.

## Choose the repos

By default: your own non-fork, non-archived repos (`gh repo list`, newest 20). To pick them yourself, put
one `owner/name` per line in `~/.config/slashrepo/repos` (`#` starts a comment).

## Use

- `/repo` or "repo check" / "check the repos" in Claude Code opens the window.
- `repo-status` opens it from a terminal or the app menu; `repo-status --text` prints the same report.

## Limits

Linux + GTK4 only. The hook matches the prompt text, so a prompt that mentions "repo check" in passing also opens it.

MIT licence.
