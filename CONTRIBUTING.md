# Contributing

Issues and pull requests are welcome. This file is the setup and the exact
commands; the [README](README.md) says what slashRepo does.

## Setup

| Path | What |
|---|---|
| `bin/repo-status` | the GTK4 window and `--text` report (Python, uses `gh`) |
| `bin/repo-status-hook` | the Claude Code `UserPromptSubmit` hook that opens it |
| `commands/repo.md` | the `/repo` command |
| `install.sh` | copies all of it into place and adds the hook to `settings.json` |
| `tests/hook_test.sh` | which prompts open the window |

Needs `gh` (logged in), `jq`, and `python3` with PyGObject + GTK 4 to see the window.

## Before you open a pull request

These are what CI runs (`.github/workflows/ci.yml`):

```sh
python3 -I -c "import ast; ast.parse(open('bin/repo-status').read())"
sh -n install.sh && sh -n bin/repo-status-hook
tests/hook_test.sh
```

CI cannot open the window; say in the pull request whether you tried
`repo-status` on a real desktop.

## Conventions

- **The installer is safe to run twice** and backs up `settings.json` first.
- **The script is read-only**: it must not change anything on GitHub.
- **GitHub Actions are pinned to a full commit SHA** with the version in a
  comment; Dependabot proposes updates.

## Issues and pull requests

Open an issue first for anything beyond a typo, saying what you want and why.
Work on a branch named `<issue number>-short-name`, and start the pull request
description with `Closes #<number>`. Wait for CI to be green; the maintainer
merges with a rebase (linear history).
