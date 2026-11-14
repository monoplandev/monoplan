# Typed Swift bindings (plan; phases 1 and 2 built 2026-11-14)

Decided 2026-11-14. Supersedes the client half of
[`mac-window-plan.md`](mac-window-plan.md): the webview window is
scrapped, macOS goes native like iOS, and both apps stop driving the
engine through CLI commands.

## Goal

1. macOS and iOS call the engine through typed uniffi methods and
   records. No MessagePack, no clap-shaped requests, no JSON-in-a-string
   replies in Swift.
2. A very basic native macOS window replaces the webview. A debug
   screen is enough.

`Engine.request` and the rest of the old surface are removed once both
apps are over (phase 5), not before.

## What is wrong today

Every read and write from Swift goes: hand-encoded msgpack shaped like
the clap arg structs → `Engine.request(bytes)` → `run_doc` renders to a
`stdout` string → msgpack `Reply::Output` → `JSONDecoder` into
hand-kept Swift structs. Swift mirrors the Rust types twice (request
shapes in `EngineClient.swift`, reply rows in `Agenda.swift`,
`Items.swift`, ...) with nothing but `wire_shape_is_named_maps_and_bare_unit_variants`
holding them together. Errors arrive as a rendered message string.

## Probe (run 2026-11-14, uniffi 0.32.2)

Question: can the row types live in the crate that computes them (a
plain rlib, uniffi optional) and be exported from `monoplan-ffi`?

Yes. A two-crate workspace, `rows` (rlib; `uniffi` an optional
dependency behind a `uniffi` feature) and `ffi` (staticlib + cdylib,
depends on `rows` with the feature on), built, generated Swift in
library mode, and a Swift program linked against the staticlib ran
through it: nested records, `Option` fields, `Vec<Record>`, a plain
enum, and a typed error thrown across the crate boundary and caught as
`OpError.NoSuchItem(let id)`.

What it takes:

- In the type crate: `#[cfg(feature = "uniffi")] uniffi::setup_scaffolding!();`
  at the crate root and `#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]`
  (`Enum`, `Error`) on each type. Nothing else; the exporting crate
  just names the types in its signatures.
- Library-mode bindgen then emits **one set of files per crate**:
  `<crate>.swift`, `<crate>FFI.h`, `<crate>FFI.modulemap`.
  `apple/build-xcframework.sh` copies exactly one of each today, so it
  has to change: copy every `*.swift` into `Generated/`, every `*.h`
  into the headers dir, and concatenate the modulemaps into the one
  `module.modulemap` (two `module` blocks in one file works).
- Both generated Swift files compile in one Swift module with no
  import configuration.
- Records must be owned. The CLI's current row structs borrow
  (`ItemJson<'a>`, `RowJson<'a>`), so they cannot be reused as is.
- Generated names land unprefixed in the app's module. A Rust `State`
  becomes a Swift `State` and collides with SwiftUI's `@State`; name
  the types for Swift (`ItemState`, `ListInfo`). Until `MonoplanMacKit`'s
  models are deleted they must not share its names either (`ItemRow`,
  `ListRow`, `SyncStatus`, `AgendaDay`, `AgendaRow`, ...): a file that
  imports both modules would stop compiling on the ambiguity. Hence
  `Item`, `ListInfo`, `Place`, `SyncState`; all checked free in SwiftUI,
  MapKit, CoreLocation and WidgetKit on the macOS and iOS SDKs.

Not probed: three crates deep (core → cli → ffi). Same mechanism; not
needed under the layout below.

## Rust side

### `cli/src/api/`: the typed layer

One module per area (`items`, `lists`, `agenda`, `focus`, `bin`,
`done`, `find`, `places`), each holding owned row types and plain
functions over a context:

```rust
pub struct Ctx<'a> {
    pub doc: &'a Doc,            // the resolved workspace
    pub open_docs: &'a [&'a Doc],
    pub lens: &'a focus::Lens<'a>,
}

pub fn items(cx: &Ctx, list: &str, include_done: bool) -> anyhow::Result<Vec<Item>>;
pub fn add_items(cx: &Ctx, list: &str, texts: &[String], top: bool) -> anyhow::Result<Vec<String>>;
```

