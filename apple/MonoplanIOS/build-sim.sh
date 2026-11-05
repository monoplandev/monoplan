#!/usr/bin/env bash
#
# Build the iOS app for the simulator and, with --run, install and
# launch it there. `xcodebuild` against the hand-written project in
# this directory; DerivedData lands in `.build/` (gitignored) so a
# clean checkout stays clean.
#
#   apple/MonoplanIOS/build-sim.sh [--run] [-d "iPhone 16"] [-c debug|release]
#
# Needs `bun run build:apple` first (MonoplanCore wraps a prebuilt
# XCFramework) and full Xcode. Running on a device needs a signing
# team: open the project in Xcode and set one, or pass
# DEVELOPMENT_TEAM=… in the environment.

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
PROJ="$ROOT/apple/MonoplanIOS/Monoplan.xcodeproj"
DERIVED="$ROOT/apple/MonoplanIOS/.build"
BUNDLE_ID=app.monoplan.ios

CONFIG=Debug
DEVICE="iPhone 16"
RUN=0
while [ $# -gt 0 ]; do
  case "$1" in
    --run) RUN=1; shift ;;
    -d|--device) DEVICE="$2"; shift 2 ;;
    -c|--configuration)
      case "$2" in
        debug) CONFIG=Debug ;;
        release) CONFIG=Release ;;
        *) echo "usage: $0 [--run] [-d name] [-c debug|release]" >&2; exit 2 ;;
      esac
      shift 2 ;;
    *) echo "usage: $0 [--run] [-d name] [-c debug|release]" >&2; exit 2 ;;
  esac
done

if ! xcodebuild -version >/dev/null 2>&1; then
  if [ -d /Applications/Xcode.app ]; then
    export DEVELOPER_DIR=/Applications/Xcode.app/Contents/Developer
  else
    echo "error: needs full Xcode." >&2
    exit 1
  fi
fi

if [ ! -d "$ROOT/apple/MonoplanCore/MonoplanCoreFFI.xcframework" ]; then
  echo "error: no XCFramework; run \`bun run build:apple\` first." >&2
  exit 1
fi

# One simulator by name: several runtimes can carry the same device
# name, which `-destination name=` refuses as ambiguous, so pick the
# newest runtime's and address it by id.
UDID="$(xcrun simctl list devices available -j | python3 -c '
import json, sys
name = sys.argv[1]
best = None
for runtime, devs in json.load(sys.stdin)["devices"].items():
    for d in devs:
        if d["name"] == name and (best is None or runtime > best[0]):
            best = (runtime, d["udid"])
if best is None:
    sys.exit(1)
print(best[1])' "$DEVICE")" || { echo "error: no available simulator named \"$DEVICE\" (see \`xcrun simctl list devices available\`)." >&2; exit 1; }

echo ">> building ($CONFIG, $DEVICE, $UDID)"
xcodebuild \
  -project "$PROJ" \
  -scheme Monoplan \
  -configuration "$CONFIG" \
  -destination "platform=iOS Simulator,id=$UDID" \
  -derivedDataPath "$DERIVED" \
  ${DEVELOPMENT_TEAM:+DEVELOPMENT_TEAM="$DEVELOPMENT_TEAM"} \
  -quiet \
  build

APP="$DERIVED/Build/Products/$CONFIG-iphonesimulator/Monoplan.app"
echo "built: $APP"

if [ "$RUN" = 1 ]; then
  xcrun simctl boot "$UDID" 2>/dev/null || true
  # Simulator.app lives inside Xcode when it ships at all (Xcode 27
  # here has none, 2026-11-05). The device runs headless either way;
  # the window is for the human, and `xcrun simctl io <udid>
  # screenshot out.png` sees the app without one.
  open "${DEVELOPER_DIR:-$(xcode-select -p)}/Applications/Simulator.app" 2>/dev/null \
    || open -a Simulator 2>/dev/null || echo "note: could not open Simulator.app; the device is booted headless"
  xcrun simctl install "$UDID" "$APP"
  xcrun simctl launch "$UDID" "$BUNDLE_ID"
fi
