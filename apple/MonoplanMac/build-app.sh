#!/usr/bin/env bash
#
# Build Monoplan.app, the menubar app bundle, from the SwiftPM package.
# This is the only way the app gets its icon: AppKit reads an app icon
# from the bundle's Info.plist and Assets.car, so a bare `swift run`
# binary shows the generic executable icon in Finder, Login Items,
# Force Quit and so on.
#
#   apple/MonoplanMac/.build/Monoplan.app     (gitignored with .build/)
#
# The icon is the Icon Composer document `apple/icon.icon`. actool
# compiles it straight from that file (no asset catalog needed) into
# Assets.car (the layered icon macOS 26 renders) plus icon.icns (the
# flat fallback for macOS 14 and 15), and emits the two Info.plist keys
# that name them.
#
# Usage: apple/MonoplanMac/build-app.sh [-c debug|release]  (default debug)
# Run from anywhere: `bun run tray` builds this and launches the binary
# inside the bundle, so dev runs carry the icon too and stderr stays
# attached.

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
PKG="$ROOT/apple/MonoplanMac"
ICON="$ROOT/apple/icon.icon"
STRINGS="$PKG/Localizable.xcstrings"

CONFIG=debug
while [ $# -gt 0 ]; do
  case "$1" in
    -c|--configuration) CONFIG="$2"; shift 2 ;;
    *) echo "usage: $0 [-c debug|release]" >&2; exit 2 ;;
  esac
done

# actool and xcstringstool ship with full Xcode, not the Command Line Tools.
if ! xcrun --find actool >/dev/null 2>&1; then
  if [ -d /Applications/Xcode.app ]; then
    export DEVELOPER_DIR=/Applications/Xcode.app/Contents/Developer
  else
    echo "error: actool needs full Xcode (found only Command Line Tools)." >&2
    exit 1
  fi
fi

VERSION="$(sed -n 's/^version = "\(.*\)"/\1/p' "$ROOT/Cargo.toml" | head -1)"
# CFBundleVersion must be dot-separated integers; the pre-release tag
# ("0.0.1-alpha.15") stays in CFBundleShortVersionString.
BUILD_NUMBER="$(printf '%s' "$VERSION" | sed -E 's/-alpha\.([0-9]+)$/.\1/; s/-.*$//')"

# The app bundles the `monoplan` binary (spec/daemon-plan.md) so the
# Settings "Command line" row can symlink it onto PATH: the terminal
# then talks to the app's in-process host at the app's own version.
# The app never starts it; the host loop is linked in via MonoplanCore.
# Debug and release follow the app's configuration; sync on a
# real-sized doc is slow in debug, so install the release build.
echo ">> cargo build -p monoplan ($CONFIG)"
if [ "$CONFIG" = release ]; then
  cargo build -p monoplan --release
else
  cargo build -p monoplan
fi
CLI="$ROOT/target/$CONFIG/monoplan"

# The app links MonoplanCore (the Rust host loop) through a prebuilt
# XCFramework. Build it when missing; after core changes, refresh it
# yourself with `bun run build:apple` (it is release-only and slow).
if [ ! -d "$ROOT/apple/MonoplanCore/MonoplanCoreFFI.xcframework" ]; then
  echo ">> xcframework missing; running build-xcframework.sh"
  "$ROOT/apple/build-xcframework.sh"
fi

# The String Catalog is the one source of UI text in both languages
# (apple/README.md "Localisation"). check-strings.ts refuses a source
# string that is missing from it or has no Spanish, before anything
# slow runs.
echo ">> check-strings"
bun "$PKG/check-strings.ts"

echo ">> swift build ($CONFIG)"
swift build --package-path "$PKG" -c "$CONFIG" --product MonoplanMac
BIN="$(swift build --package-path "$PKG" -c "$CONFIG" --show-bin-path)/MonoplanMac"

APP="$PKG/.build/Monoplan.app"
CONTENTS="$APP/Contents"
rm -rf "$APP"
mkdir -p "$CONTENTS/MacOS" "$CONTENTS/Resources"
cp "$BIN" "$CONTENTS/MacOS/MonoplanMac"
cp "$CLI" "$CONTENTS/MacOS/monoplan"

# SwiftPM cannot put the catalog in Bundle.main, so compile it here:
# xcstringstool emits one <lang>.lproj/Localizable.strings per
# translated language. The source language gets an empty en.lproj so
# the bundle lists two localizations, which is what makes macOS offer
# the app in System Settings › Language & Region › Applications.
echo ">> xcstringstool $STRINGS"
xcrun xcstringstool compile "$STRINGS" --output-directory "$CONTENTS/Resources"
mkdir -p "$CONTENTS/Resources/en.lproj"

echo ">> actool $ICON"
PARTIAL="$(mktemp -t monoplan-icon-plist)"
# actool reports on stdout even when it fails; keep it quiet on success
# and show everything on failure.
if ! ACTOOL_OUT="$(xcrun actool "$ICON" \
  --compile "$CONTENTS/Resources" \
  --platform macosx \
  --minimum-deployment-target 14.0 \
  --app-icon icon \
  --output-partial-info-plist "$PARTIAL" \
  --output-format human-readable-text \
  --notices --warnings --errors 2>&1)"; then
  echo "$ACTOOL_OUT" >&2
  exit 1
fi

cat > "$CONTENTS/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleDevelopmentRegion</key><string>en</string>
  <key>CFBundleExecutable</key><string>MonoplanMac</string>
  <key>CFBundleIdentifier</key><string>app.monoplan.mac</string>
  <key>CFBundleInfoDictionaryVersion</key><string>6.0</string>
  <key>CFBundleName</key><string>Monoplan</string>
  <key>CFBundleDisplayName</key><string>Monoplan</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleShortVersionString</key><string>$VERSION</string>
  <key>CFBundleVersion</key><string>$BUILD_NUMBER</string>
  <key>LSMinimumSystemVersion</key><string>14.0</string>
  <key>LSUIElement</key><true/>
  <key>NSHighResolutionCapable</key><true/>
  <key>NSHumanReadableCopyright</key><string>Monoplan</string>
</dict>
</plist>
PLIST
# Merge CFBundleIconFile / CFBundleIconName from actool rather than
# hard-coding them, so a renamed .icon only changes the --app-icon above.
/usr/libexec/PlistBuddy -c "Merge $PARTIAL" "$CONTENTS/Info.plist" >/dev/null
rm -f "$PARTIAL"
printf 'APPL????' > "$CONTENTS/PkgInfo"

# Ad-hoc signatures: unsigned bundles are refused by Login Items and get
# a fresh TCC identity every build. The helper is signed on its own;
# bundle signing does not descend into Contents/MacOS siblings.
codesign --force --sign - "$CONTENTS/MacOS/monoplan" >/dev/null 2>&1
codesign --force --sign - "$APP" >/dev/null 2>&1

echo ">> $APP"
