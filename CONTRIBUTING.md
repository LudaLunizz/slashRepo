# Contributing

Issues and pull requests are welcome. This file is the setup and the exact
commands; the [README](README.md) says what slashRepo does.

## Setup

| Path | What |
|---|---|
| `src/main.rs` | `repo-status`: the command line (window, `--text`, list flags, `hook`, `install`) |
| `src/config.rs` | the tabs file: reading it and every add / remove / move / sort |
| `src/forge.rs` | GitHub (through `gh`), GitLab and Codeberg calls; tokens only to their own host |
| `src/report.rs` | a repo's card: status line, PR and issue lines, to-dos, the coloured dot |
| `src/gui.rs` | the GTK 4 window (Linux) |
| `src/gui_win.rs` | the same window on Windows (egui + wgpu: one self-contained exe) |
| `src/win.rs` | the few Windows console calls (the exe is a windowed program) |
| `build.rs`, `data/slashrepo.ico` | the icon inside `repo-status.exe` (when built on Windows) |
| `src/hook.rs` | the Claude Code `UserPromptSubmit` hook: which prompts open the window |
| `src/install.rs` | `repo-status install`: copies itself and sets up `/repo` per agent |
| `commands/repo.md` | the `/repo` command for Claude Code |
| `commands/opencode/`, `commands/gemini/` | `/repo` for opencode and Gemini CLI |
| `systemd/` | the trigger unit for agents in a sandbox (`install.sh --trigger FILE`) |
| `skills/repo/SKILL.md` | the `repo` skill for Codex and Antigravity |
| `install.sh` | bootstrap: builds (or downloads and checks) the binary, then runs `repo-status install` |
| `install.ps1` | the same for Windows |
| `tests/cli.rs` | the built binary in a throwaway HOME: list flags, the hook, the installer |
| `.github/workflows/release.yml` | attaches the Linux x86_64 binary, the Windows x86_64 exe and one `SHA256SUMS` to each published release |

Unit tests sit next to the code (`#[cfg(test)]`). Needs Rust (stable, 1.92 or newer), `gh` (logged
in) for GitHub repos, and on Linux the GTK 4 dev files (`libgtk-4-dev`). On Windows nothing more
(the MSVC build tools that rustup asks for). From Linux, `cargo clippy --all-targets --target
x86_64-pc-windows-gnu` checks the Windows code without a Windows machine (no linking).

## Before you open a pull request

These are what CI runs (`.github/workflows/ci.yml`):

```sh
cargo fmt --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked
sh -n install.sh
```

CI opens the Windows window on its runner and keeps a picture of it (the `repo-status-windows`
artifact, with the exe); it cannot open the Linux one. Say in the pull request whether you tried
`repo-status` on a real desktop. Without one, `SLASHREPO_SNAPSHOT=out.png repo-status`
saves the window to a PNG once every repo is checked and quits (on a broadway
display a browser must be connected to it, or nothing is drawn).

## Conventions

- **The installer is safe to run twice** and backs up `settings.json` first.
- **The program is read-only** towards the forges: it must not change anything on GitHub, GitLab or Codeberg.
- **No `unsafe`** (`unsafe_code = "forbid"`), and release builds keep overflow checks on.
- **GitHub Actions are pinned to a full commit SHA** with the version in a
  comment; Dependabot proposes updates.

## Issues and pull requests

Open an issue first for anything beyond a typo, saying what you want and why.
Work on a branch named `<issue number>-short-name`, and start the pull request
description with `Closes #<number>`. Wait for CI to be green; the maintainer
merges with a rebase (linear history).
