#!/usr/bin/env bash
#
# Build Monoplan.app (release) and install it into /Applications.
#
# `bun run mac` and `bun run mac:app` run the bundle out of
# apple/MonoplanMac/.build/, which is fine for dev but means Login Items,
# the "Command line" symlink and Spotlight all point into a checkout
# that the next `rm -rf .build` or branch switch invalidates. This puts
# a release build at a stable path instead.
#
# Steps: build-app.sh -c release, quit a running Monoplan (it hosts the
# sync loop in-process, so it must exit cleanly rather than be killed),
# replace the bundle atomically, then launch it.
#
# Usage: bun run mac:install [-d <dir>] [--no-open]
#   -d, --dest <dir>   install directory (default /Applications;
#                      ~/Applications needs no admin rights)
#   --no-open          install without launching

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
PKG="$ROOT/apple/MonoplanMac"
SRC="$PKG/.build/Monoplan.app"
BUNDLE_ID="app.monoplan.mac"

DEST_DIR=/Applications
OPEN=1
while [ $# -gt 0 ]; do
  case "$1" in
    -d|--dest) DEST_DIR="$2"; shift 2 ;;
    --no-open) OPEN=0; shift ;;
    *) echo "usage: $0 [-d <dir>] [--no-open]" >&2; exit 2 ;;
  esac
done
DEST="$DEST_DIR/Monoplan.app"

if [ ! -d "$DEST_DIR" ] || [ ! -w "$DEST_DIR" ]; then
  echo "error: $DEST_DIR is not a writable directory; try -d ~/Applications" >&2
  exit 1
fi

"$PKG/build-app.sh" -c release

# Quit the running app, if any, and wait for it to exit. A plain kill
# would drop the in-process host mid-write; the AppleScript quit goes
# through applicationShouldTerminate like a Cmd-Q.
if pgrep -xq MonoplanMac; then
  echo ">> quitting running Monoplan"
  osascript -e "tell application id \"$BUNDLE_ID\" to quit" >/dev/null 2>&1 || true
  for _ in $(seq 1 30); do
    pgrep -xq MonoplanMac || break
    sleep 0.5
  done
  if pgrep -xq MonoplanMac; then
    echo "error: Monoplan did not quit within 15s; quit it and rerun" >&2
    exit 1
  fi
fi

# ditto preserves the ad-hoc signature, resource forks and permissions;
# stage next to the destination and rename so a failed copy never
# leaves a half-written bundle at the install path.
STAGE="$DEST_DIR/.Monoplan.app.installing"
rm -rf "$STAGE"
echo ">> ditto $SRC -> $DEST"
ditto "$SRC" "$STAGE"
rm -rf "$DEST"
mv "$STAGE" "$DEST"

# The Settings "Command line" row symlinks /usr/local/bin/monoplan to
# the bundled binary. Say where it points so a link into a dev
# checkout is noticed; the app's Install button re-points it.
LINK=/usr/local/bin/monoplan
if [ -L "$LINK" ]; then
  TARGET="$(readlink "$LINK")"
  case "$TARGET" in
    "$DEST"/*) echo ">> $LINK -> installed bundle" ;;
    *) echo ">> note: $LINK -> $TARGET (not the installed bundle; use Settings > Command line > Install)" ;;
  esac
fi

echo ">> $DEST"
if [ "$OPEN" = 1 ]; then
  open "$DEST"
fi
