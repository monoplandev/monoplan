# Monoplan on Apple

The Rust↔Swift FFI boundary for the Apple clients. What crosses it today
is the profile's lifecycle, not the doc: `login` / `logout` (the bodies of
the CLI commands) and `Host`, the daemon loop embedded in the app's
process (`spec/daemon-plan.md`, amended 2026-10-27). The macOS menubar app
reads the doc as a client of its own host's socket. In-process doc access
for the native clients (typed views, a mutation surface, a change callback
on `Host`, a host that boots without serving a socket) is the next layer
and is not built. The offline `MonoplanStore` prototype from
[`spec/swift-ffi-plan.md`](../spec/swift-ffi-plan.md) was removed on
2026-10-27; that plan is kept for the build pipeline it set up.

## Layering

```
monoplan-ffi (Rust staticlib)          core/ffi — uniffi over the CLI crate's daemon: Host, login, logout
   → MonoplanCoreFFI.xcframework        static libs (macOS/iOS/iOS-sim) + C headers
      → MonoplanCore (SwiftPM)          generated Swift bindings + a thin facade
         → MonoplanMac / future iOS app  consume MonoplanCore as a local package
```

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

# 2. Run the SwiftPM tests (the embedded host's start-up refusals, no server needed).
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

`apple/MonoplanMac/` is a SwiftPM package: the macOS status-item app, and
on macOS the normal host of the profile (`spec/daemon-plan.md`,
amendment 2026-10-27). One process: it runs the Rust host loop
in-process (`Host` from `MonoplanCore`, the same loop as `monoplan
daemon`) on a background thread, and its tray is a client of its own
socket, so the UI is exactly the client it was. It connects, sends
`Watch`, and re-fetches `agenda --days 0 --json` whenever the host
reports a change, once a minute, and when the menu opens. The menu shows
Today's rows (time, title, "in 2h 15m" / "now" / "due today" /
"overdue"; a row opens the item in the web client), a Starred section
with the first three rows of the Focus lens (`starred --json`) each with
its list's name muted on the right (`lists ls --json --all`), an offline
notice when there is no server, "Open Monoplan" (alpha.monoplan.app),
"Settings…" and "Quit Monoplan" (which stops the host: persist, fold,
release the profile).

Hosting rules (`Sources/MonoplanMac/HostController.swift`): the app
hosts whenever it runs and only ever talks to its own host. Whenever
the socket is down and the profile is signed in it starts the host,
which takes a headless `monoplan daemon` over (asks it to exit, then
claims the profile). It honours `Shutdown`, so `logout`, `login` and
`password` in the terminal stop it, hold the profile while they
rewrite it, and the app re-hosts once it is free. The one thing it
waits out rather than takes over is "busy": another copy of the app, a
host still starting, or a command holding the profile; the menu says
"profile in use" and the client's backoff retries. Doc commands from
the CLI work against the app's host; it is never restarted as stale,
its hello only has to match the app's package version, which the
bundled CLI symlinked onto `PATH` guarantees.

Three targets: `MonoplanMacKit` (MessagePack codec for the subset
`rmp_serde::to_vec_named` emits, the socket client, the agenda model;
unit-tested, no MonoplanCore dependency), the `MonoplanMac` executable
(AppKit, the host controller) and `MonoplanCore` (the prebuilt
XCFramework). The wire shape it hand-encodes is pinned on the Rust side
by `wire_shape_is_named_maps_and_bare_unit_variants` in `cli/src/ipc.rs`.

```sh
bun run build:apple                                   # once, and after core changes: the XCFramework the app links
bun run mac                                           # build Monoplan.app (debug) and run the binary inside it
bun run mac:app                                       # build Monoplan.app (release) and `open` it
swift test --package-path apple/MonoplanMac           # codec / model tests, no host needed
MONOPLAN_WEB_URL=http://localhost:5176 bun run mac    # point links at the dev stack
MONOPLAN_DATA_DIR=local/cli bun run mac               # a different profile, same rule as the CLI
MONOPLAN_MAC_DEBUG=1 bun run mac                      # log state and rows to stderr
```

