#!/bin/sh
# A picture of the real GTK window without putting it on your desktop: it is drawn on an
# off-screen display (gtk4-broadwayd) and saves itself.
# usage: snap-gtk.sh BINARY OUT.png [TAB] [WIDTHxHEIGHT]
#   BINARY: a repo-status binary; its path must not contain spaces
# needs: gtk4-broadwayd (package libgtk-4-bin), dbus-run-session
set -eu
BIN=$1 OUT=$2 TAB=${3:-0} SIZE=${4:-940x940}
D=$((40 + $$ % 50))
H=$(mktemp -d)
mkdir -p "$H/rt"
chmod 700 "$H/rt"
dbus-run-session -- sh -c '
  export HOME="$1" XDG_RUNTIME_DIR="$1/rt" GDK_BACKEND=broadway BROADWAY_DISPLAY=":$2"
  gtk4-broadwayd ":$2" >/dev/null 2>&1 & B=$!
  sleep 1
  SLASHREPO_SNAPSHOT="$4" SLASHREPO_SNAPSHOT_SCALE=2 SLASHREPO_SNAPSHOT_TAB="$5" SLASHREPO_SNAPSHOT_SIZE="$6" \
    timeout 120 "$3" --demo
  kill $B 2>/dev/null; wait 2>/dev/null' sh "$H" "$D" "$BIN" "$OUT" "$TAB" "$SIZE" 2>/dev/null
rm -rf "$H"
test -s "$OUT"