This is the query / mutate half of what each `commands/*.rs` function
does today, with the rendering cut off. The command functions become
args → `api::` call → render (`--json` serialises the same owned rows,
so `ItemJson<'a>` and friends go away and the JSON shape is pinned by
the existing CLI tests). Behaviour moves; it is not duplicated.

- Row types derive `Serialize` always and the uniffi traits behind the
  cli crate's new `uniffi` feature, which only `monoplan-ffi` turns on.
- Core types that cross (`Place`) are copied into an api row type at
  the boundary rather than deriving uniffi in `monoplan-core`: one
  generated Swift file instead of two, and core stays free of it.
- Writes are per field, as the Swift `Request` enum already is
  (`set_text`, `set_when`, `set_duration`, `set_place`, `set_starred`,
  `move_item`, ...), not one patch record. Keep / clear / set does not
  survive `Option<Option<T>>` across uniffi; where it is needed, a
  small enum (`DurationEdit`).
- Writes return nothing (`add_item` returns the id). The UI re-reads on
  the change event, as it does today.
- Functions return `anyhow::Result`, as the commands they came out of
  do, so a command calls them with `?`. A caller outside the loop gets
  `ApiError` (`Stopped`, `NoSuchWorkspace`, `Failed { message }`),
  converted at `call_blocking`. A failure Swift has to branch on is
  raised as an `ApiError` variant inside the `anyhow::Error` and
  downcast back out; add variants when one is needed. At the FFI it
  maps into the existing `EngineError`, so Swift keeps one error type.

### Reaching the loop

One new `HostCmd` variant carrying a closure, instead of a typed
request and reply enum per verb:

```rust
HostCmd::Call {
    workspace: Option<String>,
    run: Box<dyn FnOnce(Result<&Ctx, ApiError>) -> Box<dyn FnOnce() + Send> + Send>,
}
```

`run` computes the result and returns the thunk that sends it; the
loop runs the thunk after the tail below, so a write is durable before
its caller hears back, as with a doc command's reply.

`LocalClient::call_blocking(workspace, |cx| api::items(cx, ..))` wraps
the closure around a oneshot and blocks for it. The loop resolves the
workspace, builds the `Ctx` exactly as the `Request::Doc` arm does
(open docs, Home or foreign lens), runs the closure, then runs the
same tail: `persist_replica_state`, `flush_replicas`, `pump`,
`notify_if_changed`. That tail is factored out of the `Request::Doc`
arm so both paths share it. In process nothing is serialised.

The cross-doc reads the loop special-cases today (`agenda --all`,
Starred, `find`, `places --items`) become api functions over
`open_docs`; the loop's special arms then call those.

### `monoplan-ffi`

`Engine` gains one method per api function, each a `call_blocking`,
plus typed versions of the non-doc requests:

- `sync()`, `reconnect()` → `SyncState`
- `account()` → `Account?`
- `create_workspace(name)`, `delete_workspace(doc_id)`
- `observe(observer)` → `Watcher`, with
  `trait EngineObserver { fn changed(); fn sync(state: SyncState); fn stopped(); }`
  replacing the msgpack frame sink. Same forwarder thread as `watch`.

Calls stay blocking; Swift puts them on a queue. uniffi async is not
needed for this.

## Swift side

- `MonoplanCore` gets a thin facade over `Engine`: `EngineCalls`
  (one serial queue; `try await calls.run { try $0.items(...) }`, so
  there is no per-verb wrapper to keep in step; a callback form,
  `calls.run({ ... }) { result in }` delivering on the main queue, for
  the callback-shaped `AppModel` and the AppKit tray) and
  `Engine.observe(onChanged:onSync:onStopped:)` delivering on the main
  queue. This replaces `EngineClient` and both `FFITransport`s.
- `MonoplanMacKit` loses `MessagePack.swift`, `EngineClient.swift`,
  `MirrorFraming.swift` and the JSON model files; the generated
  records replace the models, with extensions for the derived bits
  (`SyncStatus.level`, `offlineNotice`). It keeps presentation helpers
  (`WhenPhrase`, `AgendaSnapshot`, install / launch preferences). It
  then depends on `MonoplanCore`, so its tests need `build:apple` run
  first; accepted.
- The engine-lifecycle rules in the two `EngineController`s do not
  change.

## The macOS window

A SwiftUI window, opened from the tray where "Open Monoplan" opens the
webview today. First cut, deliberately a debug screen:

