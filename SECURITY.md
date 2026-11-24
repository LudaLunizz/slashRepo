# Security policy

## Reporting a vulnerability

Please report it **privately**, not in a public issue.

Go to the **Security** tab of this repository, choose **Report a vulnerability**, and fill in the form. Only you and the maintainer can see the report.

Please include:

- what happens and why you think it is a security problem,
- the steps to reproduce it (the prompt, the file or the command that triggers it),
- the slashRepo version (the commit or release) and your distribution.

You will get an acknowledgement as soon as I see the report. For a verified problem I aim to have a fix released within about 90 days, and I will tell you if it will take longer. If you want to be credited in the release notes, say so.

## Supported versions

Only the latest release (or the latest `main`, while there is no release) gets security fixes.

## What is in scope

- `install.sh` and `repo-status install`: they download a release binary (checked against `SHA256SUMS`) when Rust is missing, edit `~/.claude/settings.json` and copy files into `~/.local/bin`, `~/.claude` and the other agents' folders;
- `repo-status uninstall`: it edits `~/.claude/settings.json` (removing our hook entry only) and deletes files, so anything that makes it remove or change a file that is not ours is in scope;
- `repo-status hook`: it runs on every prompt you send in Claude Code and reads the prompt text, so anything that lets a prompt run a command is in scope;
- `repo-status`: it runs `gh` with your login, calls GitLab and Codeberg with your tokens (each only to its own host) and shows what they return (titles of issues and pull requests), so markup or text from a forge that is executed or misleads the window, or a token sent to the wrong host, is in scope.

## What is not a vulnerability by itself

- It uses your own `gh` login, read-only; that it can see everything `gh` can see is how it works.
- A prompt that mentions "repo check" in passing opens the window; the README names this as a limit.

## Disclosure

Please give me the time above before you publish details. When a fix is released I will publish a GitHub security advisory that names the affected versions.
