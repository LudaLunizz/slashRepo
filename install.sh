#!/bin/sh
# Install slashRepo: get the repo-status binary, then let it set itself up (~/.local/bin, app menu,
# /repo for every coding agent found: Claude Code, opencode, Codex, Antigravity, Gemini CLI).
# Safe to run again. --trigger FILE also installs the systemd unit for agents in a sandbox.
#
# With Rust (cargo) and the GTK 4 dev files (libgtk-4-dev) it builds from this checkout.
# Without cargo it downloads the prebuilt Linux x86_64 binary of this version from the GitHub
# release and checks it against the release's SHA256SUMS before running it.
set -eu
here=$(cd "$(dirname "$0")" && pwd)
if command -v cargo >/dev/null 2>&1; then
  cargo build --release --locked --manifest-path "$here/Cargo.toml"
  bin="$here/target/release/repo-status"
  [ -x "$bin" ] || bin="$(cargo metadata --format-version 1 --no-deps --manifest-path "$here/Cargo.toml" \
    | sed -n 's/.*"target_directory":"\([^"]*\)".*/\1/p')/release/repo-status"
else
  [ "$(uname -s)-$(uname -m)" = Linux-x86_64 ] || { echo "no prebuilt binary for $(uname -sm): install Rust (rustup.rs) and run again" >&2; exit 1; }
  command -v curl >/dev/null || { echo "missing: curl (or install Rust to build instead)" >&2; exit 1; }
  version=$(sed -n 's/^version = "\(.*\)"/\1/p' "$here/Cargo.toml" | head -n 1)
  url="https://github.com/LudaLunizz/slashRepo/releases/download/v$version"
  dir=$(mktemp -d); trap 'rm -rf "$dir"' EXIT
  curl -fsSL -o "$dir/repo-status-x86_64-linux" "$url/repo-status-x86_64-linux"
  curl -fsSL -o "$dir/SHA256SUMS" "$url/SHA256SUMS"
  (cd "$dir" && grep ' repo-status-x86_64-linux$' SHA256SUMS | sha256sum -c -) >/dev/null \
    || { echo "checksum mismatch: not installing" >&2; exit 1; }
  chmod 755 "$dir/repo-status-x86_64-linux"
  bin="$dir/repo-status-x86_64-linux"
fi
"$bin" install "$@"
