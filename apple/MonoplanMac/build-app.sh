#!/usr/bin/env bash
#
# Build Monoplan.app, the menubar app, from Monoplan.xcodeproj and leave
# a copy at a stable path:
#
#   apple/MonoplanMac/.build/Monoplan.app     (gitignored with .build/)
#
# The project does the bundling (Info.plist, the icon from
# apple/icon.icon, the String Catalog, the sandbox entitlements,
# signing). This script adds what Xcode does not know: the version from
# Cargo.toml, the commit for the Settings Build row, the XCFramework
# when it is missing, and the strings check.
#
# Signing. By default the build is signed for development by the
# project's team with a provisioning profile Xcode manages
# (-allowProvisioningUpdates and -allowProvisioningDeviceRegistration:
# the first build registers the bundle id
# and this Mac with the team, and needs that team's account signed in
# under Xcode > Settings > Accounts). That is the build that keeps the
# profile's keys in the keychain (spec/cli.md "Secret stores").
#
#   MONOPLAN_SIGN=adhoc   "Sign to Run Locally": no team, no account, no
#                         profile. Runs on this Mac, sandboxed, with the
#                         keys in secrets.toml; cannot open a profile a
#                         team-signed build moved into the keychain, so
#                         give it MONOPLAN_APP_DATA_DIR=dev.
#   DEVELOPMENT_TEAM=<id> another team's id, for a checkout that is not
#                         the project's own.
#
# Usage: apple/MonoplanMac/build-app.sh [-c debug|release]  (default debug)
# Run from anywhere: `bun run mac` builds this and launches the binary
# inside the bundle, so stderr stays attached.

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
PKG="$ROOT/apple/MonoplanMac"

CONFIG=debug
while [ $# -gt 0 ]; do
  case "$1" in
    -c|--configuration) CONFIG="$2"; shift 2 ;;
    *) echo "usage: $0 [-c debug|release]" >&2; exit 2 ;;
  esac
done
case "$CONFIG" in
  debug) XCCONFIG=Debug ;;
  release) XCCONFIG=Release ;;
  *) echo "usage: $0 [-c debug|release]" >&2; exit 2 ;;
esac

# xcodebuild ships with full Xcode, not the Command Line Tools.
if ! xcrun --find actool >/dev/null 2>&1; then
  if [ -d /Applications/Xcode.app ]; then
    export DEVELOPER_DIR=/Applications/Xcode.app/Contents/Developer
  else
    echo "error: building the app needs full Xcode (found only Command Line Tools)." >&2
    exit 1
  fi
fi

VERSION="$(sed -n 's/^version = "\(.*\)"/\1/p' "$ROOT/Cargo.toml" | head -1)"
# CFBundleVersion must be dot-separated integers; the pre-release tag
# ("0.0.1-alpha.15") stays in CFBundleShortVersionString.
BUILD_NUMBER="$(printf '%s' "$VERSION" | sed -E 's/-alpha\.([0-9]+)$/.\1/; s/-.*$//')"
# The commit the bundle was built from, for the Settings Build row:
# the version string does not move between dev builds, the commit does.
# Same width as the server's build_info.rs; "+" marks a dirty tree.
GIT_COMMIT="$(git -C "$ROOT" rev-parse --short=12 HEAD 2>/dev/null || true)"
if [ -n "$GIT_COMMIT" ] && [ -n "$(git -C "$ROOT" status --porcelain 2>/dev/null)" ]; then
  GIT_COMMIT="$GIT_COMMIT+"
fi

# The app links MonoplanCore (the Rust engine loop) through a prebuilt
# XCFramework. Build it when missing; after core changes, refresh it
# yourself with `bun run build:apple` (it is release-only and slow).
if [ ! -d "$ROOT/apple/MonoplanCore/MonoplanCoreFFI.xcframework" ]; then
  echo ">> xcframework missing; running build-xcframework.sh"
  "$ROOT/apple/build-xcframework.sh"
fi

# The String Catalog is the one source of UI text in both languages
# (apple/README.md "Localisation"). check-strings.ts refuses a source
# string that is missing from it or has no Spanish, before anything
# slow runs. Not a build phase: Xcode's script sandbox and PATH do not
# suit bun, so a build started from Xcode skips it.
echo ">> check-strings"
bun "$PKG/check-strings.ts"

SIGNING=()
case "${MONOPLAN_SIGN:-team}" in
  adhoc) SIGNING=(CODE_SIGN_STYLE=Manual CODE_SIGN_IDENTITY=- DEVELOPMENT_TEAM= CODE_SIGN_ENTITLEMENTS=Monoplan-adhoc.entitlements) ;;
  team)
    # The second flag lets the first build register this Mac with the
    # team; without it xcodebuild stops at "device isn't registered".
    SIGNING=(-allowProvisioningUpdates -allowProvisioningDeviceRegistration)
    if [ -n "${DEVELOPMENT_TEAM:-}" ]; then SIGNING+=("DEVELOPMENT_TEAM=$DEVELOPMENT_TEAM"); fi
    ;;
  *) echo "error: MONOPLAN_SIGN is 'team' (default) or 'adhoc'" >&2; exit 2 ;;
esac

DERIVED="$PKG/.build/xcode"
LOG="$DERIVED/build.log"
mkdir -p "$DERIVED"
echo ">> xcodebuild ($XCCONFIG, ${MONOPLAN_SIGN:-team} signing)"
# xcodebuild narrates every step; keep the log and show it on failure.
if ! xcodebuild \
  -project "$PKG/Monoplan.xcodeproj" -scheme Monoplan -configuration "$XCCONFIG" \
  -derivedDataPath "$DERIVED" \
  "MARKETING_VERSION=$VERSION" "CURRENT_PROJECT_VERSION=$BUILD_NUMBER" \
  "MONOPLAN_GIT_COMMIT=$GIT_COMMIT" \
  "${SIGNING[@]}" build >"$LOG" 2>&1; then
  grep -E "error:|BUILD FAILED" "$LOG" >&2 || tail -40 "$LOG" >&2
  echo "error: xcodebuild failed; full log in $LOG" >&2
  if [ "${MONOPLAN_SIGN:-team}" = team ]; then
    echo "       if this is a signing error, MONOPLAN_SIGN=adhoc builds without a team (see the header)" >&2
  fi
  exit 1
fi

# The stable path the bun scripts and install-app.sh use. ditto keeps
# the signature, resource forks and permissions.
APP="$PKG/.build/Monoplan.app"
rm -rf "$APP"
ditto "$DERIVED/Build/Products/$XCCONFIG/Monoplan.app" "$APP"

# A team-signed build is only worth the name if it came out
# provisioned: the keychain needs the application identifier a profile
# carries, and Xcode signs a Mac app without one whenever nothing in
# its entitlements demands it.
if [ "${MONOPLAN_SIGN:-team}" = team ] && [ ! -f "$APP/Contents/embedded.provisionprofile" ]; then
  echo "warning: signed without a provisioning profile; this build will keep its keys in secrets.toml" >&2
fi

echo ">> $APP"
