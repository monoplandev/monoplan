# Typed Swift bindings (plan, not built)

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
  the types for Swift (`ItemState`, `ItemRow`, `PlaceRow`, `AgendaDay`).

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

pub fn items(cx: &Ctx, list: &str, include_done: bool) -> Result<Vec<ItemRow>, ApiError>;
pub fn add_item(cx: &Ctx, list: &str, text: &str, top: bool) -> Result<String, ApiError>;
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
- `ApiError`: `Stopped`, `NoSuchWorkspace`, `NotFound { what, id }`,
  `Invalid { message }`, `Failed { message }`. Start with `Failed` for
  everything that is `anyhow` today and promote a case to its own
  variant when Swift needs to branch on it.

### Reaching the loop

One new `HostCmd` variant carrying a closure, instead of a typed
request and reply enum per verb:

```rust
HostCmd::Call {
    workspace: Option<String>,
    run: Box<dyn FnOnce(Result<&Ctx, ApiError>) + Send>,
}
```

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

- `sync()`, `reconnect()` → `SyncStatus`
- `account()` → `Account?`
- `create_workspace(name)`, `delete_workspace(doc_id)`
- `observe(observer)` → `Watcher`, with
  `trait EngineObserver { fn changed(); fn sync(status: SyncStatus); fn stopped(); }`
  replacing the msgpack frame sink. Same forwarder thread as `watch`.

Calls stay blocking; Swift puts them on a queue. uniffi async is not
needed for this.

## Swift side

- `MonoplanCore` gets a thin async facade over `Engine` (a serial
  queue, `async throws` methods) and the observer bridged to an
  `AsyncStream` or a callback on the main actor. This replaces
  `EngineClient` and both `FFITransport`s.
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

1. **Rust foundation and one slice.** `api::{lists, items}` with
   `lists`, `items`, `add_item`, `set_done`; `HostCmd::Call` and the
   shared tail; the `uniffi` feature; `Engine.lists/items/add_item/set_done/observe`;
   build script handles multiple generated files. `item ls` / `item add`
   / `list ls` render from the api rows. Check: `cargo test` (CLI JSON
   unchanged), `bun run build:apple`, a `MonoplanCore` Swift test that
   starts a local-only engine and round-trips an item.
2. **iOS slice.** The Home list screen reads and writes through the
   new calls; everything else stays on `EngineClient`. Both run against
   the same engine. This is where naming, error shape and the facade
   get settled; expect to revise phase 1.
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

- iOS cannot be built here (no simulator). Phases 2 and 3 are verified
  by `swift build` on the shared packages and by the user building in
  Xcode; whether `xcodebuild -destination generic/platform=iOS` works
  as a compile check without signing is untested.
- `AgendaWidget` reads a snapshot file, not the engine; left alone
  unless its model type goes away with the JSON models.
- Whether the typed layer later moves out of the cli crate into an
  engine crate that both `cli` and `ffi` depend on. Not needed for any
  of the above.
