# Monoplan on Apple

The Rust↔Swift FFI boundary for the Apple clients. What crosses it today
is the profile's lifecycle, not the doc: `login` / `logout` (the bodies of
the CLI commands) and `Engine`, the daemon loop embedded in the app's
process (`spec/daemon-plan.md`, amended 2026-10-27), and the app's
client path into it: `Engine.request` and `Engine.watch`, the socket's
request and watch payloads as function calls (phase 1 of
[`spec/mac-window-plan.md`](../spec/mac-window-plan.md), built
2026-10-29), and the engine side of the window's mirror: `Engine.subscribe`
(a plaintext snapshot, a peer id leased for the mirror, and every later
change as Loro update bytes through a `MirrorSink`) and `Engine.import`
(the mirror's ops applied as local), phase 2, built 2026-11-03, plus
`Engine.session` (the signed-in identity, server and open docs the
window boots from) and `Engine.device_token` (what the app puts on the
window's HTTP calls), phase 4. The tray never opens the socket. The
window is that plan's phases 3 and 4, both built 2026-11-04: a webview
running the web client (`js/web/mirror.html`, over `Replica.mirror`)
over a mirror doc that `Engine` seeds and converges through that
surface, with a peer id of its own. The page talks to the app through
a WebKit message handler named `monoplan` and the
`window.__monoplanHost` global; the exact requests, replies and base64
framing are in that spec's "Swift side" and in
`js/web/src/sync/mirrorHost.ts`, the Swift side in
`MonoplanMac/Sources/MonoplanMac/WebWindow.swift`.
The socket stays the CLI's; it never carries the mirror. iOS is native
(decided 2026-11-05, no webview): `apple/MonoplanIOS` below is its
first surface, a client of the same embedded engine over the same FFI,
so the FFI grows typed reads as that UI does. The offline
`MonoplanStore` prototype from
[`spec/swift-ffi-plan.md`](../spec/swift-ffi-plan.md) was removed on
2026-10-27; that plan is kept for the build pipeline it set up.

## Layering

```
monoplan-ffi (Rust staticlib)          core/ffi — uniffi over the CLI crate's daemon: Engine (start, request, watch, subscribe, import, session, device_token, shutdown), login, logout
   → MonoplanCoreFFI.xcframework        static libs (macOS/iOS/iOS-sim) + C headers
      → MonoplanCore (SwiftPM)          generated Swift bindings + a thin facade
         → MonoplanMac, MonoplanIOS      consume MonoplanCore (and MonoplanMac's Kit product) as local packages
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

# 2. Run the SwiftPM tests (the embedded engine's start-up refusals, no server needed).
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
on macOS the process that normally runs the profile's engine (`spec/daemon-plan.md`,
amendment 2026-10-27). One process: it runs the Rust engine loop
in-process (`Engine` from `MonoplanCore`, the same loop as `monoplan
daemon`) on a background thread, and its tray is an in-process client
of that loop: `DaemonClient` (MonoplanMacKit) speaks the socket's
MessagePack payloads over an `EngineTransport`, which in the app is
`FFITransport` over `Engine.request` / `Engine.watch`, never the socket. It
sends hello, opens a watch, and re-fetches `agenda --days 0 --json`
whenever the engine reports a change, once a minute, and when the menu
opens. The menu shows
Today's rows (time, title, "in 2h 15m" / "now" / "due today" /
"overdue"; a row opens the item in the web client), a Starred section
with the first three rows of the Focus lens (`starred --json`) each with
its list's name muted on the right (`list ls --json --all`), an offline
notice when there is no server, "Open Monoplan" (the window, below),
"Settings…" and "Quit Monoplan" (which stops the engine: persist, fold,
release the profile). An item row opens the window on that item.

Engine rules (`Sources/MonoplanMac/EngineController.swift`): the engine
runs whenever the app does, and the app only ever talks to its own engine. Whenever
the client has no engine and the profile is signed in it starts one,
which takes a headless `monoplan daemon` over (asks it to exit, then
claims the profile), and hands the client a transport to it. It
honours `Shutdown`, so `logout`, `login` and `password` in the
terminal stop it, hold the profile while they rewrite it, and the app
starts the engine again once it is free: the client hears the loop end through its
watch and asks the controller again. The one thing it
waits out rather than takes over is "busy": another copy of the app, a
engine still starting, or a command holding the profile; the menu says
"profile in use" and the client's backoff retries. Doc commands from
the CLI work against the app's engine; it is never restarted as stale,
its hello only has to match the app's package version, which the
bundled CLI symlinked onto `PATH` guarantees.

Three targets: `MonoplanMacKit` (MessagePack codec for the subset
`rmp_serde::to_vec_named` emits, the socket client, the agenda model,
the window's message framing and scheme routes; unit-tested, no
MonoplanCore dependency), the `MonoplanMac` executable (AppKit, the
engine controller, the window) and `MonoplanCore` (the prebuilt
XCFramework). The wire shape it hand-encodes is pinned on the Rust side
by `wire_shape_is_named_maps_and_bare_unit_variants` in `cli/src/ipc.rs`.

### The window

`Sources/MonoplanMac/WebWindow.swift`, per
[`spec/mac-window-plan.md`](../spec/mac-window-plan.md) "Swift side". A
`WKWebView` loads `monoplan://app/mirror.html`: the web client, built
by Vite and copied into `Contents/Resources/web` by `build-app.sh`,
served by the app's own URL scheme handler (right MIME types so the
wasm streams, a stable origin for `localStorage`, no `file://` rules).
The page asks the app for the session (`Engine.session`), subscribes to
each open doc (`Engine.subscribe`: a plaintext snapshot and a leased
peer id), imports the engine's later updates as they arrive and hands
its own edits back on every store flush (`Engine.import`); the web store
is the browser's, unchanged, over a mirror doc. `/api/*` requests from
the page are forwarded by the same scheme handler to the engine's server
with the device token (`Authorization: Bearer`), so the page never
holds a credential and never logs in; it holds no DEK either, so the
flows that wrap or unwrap one (new / delete workspace, invite links,
log out, delete account) are hidden there and stay with the tray, the
browser and the CLI. The engine's sync status is pushed into the page
for its footer. Closing or quitting first asks the page to hand
everything to the engine and waits for it (bounded), then drops the
subscriptions, which releases the peer slots.

While the window is open the app is a regular app (Dock icon, a main
menu whose Edit items route ⌘Z / ⌘C / ⌘V to the webview; the page
sees each key first, so its own shortcuts win); closed, it goes back
to a bare status item. Clicking the Dock icon or launching the app
again reopens it. `MONOPLAN_MIRROR_URL=http://localhost:5176/mirror.html`
loads the Vite dev server instead of the bundle for UI work (the doc
still mirrors the engine; `/api` then goes through Vite's proxy without
the token). `MONOPLAN_MAC_DEBUG=1` also enables Inspect Element and
logs the page's security context on load.

```sh
bun run build:apple                                   # once, and after core changes: the XCFramework the app links
bun run mac                                           # build Monoplan.app (debug) and run the binary inside it
bun run mac:app                                       # build Monoplan.app (release) and `open` it
bun run mac:install                                   # build release, quit a running Monoplan, install to /Applications (`-d ~/Applications`, `--no-open`) and launch
swift test --package-path apple/MonoplanMac           # codec / model tests, no engine needed
MONOPLAN_MIRROR_URL=http://localhost:5176/mirror.html bun run mac   # the window over the Vite dev server
MONOPLAN_DATA_DIR=local/cli bun run mac               # a different profile, same rule as the CLI
MONOPLAN_MAC_DEBUG=1 bun run mac                      # log state and rows to stderr
```

The in-app engine logs to `daemon.log` in the profile dir (`~/Library/
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
configuration, for the terminal), the web client in
`Contents/Resources/web` (`vite build` of `js/web`, which needs
`js/core/wasm-web`; built when missing, refreshed by hand with `bun run
build:wasm:web` after core changes, like the XCFramework), an
Info.plist (`app.monoplan.mac`,
`LSUIElement`, version from the workspace `Cargo.toml`), ad-hoc
signatures, and the app icon. The icon is the Icon Composer document
`apple/icon.icon`; `actool` compiles it straight from that file into
`Assets.car` (the layered icon macOS 26 renders) and `icon.icns` (the
flat fallback for macOS 14 and 15). Edit the icon in Icon Composer and
rebuild; the iOS project references the same document (below), nothing
else does. `bun run mac` runs the binary
*inside* the bundle rather than `swift run`, so dev runs show the icon
in Finder, Login Items and Force Quit while stderr stays on the
terminal. The menu bar glyph is separate: a template image drawn in
`main.swift`.

No account is needed: with no profile the app seeds the CLI's
local-only workspace (`monoplan-ffi::ensure_local_workspace`) and
runs it; the window runs it as the web client's anonymous session.
Settings has two panes. Account: server, status ("Local workspace (no
account)" until there is one), sync, an "Open at login" toggle
(`SMAppService.mainApp`, which lists the app under System Settings ›
Login Items; works only from an .app bundle), and Sign In / Create
Account / Sign Out. The account sheet (email, password; server and
device name under an Advanced disclosure, the server defaulting to
`https://alpha.monoplan.app`) has two modes: Sign In runs
`monoplan-ffi::login`, the same body as `monoplan auth login`, and
replaces the local workspace (the form says so); Create Account runs
`monoplan-ffi::signup`, the same body as `monoplan auth signup`, adopts
the local workspace as the account's Home doc, and shows the recovery
code once. The tray shows "Sign In…" while there is no account. Sign
Out is confirmed and runs `monoplan-ffi::logout`, the same as
`monoplan auth logout`: it stops the in-app engine, revokes this device on
the server (best-effort; an unreachable server does not block it) and wipes
the profile,
and a fresh local workspace takes its place.
Advanced: the
Engine row (running, or why not), the Build row (package version, then
the short commit `build-app.sh` baked into Info.plist, "+" if the tree
was dirty), a
"Command line" row whose Install button symlinks `/usr/local/bin/monoplan`
to the bundled binary (admin prompt if the directory is root's), and
the profile path with a Reveal button. The
launchd agent from `monoplan daemon install` is the headless
alternative for Macs without the app: the app takes its daemon over at
launch, and `install` refuses while the app's engine runs.

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

Next: the window's deferred items in `spec/mac-window-plan.md`
(DEK-touching flows in the window, several windows, typed uniffi
request and reply for the tray).

## MonoplanIOS — the native iOS surface

`apple/MonoplanIOS/` (2026-11-05) is a hand-written Xcode project (no
xcodegen on this machine; `Monoplan.xcodeproj` uses a synchronized
folder, so new files under `Monoplan/` need no project edit) with one
SwiftUI target. It is the first native client and exists to drive the
architecture, not to be used yet: sign in, sign out, run a sync, and
show what the engine reports.

The app runs the profile's engine, as the Mac app does: `EngineController`
runs `Engine` from MonoplanCore in process and `DaemonClient` (the Kit,
consumed as the `MonoplanMacKit` product of `../MonoplanMac`) talks to
it through `FFITransport`, hello then watch. Two things differ from
the Mac. The engine serves **no socket**: nothing on iOS would dial one,
and the container paths are longer than `sun_path` allows, so
`monoplan-ffi` passes `serve_socket = false` (`daemon::start_with`)
under `target_os = "ios"` and skips the headless-daemon takeover. And
the process is suspended rather than quit: on return to the
foreground the app sends `Reconnect`, since the WebSocket is dead by
then (`spec/daemon-plan.md` "Mobile hosts"); there is no clean
shutdown, the engine's persist tick and the WAL cover a kill. The
missing socket also means `login` cannot stop the app's engine itself,
as it does on the Mac (`client::claim` sends `Shutdown` over the
socket and would wait out its deadline here), so `EngineController`
shuts its engine down before calling `login`, as both apps already do
before `logout`; a rejected password costs an engine restart.

```sh
bun run ios          # build for the simulator, install, launch
bun run ios:build    # build only
apple/MonoplanIOS/build-sim.sh -d "iPhone 17 Pro" --run
xcrun simctl io booted screenshot shot.png   # Xcode 27 ships no Simulator.app
```

Needs `bun run build:apple` first. DerivedData goes to
`apple/MonoplanIOS/.build/` (gitignored). A device build needs a
signing team: set `DEVELOPMENT_TEAM` in the environment or in Xcode.

The app icon is the same `apple/icon.icon` the Mac bundle uses, added
to the project as a file reference (`../icon.icon`) in the Resources
phase with `ASSETCATALOG_COMPILER_APPICON_NAME = icon`; Xcode runs
`actool` on it as it would an asset catalog, producing the layered
icon in `Assets.car` for iOS 26 and flat `icon60x60` / `icon76x76`
PNGs for iOS 17 and 18. There is no `Assets.xcassets`; one is only
needed once the app wants named colours or images of its own.

Four tabs in a system `TabView` (`ContentView.swift`; the floating
glass pill on iOS 26, the flat bar before). Item rows outside the
Agenda lead with a checkbox (`ItemCheckbox.swift`: empty while open,
ticked when Done, crossed when Cancelled). Tapping it ticks an open item
or reopens a closed one into Backlog (`AppModel.setDone` →
`Request.setDone`, i.e. `item set <id> --lifecycle done|backlog`; the
hand-encoded frame is pinned by a test in `cli/src/commands/items.rs`),
after which every loaded read is fetched again at once: no optimistic
state. The first write from the iOS app. Every such row opens the
item as a sheet over its list (`ItemView.swift`, 2026-11-13; half
height, draggable to full; `opensItems()` on each tab's stack presents
it): the web task dialog's
content in its order (status, title, list, planned date and length,
deadline when set, place, notes, activity log), read through
`Request.item(id:workspace:)`, i.e. `item show <id> --json`
(`spec/cli.md` §Items; decoded by `ItemDetail.parse`), and refetched on
`Changed` while shown; its checkbox ticks too, the rest is display only. A place
with coordinates also shows a small non-interactive MapKit map with a
pin (`PlaceMapCard`); tapping it opens the spot in Apple Maps under the
place's label. Tiles come from Apple, so this is a second non-E2EE
lookup beside Photon (`spec/place-plan.md`). **Inbox** (`InboxView.swift`)
is the capture list's Open items in list order, read through the Kit's
`Request.items` (`item ls --list inbox --json`, decoded by
`Items.parse`) and refetched on every `Changed` event, coalesced over
250 ms as the Mac tray's agenda is; read-only so far, capture and
lifecycle moves are next. **Starred** (`StarredView.swift`) is the
Focus lens as the daemon resolves it (`Request.focus(all: true)`, i.e.
`starred --all --json`, `spec/cli.md` §Focus: Home's curated order with
its cross-workspace entries resolved), each row with its list name and,
when not Personal's, its workspace; read-only. **Agenda** (`AgendaView.swift`) is the
daemon's merged agenda (`Request.agenda(days: AppModel.agendaDays, all: true)`, i.e.
`agenda --all --json`, `spec/cli.md` §Items: every open workspace in
one placement, the Home doc's hidden-from-calendar docs left out; the
horizon is a century, so the whole future in one fetch, no paging), every
group as a section: Today, Unscheduled events, then each non-empty
day, with the row's time, its workspace under the text when it is not
Personal's, and the duration or overdue / due-today tone; read-only
markup, nothing tappable yet. **Browse** (`BrowseView.swift`,
2026-11-13) is the index: a section per open workspace (`Engine.session`'s
docs, Personal first), each with its Inbox and then its active lists
(`Request.lists(workspace:)`, i.e. `list ls --json --all -w <doc id>`,
one fetch per doc, refetched with the session on every `Changed`;
`Lists.active` drops archived and reserved rows). A list shows its
emoji (`ListIcon`: the `icon` the list JSON carries, a folder without
one) on its Browse row, in its page's title and on the item sheet. A list pushes its Open
items (`Request.items(list:workspace:)`), refetched on `Changed` while
shown; read-only. Under the workspaces sit the two pages that lost their
tabs, pushed onto Browse's navigation stack. The tab keeps
its place across tab switches, and tapping it again pops to the index
(the system's behaviour, nothing of ours). **Done** (`DoneView.swift`) is the
Personal workspace's Done items across every list, newest first, in a
section per day ticked (`Request.done(limit:)`, i.e. `done -n N --json`,
grouped by `Done.days`), each row with its list name. It is a window,
not the history: 50 rows to start, doubled each time the end of the
list scrolls into sight (`AppModel.loadMoreDone`), and a `Changed`
refetches only the window reached; read-only. **Debug**
(`DebugView.swift`) is the original surface: the `SyncStatus` the engine
publishes (online, pending, local-only, last synced, offline reason),
`Changed` events counted as they arrive, the engine's build id and pid,
the profile path, and the `Engine.session` record (email, account,
device, server, open docs), with the sign-in / sign-out buttons until
a settings surface takes them. Sign-in is the Mac sheet's form without
account creation or recovery; the local-only workspace a first launch
runs is replaced by the account's, as on the Mac.

Not shared yet: `EngineController` and `FFITransport` are copies of the
Mac app's. The obvious next step is a third target beside the Kit,
depending on MonoplanCore, that both apps consume; the Kit's name
(`MonoplanMacKit`) should go with it. After that, the native reads
the UI needs (lists, items, a board) belong on the FFI as typed
calls, and the cross-doc layer the web client keeps in
`js/web/src/sync/docs.ts` belongs in Rust above `Doc`, since a native
UI cannot reuse it.
