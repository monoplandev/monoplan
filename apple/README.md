# Monoplan on Apple

The Rust↔Swift FFI boundary for the Apple clients. This is layers 1 & 2 of
[`spec/swift-ffi-plan.md`](../spec/swift-ffi-plan.md): an **offline** capture/read
surface only — no sync, auth, HTTP, UI, or Keychain yet. The `SyncEngine` is not
exposed; that plumbing is a follow-up once the boundary is proven.

## Layering

```
monoplan-ffi (Rust staticlib)          core/ffi — uniffi object over Doc + sqlite storage + DEK
   → MonoplanCoreFFI.xcframework        static libs (macOS/iOS/iOS-sim) + C headers
      → MonoplanCore (SwiftPM)          generated Swift bindings + a thin facade
         → future Xcode app           consumes MonoplanCore as a local package
```

The one exported object, `MonoplanStore`, mirrors the CLI's `boot_doc`: it opens
`<dir>/monoplan.sqlite`, boots the `Doc` from persisted encrypted ops, and after
every mutation captures the fresh Loro delta into an encrypted oplog row so a
later reopen replays it. The DEK crosses the boundary as raw bytes — key storage
(Keychain) is the caller's problem.

## Prerequisites

- **Rust** with the Apple targets (the build script adds them for you):
  `aarch64-apple-darwin`, `aarch64-apple-ios`, `aarch64-apple-ios-sim`.
- **Full Xcode** (not just the Command Line Tools). `-create-xcframework` and
  `swift test`'s `XCTest` both need it. If `xcode-select -p` points at
  `CommandLineTools`, either `sudo xcode-select -s /Applications/Xcode.app` once,
  or prefix commands with `DEVELOPER_DIR=/Applications/Xcode.app/Contents/Developer`.
  (`build-xcframework.sh` already falls back to `/Applications/Xcode.app` on its own.)

## The two commands

```sh
# 1. Build the XCFramework + generated Swift bindings from the Rust crate.
bun run build:apple            # == apple/build-xcframework.sh

# 2. Run the SwiftPM smoke test (open → mutate → read → reopen → assert).
swift test --package-path apple/MonoplanCore
# on a CLT-default machine:
# DEVELOPER_DIR=/Applications/Xcode.app/Contents/Developer swift test --package-path apple/MonoplanCore
```

`build:apple` is the **single** way to produce the two build outputs, both
gitignored:

- `apple/MonoplanCore/MonoplanCoreFFI.xcframework` — the static libs + headers.
- `apple/MonoplanCore/Sources/MonoplanCore/Generated/` — the uniffi Swift bindings.

Run it before `swift build` / `swift test` on a fresh checkout — the package
won't compile without those outputs present.

## MonoplanMac — the menubar app

`apple/MonoplanMac/` is a SwiftPM package: the macOS status-item app.
Per `spec/daemon-plan.md` it is a *client* of `monoplan daemon`, never a
host, and the tray and the future window app are one process. It holds
no doc and does not depend on `MonoplanCore`: it connects to the
daemon's Unix socket, sends `Watch`, and re-fetches
`agenda --days 0 --json` whenever the daemon reports a change, once a
minute, and when the menu opens. The menu shows Today's rows (time,
title, "in 2h 15m" / "now" / "due today" / "overdue"; a row opens the
item in the web client), a Starred section with the first three rows of
the Focus lens (`starred --json`) each with its list's name muted on the
right (`lists ls --json --all`), an offline notice when the daemon has no
server, "Open Monoplan" (alpha.monoplan.app) and "Quit Monoplan" (the
daemon keeps running).

Two targets: `MonoplanMacKit` (MessagePack codec for the subset
`rmp_serde::to_vec_named` emits, the socket client, the agenda model;
unit-tested) and the `MonoplanMac` executable (AppKit). The wire shape
it hand-encodes is pinned on the Rust side by
`wire_shape_is_named_maps_and_bare_unit_variants` in `cli/src/ipc.rs`.

```sh
bun run tray                                          # == swift run --package-path apple/MonoplanMac
swift test --package-path apple/MonoplanMac           # codec / model tests, no daemon needed
MONOPLAN_WEB_URL=http://localhost:5176 bun run tray   # point links at the dev stack
MONOPLAN_DATA_DIR=local/cli bun run tray              # a different profile, same rule as the CLI
MONOPLAN_TRAY_DEBUG=1 bun run tray                    # log state and rows to stderr
```

If no daemon holds the profile, the tray starts one like the CLI does
(`monoplan daemon --detach`), using `MONOPLAN_BIN`, then
`~/.cargo/bin`, `/opt/homebrew/bin`, `/usr/local/bin`, then
`./target/debug/monoplan` when run from the repo. It cannot judge build
staleness (it is not built from the daemon's sources); the CLI still
restarts a stale daemon on its next command.

Next: an .app bundle with login-item registration; then the window.
