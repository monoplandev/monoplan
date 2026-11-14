# Monoplan on Apple

The Rust↔Swift FFI boundary for the Apple clients. What crosses it is
the profile's lifecycle (`login` / `signup` / `logout`, the bodies of
the CLI commands) and `Engine`, the daemon loop embedded in the app's
process (`spec/daemon-plan.md`, amended 2026-10-27), which both apps
drive through typed calls returning generated records and follow with
`Engine.observe` ([`spec/swift-bindings-plan.md`](../spec/swift-bindings-plan.md),
built 2026-11-14). Neither app opens the socket, encodes
a request or decodes JSON from the engine; the socket stays the CLI's.
Both are native: `apple/MonoplanIOS` (2026-11-05) and, since
2026-11-14, the Mac app's window, which replaced the webview of
[`spec/mac-window-plan.md`](../spec/mac-window-plan.md). The surface
that webview and the first tray used (`Engine.request`, `watch`,
`subscribe`, `import`, `session`) was removed on 2026-11-14 with the
mirror behind it (the bindings plan's phase 5). The offline `MonoplanStore` prototype from
[`spec/swift-ffi-plan.md`](../spec/swift-ffi-plan.md) was removed on
2026-10-27; that plan is kept for the build pipeline it set up.

## Layering

```
monoplan-ffi (Rust staticlib)          core/ffi — uniffi over the CLI crate's daemon: Engine (start, shutdown, is_running, device_token; typed: one method per `monoplan_cli::api` function, plus sync, reconnect, account, workspaces, create_workspace, delete_workspace, observe), login, signup, logout, ensure_local_workspace, matches_name, build_id
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
- `apple/MonoplanCore/Sources/MonoplanCore/Generated/` — the uniffi Swift bindings,
  one file per Rust crate that defines exported types: `monoplan_ffi.swift`
  (`Engine` and its methods) and `monoplan_cli.swift` (the records the typed
  calls return: `Item`, `ItemInfo`, `ListInfo`, `AgendaGroup`, `StarredItem`,
  `FindHit`, `BinEntry`, `LocatedItem`, `PlaceSuggestion`, ...).

From Swift: `EngineCalls(engine)` and
`try await calls.run { try $0.items(workspace: nil, list: "inbox", includeDone: false) }`
(or `calls.run({ ... }) { result in ... }`, the result on the main queue,
for a caller that is not async);
`engine.observe(onChanged:onSync:onStopped:)` for the change and sync
events. Both apps use nothing else: an `EngineController` that follows
the engine through `observe` and hands out its `EngineCalls`.

Run it before `swift build` / `swift test` on a fresh checkout — the package
won't compile without those outputs present.

## MonoplanMac — the menubar app

`apple/MonoplanMac/` is a SwiftPM package: the macOS status-item app, and
on macOS the process that normally runs the profile's engine (`spec/daemon-plan.md`,
amendment 2026-10-27). One process: it runs the Rust engine loop
in-process (`Engine` from `MonoplanCore`, the same loop as `monoplan
daemon`) on a background thread, and its tray and window call that
engine's typed methods (`EngineCalls`), never the socket. The tray
reads `Engine.agenda(days: 0)` and `Engine.starred()` again whenever
the engine reports a change, once a minute, and when the menu opens.
The menu shows Today's rows (time, title, "in 2h 15m" / "now" / "due
today" / "overdue"), a Starred section with the first three rows of
the Focus lens, each with its list's name muted on the right, an
offline notice when there is no server, "Open Monoplan" (the window,
below), "Settings…" and "Quit Monoplan" (which stops the engine:
persist, fold, release the profile). Both reads span every open
workspace. An item row opens the window on the item's list. The rows
are rendered in `Sources/MonoplanMac/TrayMenu.swift`.

Engine rules (`Sources/MonoplanMac/EngineController.swift`): the engine
runs whenever the app does, and the app only ever talks to its own engine. Whenever
there is no engine it starts one, which takes a headless `monoplan
daemon` over (asks it to exit, then claims the profile), follows it
with `Engine.observe` and reports its state (`EngineState`: running
with the sync state, or not and why). It
honours `Shutdown`, so `logout`, `login` and `password` in the
terminal stop it, hold the profile while they rewrite it, and the app
starts the engine again once it is free: the observer hears the loop
end. The one thing it
waits out rather than takes over is "busy": another copy of the app, an
engine still starting, or a command holding the profile; the menu says
"profile in use" and the start is retried, backing off to ten seconds.
Doc commands from
the CLI work against the app's engine; it is never restarted as stale,
its hello only has to match the app's package version, which the
bundled CLI symlinked onto `PATH` guarantees.

Targets: `MonoplanMacKit` (Foundation-only helpers: `ProfileLocation`,
`WhenPhrase`, the widget's agenda snapshot, the device API requests,
install / launch / appearance preferences; no MonoplanCore dependency,
because the iOS widget links it and must not link the engine), the
`MonoplanMac` executable (AppKit and SwiftUI: the engine controller,
the tray, the window, Settings) and `MonoplanCore` (the prebuilt
XCFramework). Two test targets: `MonoplanMacKitTests`, and
`MonoplanMacTests` for the executable's pure code (the tray's rows),
which links MonoplanCore and so needs `build:apple` run first.

### The window

`Sources/MonoplanMac/MainWindow.swift`, per
[`spec/swift-bindings-plan.md`](../spec/swift-bindings-plan.md) "The
macOS window": native SwiftUI over the typed calls, and for now
deliberately a debug screen. The chrome is AppKit
(`Sources/MonoplanMac/MainSplit.swift`): a split view controller of
sidebar, list and the item's panel as inspector under one unified
toolbar, whose tracking separators keep the panel's controls (close,
status, star) over the panel; the window's title is the name of what is
listed. A sidebar shaped as the web nav
is (the lenses Starred, Calendar and Done on top, then one group per open workspace, Personal
first; each a heading that folds its group, the fold kept in
`UserDefaults`, over Inbox and the active lists with their open counts,
`Engine.lists` once per workspace), the Calendar (`Engine.agenda` over
every workspace: Today, Unscheduled, then each non-empty day, rows
with a tick; a tray row for an event opens it), Starred
(`Engine.starred`, with a tick and a star that takes the row out), Done
(`Engine.done` per workspace, merged newest first, capped at 200, the
tick putting a row back), and the selected list's
open items (`Engine.items`) with a tick (`Engine.setDone`) and a field
that adds one (`Engine.addItem`). Everything shown is read in one turn
of the call queue, again on every change the engine reports, on a new
engine, and after each of its own writes; nothing is optimistic. A row
of a list or a lens can be selected, which opens the item's panel at
the right of the window (`ItemPanel.swift`, `Engine.item` in the same
read): the iOS item page top to bottom (status, title, list, planned
date, deadline, place with its map, notes, activity), read only but
for its tick and its star (`Engine.setStarred`); the cross, Escape, a
click on no row or a move to another list closes it. No board, no drag
and drop, no item editing: those come from sharing views with iOS. Monoplan › New Workspace… asks for a name and has the
engine create and open the doc (`Engine.createWorkspace`).

While the window is open the app is a regular app (Dock icon, a main
menu whose Edit items route ⌘Z / ⌘C / ⌘V to the focused text field);
closed, it goes back to a bare status item. Clicking the Dock icon or
launching the app again reopens it. Settings › General "Open window at
launch" (on by default) opens it with the app.

```sh
bun run build:apple                                   # once, and after core changes: the XCFramework the app links
bun run mac                                           # build Monoplan.app (debug) and run the binary inside it
bun run mac:app                                       # build Monoplan.app (release) and `open` it
bun run mac:install                                   # build release, quit a running Monoplan, install to /Applications (`-d ~/Applications`, `--no-open`) and launch
swift test --package-path apple/MonoplanMac           # Kit and tray-row tests, no engine started
MONOPLAN_DATA_DIR=local/cli bun run mac               # a different profile, same rule as the CLI
MONOPLAN_MAC_DEBUG=1 bun run mac                      # log state and rows to stderr
```

The in-app engine logs to `engine.log` in the profile dir (`~/Library/
Application Support/monoplan/engine.log` by default; `RUST_LOG`
overrides the `info` level): connect failures and drops land there
once per distinct reason, and the same reason shows on the Settings
Sync row and in `monoplan daemon status` ("offline; never synced
(connect failed: …)"). The app's own `NSLog` lines go to the unified
log: `log show --predicate 'process == "MonoplanMac"' --last 1h`.

`apple/MonoplanMac/build-app.sh` lays out `apple/MonoplanMac/.build/Monoplan.app`
(gitignored): the SwiftPM binary (which links MonoplanCore; the script
runs `build-xcframework.sh` when the XCFramework is missing), the
`monoplan` CLI beside it in `Contents/MacOS` (built by cargo in the same
configuration, for the terminal), an
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
runs it; the window shows it like any other.
Settings has four panes. General: appearance and "Open window at
launch". Devices: the account's devices over the server API (rename,
revoke), with the engine's device token. Account: server, status ("Local workspace (no
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

Next: real views in the window, shared with iOS.

## MonoplanIOS — the native iOS surface

`apple/MonoplanIOS/` (2026-11-05) is a hand-written Xcode project (no
xcodegen on this machine; `Monoplan.xcodeproj` uses a synchronized
folder, so new files under `Monoplan/` need no project edit) with one
SwiftUI target. It is the first native client and exists to drive the
architecture, not to be used yet: sign in, create an account, sign
out, run a sync, and show what the engine reports.

The app runs the profile's engine, as the Mac app does: `EngineController`
runs `Engine` from MonoplanCore in process, follows it with
`Engine.observe` (sync state, `Changed`, its end) and hands `AppModel`
its `EngineCalls`. There is no client object or wire format between
the app and its engine (spec/swift-bindings-plan.md, phase 3): every
read and write is a typed method returning generated records. The Kit
(the `MonoplanMacKit` product of `../MonoplanMac`) is still linked for
what is not wire: `ProfileLocation`, `WhenPhrase` / `WhenSuggest`,
`Agenda.start(of:)` and the widget's `AgendaSnapshot`. Two things differ from
the Mac. The engine serves **no socket**: nothing on iOS would dial one,
and the container paths are longer than `sun_path` allows, so
`monoplan-ffi` passes `serve_socket = false` (`daemon::start_with`)
under `target_os = "ios"` and skips the headless-daemon takeover. And
the process is suspended rather than quit: on return to the
foreground the app calls `Engine.reconnect`, since the WebSocket is dead by
then (`spec/daemon-plan.md` "Mobile hosts"); there is no clean
shutdown, the engine's persist tick and the WAL cover a kill. The
missing socket also means `login` cannot stop the app's engine itself,
as it does on the Mac (`client::claim` sends `Shutdown` over the
socket and would wait out its deadline here), so `EngineController`
shuts its engine down before calling `login`, as both apps already do
before `logout`; a rejected password costs an engine restart. `signup`
(2026-11-14) goes the same way: the shutdown persists the local-only
workspace, signup adopts it, and the sheet shows the recovery code
once.

```sh
bun run ios          # build for the simulator, install, launch
bun run ios:build    # build only
apple/MonoplanIOS/build-sim.sh -d "iPhone 17 Pro" --run
xcrun simctl io booted screenshot shot.png   # Xcode 27 ships no Simulator.app
```

Needs `bun run build:apple` first. DerivedData goes to
`apple/MonoplanIOS/.build/` (gitignored). A device build needs a
signing team: set `DEVELOPMENT_TEAM` in the environment or in Xcode.

A second target, `MonoplanWidgetExtension` (2026-11-13; sources in the
synchronized folder `MonoplanWidget/`, embedded in the app), is the
home-screen Agenda widget: today's open rows, then what is next, in
the small, medium and large sizes, display only. It runs no engine
and links only the Kit. The app leaves it the agenda projection's
JSON in the container of the app group `group.app.monoplan.ios`
(`AgendaSnapshot` in the Kit; written by `AppModel.refreshAgenda`
when the read changed, with a `WidgetCenter` reload, and removed on
sign-out), so the widget is as fresh as the app's last read: a change
from another device shows once the app has been opened. That file is
item text in the clear outside the encrypted store, under iOS data
protection only. Both targets carry the app group entitlement
(`Config/*.entitlements`; the widget's `NSExtension` plist is beside
them), so a device build has to register the group with the team:
automatic signing in Xcode does it, `xcodebuild` needs
`-allowProvisioningUpdates`.

The app icon is the same `apple/icon.icon` the Mac bundle uses, added
to the project as a file reference (`../icon.icon`) in the Resources
phase with `ASSETCATALOG_COMPILER_APPICON_NAME = icon`; Xcode runs
`actool` on it as it would an asset catalog, producing the layered
icon in `Assets.car` for iOS 26 and flat `icon60x60` / `icon76x76`
PNGs for iOS 17 and 18. There is no `Assets.xcassets`; one is only
needed once the app wants named colours or images of its own.

One navigation stack rooted at Home, no tab bar (2026-11-13; the
four-tab `TabView` and `ContentView.swift` went with it): the app opens
on the index and every other page is pushed from it. A floating bar
(`BottomBar.swift`: a search field with an add button to its right,
glass on iOS 26, material before) is the stack's bottom inset, so it
shows on every page. Search (2026-11-13, `SearchView.swift`) works as
the web's Find does (`spec/search.md`): while the field has focus or
text, a page covers the stack, blank until something is typed (no
default menu here), then the built-in pages whose names match
(`matches_name` over the FFI), lists, and items, ranked across every
open workspace (`AppModel.search` → `Request.find`, i.e. `find <query>
--json`, on each keystroke and again on `Changed`). A page or list
result closes search and is pushed; an item result opens its sheet over
the results. For as long as search is up the add button is a cross that
clears the text and drops the keyboard; add itself does nothing yet.
Item rows outside the
Agenda lead with a checkbox (`ItemCheckbox.swift`: empty while open,
ticked when Done, crossed when Cancelled). Tapping it ticks an open item
or reopens a closed one into Backlog (`AppModel.setDone` →
`Request.setDone`, i.e. `item set <id> --lifecycle done|backlog`; the
hand-encoded frame is pinned by a test in `cli/src/commands/items.rs`),
after which every loaded read is fetched again at once: no optimistic
state. The first write from the iOS app. Every such row opens the
item as a sheet over its list (`ItemView.swift`, 2026-11-13; half
height, draggable to full; `opensItems()` on Home's stack presents
it): the web task dialog's
content in its order (status, title, list, planned date and length,
deadline when set, place, notes, activity log), read through
`Request.item(id:workspace:)`, i.e. `item show <id> --json`
(`spec/cli.md` §Items; decoded by `ItemDetail.parse`), and refetched on
`Changed` while shown; its checkbox ticks too, and its list field is a
pull-down `Menu` of the workspace's lists that moves the item
(`Request.moveItem`, i.e. `item set <id> --list <list>`; same workspace
only, the engine has no cross-doc move). Its date field
(`ItemDateField.swift`) hands the whole sheet to an editor
(`ItemDateEditor`, Done returns to the item): a text box it opens on,
keyboard up, that reads a typed date ("fri 3pm") with `WhenPhrase` and
offers its readings underneath (2026-11-13; `WhenPhrase.readings`: one,
or two when the phrase said only a day or only a time and the item's
date has the other half; while a date is set the box shows it, the
keyboard down, and focusing it selects the text to type over; a tap or
Return takes an offer and leaves the editor up, so "fri" can be followed by "3pm"; only the tick
and the cross close it; with nothing typed and no date set the offers are Today and
Tomorrow, which typing replaces unless the text is the start of one),
then a month calendar (`MultiDatePicker` read as a range picker: a tap
sets the day, the next tap on a later day ends the span there, up to
seven days, the one after starts over; trial, 2026-11-13), and once a
date is set its start and end time
side by side (the web's time row: the end is start + `duration`, an end
at or before the start is the next day) with Clear time under them, or
on an all-day date Add time and a Length in days, then Clear date, each pick written at once (`Request.setWhen` /
`setDuration`, i.e. `item set --when / --duration`). A date phrase in a
title ("dentist friday 8:30") raises a date suggestion (2026-11-13,
`spec/calendar-plan.md` "Typed date phrases"): the date row turns
orange, "Suggested: Friday 20 Nov at 08:30", with a tick that makes it
the date (`Request.setWhen`) and a cross that drops it
(`Request.setWhenSuggested`, i.e. `item set --when-suggested`). It is
written when a capture creates the item and when the title editor
saves a title with a new phrase; the phrase is read by `WhenPhrase`
(MonoplanMacKit, over `NSDataDetector`) and the rule is
`WhenSuggest.edit`. Its place row
does the same (`ItemPlaceEditor.swift`): one text box, then the typed
text as a bare label, the account's known places that match
(`Request.places`, i.e. `places --json`, `spec/cli.md` §Items; nothing
leaves the device), and from three characters Apple's search
(`MKLocalSearchCompleter`, resolved to coordinates by `MKLocalSearch`;
the query goes to Apple, not E2EE, as the web's goes to Photon). A pick
is written at once (`Request.setPlace`, i.e. `item set --place
[--coords] [--address] [--new-place]`). Deadline and notes are display
only. A place
with coordinates also shows a small non-interactive MapKit map with a
pin (`PlaceMapCard`); tapping it opens the spot in Apple Maps under the
place's label. Tiles come from Apple, so this is a second non-E2EE
lookup beside Photon (`spec/place-plan.md`). There is no Inbox page of its own: Personal's Inbox is the first row of
its workspace section in Home, a list page like any other. The lens
reads are refetched on every `Changed` event, coalesced over 250 ms as
the Mac tray's agenda is. **Starred** (`StarredView.swift`) is the
Focus lens as the engine resolves it (`Request.focus(all: true)`, i.e.
`starred --all --json`, `spec/cli.md` §Focus: Home's curated order with
its cross-workspace entries resolved), each row with its list name and,
when not Personal's, its workspace; a long press and drag reorders a row
(2026-11-14: `Request.moveStarred`, i.e. `starred mv <id> --after <other>`). **Agenda** (`AgendaView.swift`) is the
engine's merged agenda (`Request.agenda(days: AppModel.agendaDays, all: true)`, i.e.
`agenda --all --json`, `spec/cli.md` §Items: every open workspace in
one placement, the Home doc's hidden-from-calendar docs left out; the
horizon is a century, so the whole future in one fetch, no paging), every
group as a section: Today, Unscheduled events, then each non-empty
day, with the row's time, its workspace under the text when it is not
Personal's, and the duration or overdue / due-today tone; read-only
markup, nothing tappable yet. **Home** (`HomeView.swift`,
2026-11-13) is the root and the index: first a section with Starred, Agenda, Map, Done and Bin, then a section per open workspace (`Engine.workspaces`,
Personal first), each with its Inbox and then its active lists
(the typed `Engine.lists(workspace:filter: .all)`,
one read per doc, refetched with the session on every `Changed`;
`userLists` in `TypedRows.swift` drops archived and reserved rows). A list shows its
emoji (`ListIcon`: the `icon` the list JSON carries, a folder without
one) on its Home row, in its page's title and on the item sheet. A list pushes its Open
items (the typed `Engine.items`), refetched on `Changed` while
shown; a long press and drag reorders a row (2026-11-14: `onMove`,
`Engine.reorderItem`; the rows are
reordered locally first, then reread). A gear at the top right of the index opens Settings, pushed onto the same stack. **Map**
(`PlacesMapView.swift`, 2026-11-14) is the Open items that carry a
place with coordinates (`Engine.placedItems`, every open workspace), one marker per spot, under a sheet
that stays up with the page (strip, half, full; the map stays live up
to half): the items nearest first, from the phone's location
(`LocationSource`, when-in-use, used on the device only) or else the
middle of the map, with a search field; a marker narrows the list to
its spot, and a row pushes the item's page inside the sheet. Home's
bottom bar is hidden while it is up. **Done** (`DoneView.swift`) is the
Personal workspace's Done items across every list, newest first, in a
section per day ticked (`Engine.done(workspace:limit:)`,
grouped by `DoneDay.days`), each row with its list name. It is a window,
not the history: 50 rows to start, doubled each time the end of the
list scrolls into sight (`AppModel.loadMoreDone`), and a `Changed`
refetches only the window reached; read-only. The title's count is
the whole history's, read on its own (`Engine.doneCount`). **Bin** (`BinView.swift`, 2026-11-14) is every open
workspace's binned items, newest binned first (`Engine.binned`, one
read merged in Rust; read with the other lenses, and Home shows its Bin row only while
something is binned; search reaches the page either way),
each row with its list, its workspace when not Personal's, and how long
ago it was binned. A row opens the item's sheet (Restore, Delete); a
swipe or a long press restores (`Engine.setBinned`) or deletes
(`Engine.deleteBinned`) from the list, and "Empty" deletes the lot
(`Engine.emptyBin`, per workspace with something binned). Both deletes ask first. **Settings**
(`SettingsView.swift`, 2026-11-13) holds what a person acts on: the
account (email, server, sign in / sign out), sync (the status line,
last synced) and the last error. **Debug**
(`DebugView.swift`, the original surface) is pushed from its last row
and keeps the rest: the raw `SyncState` flags (online, pending,
local-only, offline reason), `Changed` events counted as they arrive,
Sync now and Reconnect, the engine's build id and pid, the profile path, and the
session's ids (`Engine.account`, `Engine.workspaces`: account, device, open docs). Sign-in is the Mac sheet's form without
account creation or recovery; the local-only workspace a first launch
runs is replaced by the account's, as on the Mac.

Not shared yet: each app has its own `EngineController`. They are the
same shape since macOS moved onto the typed calls
(spec/swift-bindings-plan.md, phase 4) and can become a third target
beside the Kit, depending on MonoplanCore, that both apps consume; the
Kit's name (`MonoplanMacKit`) should go with it. The
reads the UI needs are on the FFI as typed calls, the ones that span
workspaces merged in Rust (`monoplan_cli::api`); what remains of the
cross-doc layer the web client keeps in `js/web/src/sync/docs.ts`
(moving an item between workspaces) is not on the FFI yet.
