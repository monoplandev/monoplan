#!/usr/bin/env bash
#
# Build the MonoplanCoreFFI XCFramework + Swift bindings from the
# `monoplan-ffi` Rust crate. This is the single source of the two build
# outputs the SwiftPM package (`apple/MonoplanCore`) consumes:
#
#   apple/MonoplanCore/MonoplanCoreFFI.xcframework          (static libs + headers)
#   apple/MonoplanCore/Sources/MonoplanCore/Generated/*.swift  (uniffi bindings)
#
# Both are gitignored; this script is how you (re)produce them. Run from
# anywhere: `bun run build:apple` or `apple/build-xcframework.sh`.
#
# See spec/swift-ffi-plan.md (Milestone 2) and apple/README.md.

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

CRATE=monoplan-ffi
STATICLIB=libmonoplan_ffi.a
DYLIB=libmonoplan_ffi.dylib
NAMESPACE=monoplan_ffi            # uniffi namespace == crate lib name

PKG_DIR="$ROOT/apple/MonoplanCore"
XCFRAMEWORK="$PKG_DIR/MonoplanCoreFFI.xcframework"
GEN_SWIFT_DIR="$PKG_DIR/Sources/MonoplanCore/Generated"

# macOS host first: its release cdylib is what uniffi library-mode bindgen
# reads metadata from. iOS device + simulator aren't consumed yet, but
# proving they compile now is cheap insurance (plan Milestone 2).
HOST_TARGET=aarch64-apple-darwin
TARGETS=("$HOST_TARGET" aarch64-apple-ios aarch64-apple-ios-sim)

# `xcodebuild -create-xcframework` needs full Xcode, not just the Command
# Line Tools. Respect an already-selected Xcode; otherwise fall back to
# /Applications/Xcode.app without requiring `sudo xcode-select`.
if ! xcodebuild -version >/dev/null 2>&1; then
  if [ -d /Applications/Xcode.app ]; then
    export DEVELOPER_DIR=/Applications/Xcode.app/Contents/Developer
  else
    echo "error: -create-xcframework needs full Xcode (found only Command Line Tools)." >&2
    echo "       Install Xcode, or run: sudo xcode-select -s /Applications/Xcode.app" >&2
    exit 1
  fi
fi

echo ">> rustup targets"
for t in "${TARGETS[@]}"; do
  rustup target add "$t" >/dev/null
done

# Minimum OS versions, matching `apple/MonoplanCore/Package.swift`. rustc
# and the C toolchain (aws-lc-sys, reqwest's TLS backend since 0.13)
# both read these; without them rustc links the iOS slices against iOS
# 10 and the link fails on `___chkstk_darwin`, which aws-lc's objects
# need and older runtimes lack.
export MACOSX_DEPLOYMENT_TARGET="${MACOSX_DEPLOYMENT_TARGET:-14.0}"
export IPHONEOS_DEPLOYMENT_TARGET="${IPHONEOS_DEPLOYMENT_TARGET:-17.0}"

echo ">> building staticlib (release) for ${#TARGETS[@]} targets"
for t in "${TARGETS[@]}"; do
  echo "   - $t"
  cargo build -p "$CRATE" --release --target "$t"
done

# Generate the Swift bindings once, in library mode, from the host cdylib.
GEN_TMP="$(mktemp -d)"
HEADERS_TMP="$(mktemp -d)"
trap 'rm -rf "$GEN_TMP" "$HEADERS_TMP"' EXIT

echo ">> generating Swift bindings"
cargo run -q -p "$CRATE" --bin uniffi-bindgen -- \
  generate \
  --library "target/$HOST_TARGET/release/$DYLIB" \
  --language swift \
  --out-dir "$GEN_TMP"

# XCFramework header dirs want the modulemap named `module.modulemap`;
# uniffi emits `<namespace>FFI.modulemap`. The module *name* inside is
# unchanged, so the generated `.swift`'s `import ${NAMESPACE}FFI` still
# resolves. All three slices share the one generated header.
echo ">> assembling headers"
cp "$GEN_TMP/${NAMESPACE}FFI.h" "$HEADERS_TMP/"
cp "$GEN_TMP/${NAMESPACE}FFI.modulemap" "$HEADERS_TMP/module.modulemap"

echo ">> creating xcframework"
rm -rf "$XCFRAMEWORK"
xcargs=()
for t in "${TARGETS[@]}"; do
  xcargs+=(-library "target/$t/release/$STATICLIB" -headers "$HEADERS_TMP")
done
xcodebuild -create-xcframework "${xcargs[@]}" -output "$XCFRAMEWORK"

echo ">> installing generated bindings"
rm -rf "$GEN_SWIFT_DIR"
mkdir -p "$GEN_SWIFT_DIR"
cp "$GEN_TMP/${NAMESPACE}.swift" "$GEN_SWIFT_DIR/"

echo "done:"
echo "  $XCFRAMEWORK"
echo "  $GEN_SWIFT_DIR/${NAMESPACE}.swift"