- session line: email, server, sync status from the observer
- workspace picker, list sidebar (`lists`)
- items of the selected list (`items`), tick to done (`set_done`), a
  text field to add (`add_item`)
- re-reads on `changed`

No board, no drag and drop, no item editor. Those come from sharing
views with iOS later, which is its own piece of work.

## Phases

1. **Rust foundation and one slice.** Built 2026-11-14. `api::{lists, items}` with
   `lists`, `items`, `add_item`, `set_done`; `HostCmd::Call` and the
   shared tail; the `uniffi` feature; `Engine.lists/items/add_item/set_done/observe`;
   build script handles multiple generated files. `item ls` / `item add`
   / `list ls` render from the api rows. Check: `cargo test` (CLI JSON
   unchanged), `bun run build:apple`, a `MonoplanCore` Swift test that
   starts a local-only engine and round-trips an item. Not yet in this
   phase: `sync`, `reconnect`, `account` and the workspace calls
   (phase 3, with the screens that use them).
2. **iOS slice.** Built 2026-11-14. The Home list screen reads and
   writes through the new calls; everything else stays on
   `EngineClient`. Both run against the same engine. What moved:
   Home's list index (`lists`, `.all`), a list page's rows and the
   Inbox counts (`items`), capture (`add_item`) and the tick
   (`set_done`, from every page's checkbox, since they share
   `AppModel.setDone`). `listsByDoc` / `listItems` hold `ListInfo` /
   `Item`. What settled:
   - Phase 1's Rust surface and names held; no Rust change.
   - Facade: `EngineCalls` gained the callback `run`; `EngineController`
     hands out one `EngineCalls` per engine (`calls`); `AppModel.call`
     is a write, `AppModel.read` a read that stays quiet with no
     engine, as `fetch` does. `Item` and `ListInfo` are `Identifiable`
     in `MonoplanCore`.
   - Errors: `EngineError` as is. A call with no engine fails as
     `Stopped`; reads drop it, writes say it.
   - Ordering between the two paths: each has its own serial queue, so
     only "reread in the write's completion" is ordered across them.
     That is how every write already rereads.
   - Bridges that go in phase 3: `ItemState.name` (`TypedRows.swift`)
     for `CheckboxRow` / `ItemRowMenu`, which still take the state's
     wire name from the pages on the old models; the list page's
     reorder, rename, emoji and archive and the Done page's list names
     are still `Request`s.
3. **The rest of the verbs**, area by area, Rust and iOS together:
   item detail and setters, agenda, starred, bin, done, find, places,
   list management, account, workspaces. iOS ends with no
   `MonoplanMacKit` wire types in use.
4. **macOS.** Tray onto the bindings. Delete `WebWindow.swift`, the
   scheme handler, `PageZoom`, `WindowSettings` and the web bundle step
   in `build-app.sh`; add the debug window.
5. **Cleanup.** Remove `Engine.request`, the frame `watch`,
   `subscribe`, `import`, `session`, `device_token`; `HostCmd::{Subscribe, Import, Unsubscribe, Session}`
   and the mirror fan-out in the loop; `Doc::import_mirror_update`;
   `Replica.mirror`, `Doc.loadWithPeer`, `MirrorBridge`, `mirror.html`,
   `createMirrorRuntime`, `MirrorApp`, `Session.mirror` in js; the
   Swift codec and client; the wire-shape test's Swift-facing comment.
   Mirror peer slots stop being leased. Update `apple/README.md`,
   `daemon-plan.md` and mark `mac-window-plan.md` superseded.

Phases 1 to 3 touch no macOS code and phase 4 no iOS code. The socket,
`ipc.rs` and the CLI's own request path are untouched throughout.

## Open

- iOS cannot be run here (no simulator), but it can be compiled: an
  unsigned device build type-checks and links the whole app,
  `xcodebuild -project apple/MonoplanIOS/Monoplan.xcodeproj -scheme Monoplan
  -destination 'generic/platform=iOS' -derivedDataPath <scratch>
  CODE_SIGNING_ALLOWED=NO build` (used for phase 2; delete the derived
  data after). Behaviour is still verified by the user on a device.
- `AgendaWidget` reads a snapshot file, not the engine; left alone
  unless its model type goes away with the JSON models.
- Whether the typed layer later moves out of the cli crate into an
  engine crate that both `cli` and `ffi` depend on. Not needed for any
  of the above.