The in-app host logs to `daemon.log` in the profile dir (`~/Library/
Application Support/monoplan/daemon.log` by default; `RUST_LOG`
overrides the `info` level): connect failures and drops land there
once per distinct reason, and the same reason shows on the Settings
Sync row and in `monoplan daemon status` ("offline; never synced
(connect failed: …)"). The app's own `NSLog` lines go to the unified
log: `log show --predicate 'process == "MonoplanMac"' --last 1h`.

`apple/MonoplanMac/build-app.sh` lays out `apple/MonoplanMac/.build/Monoplan.app`
(gitignored): the SwiftPM binary (which links MonoplanCore; the script
runs `build-xcframework.sh` when the XCFramework is missing), the
`monoplan` CLI beside it in `Contents/MacOS` (built by cargo in the same
configuration, for the terminal), an Info.plist (`app.monoplan.mac`,
`LSUIElement`, version from the workspace `Cargo.toml`), ad-hoc
signatures, and the app icon. The icon is the Icon Composer document
`apple/icon.icon`; `actool` compiles it straight from that file into
`Assets.car` (the layered icon macOS 26 renders) and `icon.icns` (the
flat fallback for macOS 14 and 15). Edit the icon in Icon Composer and
rebuild; nothing else references it. `bun run mac` runs the binary
*inside* the bundle rather than `swift run`, so dev runs show the icon
in Finder, Login Items and Force Quit while stderr stays on the
terminal. The menu bar glyph is separate: a template image drawn in
`main.swift`.

Settings has two panes. Account: server, status, sync, an "Open at
login" toggle (`SMAppService.mainApp`, which lists the app under System
Settings › Login Items; works only from an .app bundle), and Sign In /
Sign Out. Sign In is a sheet (email, password; server and device name
under an Advanced disclosure, the server defaulting to
`https://alpha.monoplan.app`) that runs `monoplan-ffi::login`, the
same body as `monoplan auth login`; the tray shows "Sign In…" while signed
out. Sign Out is confirmed and runs `monoplan-ffi::logout`, the same
as `monoplan auth logout`: it stops the in-app host and wipes the profile.
Advanced: the
Host row (running, or why not), the Build row (package version, then
the short commit `build-app.sh` baked into Info.plist, "+" if the tree
was dirty), a
"Command line" row whose Install button symlinks `/usr/local/bin/monoplan`
to the bundled binary (admin prompt if the directory is root's), and
the profile path with a Reveal button. The
launchd agent from `monoplan daemon install` is the headless
alternative for Macs without the app: the app takes its daemon over at
launch, and `install` refuses while the app hosts.

### Localisation

The app follows the system language, as a Mac app should: no language
picker. `apple/MonoplanMac/Localizable.xcstrings` is a String Catalog
with English keys and a Spanish translation for every UI string in both
targets, the menu's agenda notes included ("in 2h 15m", "overdue",
"Offline · last synced 3m ago"). English is the key itself, so the
catalog has no `en` entries. SwiftUI literals (`Text("Account")`,
`Button("Install")`) look the catalog up by themselves; everything else
that reaches a user (AppKit titles, model strings, error descriptions)
goes through `String(localized:)`, with `\(…)` interpolations as `%@` /
`%lld` in the key. A ternary of literals or a `String` variable does not
localize, so wrap those.

SwiftPM cannot put the catalog in `Bundle.main`, so `build-app.sh`
compiles it with `xcstringstool` into `Contents/Resources/es.lproj/
Localizable.strings` and adds an empty `en.lproj`; two localizations is
what makes System Settings › Language & Region › Applications offer the
app. A bare `swift run` has no catalog and shows the English keys. In
`swift test` the lookups miss too, so the Kit tests keep asserting the
English text.

`check-strings.ts` (run by `build-app.sh` and by `bun run check:strings`)
scans `Sources/` for the call sites above and fails when a string is
missing from the catalog, a catalog entry is no longer used, a Spanish
translation is missing, or its placeholders differ from the English.
Adding a string means adding the key and its Spanish to the catalog in
the same change; the build will not let it drift.

Next: the window.
