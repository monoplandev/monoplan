# macOS window: plan

Status: decided 2026-10-29; phase 1 (tray over the FFI) built the same
day, phases 2 to 4 not built. The macOS app's window is a
webview running the web client (`js/web`) over a **mirror**: a second
Loro doc in the webview, with a peer id of its own, seeded from the
app's in-process host and kept converged with it by exchanging Loro
update bytes across the Swift FFI. Nothing in the web store changes;
only its IO does. The socket (`spec/daemon-plan.md` "IPC protocol")
stays the CLI's path and never carries the mirror.

Companion to `spec/daemon-plan.md` (the host, amended 2026-10-27 and
2026-10-29), `spec/architecture.md` (sans-IO engine, host
responsibilities), `spec/peer-id-plan.md` (the slot lease the mirror
borrows), `apple/README.md` (the FFI build pipeline and the tray).
Reference code: `js/web/src/sync/runtime.ts` and `store.ts` (what the
web store expects of its engine), `core/web/src/lib.rs` (`SyncEngine`,
the wasm class the store drives), `cli/src/daemon.rs` (`Pending`, the
internal request channel the socket handler already sits on),
`core/ffi/src/lib.rs` (`Host`).

## Decisions in one screen

| Question | Decision |
|---|---|
| Native or webview? | **Webview.** The web client is the UI, on macOS as in the browser. SwiftUI is for later, once the UI has settled under real users; building it now would mean rebuilding a UI that is still moving. |
| View or mirror? | **Mirror.** A *view* makes the host the view model: it pushes state out, the webview renders it, every action goes back as an intent. Either flavour (Swift-held projections, or the web store's read half fed by host AppEvents) rebuilds the store's write half on the host: batching, undo grouping, lifecycle cascades, Focus auto-remove, draft ids, the notes bridge. That is the code with the most behaviour in it and it would then exist twice, once per client, with every feature landing in both. The mirror keeps one code path. See "Why not a view". |
| Transport | **The FFI, in process, always.** `Host` (`monoplan-ffi`) grows `subscribe` and `import`. The socket is served for the CLI only; the headless `monoplan daemon` is the fallback host for Linux, servers and CLI-only Macs and never learns the mirror. The tray moves to the FFI first (phase 1), so the app has one client path. |
| Wire | **Loro bytes, plaintext, three verbs.** Snapshot in (`export_snapshot`), updates in, updates out (`export(updates(vv))`, `import_with`). No envelope, no seqs, no acks, no encryption: the sync protocol exists to survive a network and an untrusted server, and the mirror has neither. |
| The mirror's peer id | **Its own**, leased by the host from the slot pool (`peer::claim`, a second flock in the same process takes the next slot) for the life of the subscription. Two writers on one peer would mint duplicate op ids. With its own peer the mirror is an honest concurrent writer and merges with the host like a second device would. VV width grows by one slot, which the pool already bounds by historical concurrency. |
| What the webview holds | **No storage, no DEK, no WebSocket, no login.** Close the window and the mirror is gone; reopen and it reseeds from a fresh snapshot. The host is the durable copy. |
| Durability of a window edit | The mirror hands its ops to the host on every store flush, the same cadence that reaches IndexedDB in the browser. The host WALs them on its capture tick. The window is lost between a flush and the host's import only if the process dies in that microsecond, which is the same window the browser has between a commit and its IDB write. |
| Engine surface | **One wasm class.** `SyncEngine` gains a mirror constructor (no DEK, no storage). The store's sixty engine calls and `createSyncedApp` are untouched; `runtime.ts` picks a bridge. |
| Host-side undo | Mirror ops are imported tagged, like remote ops, so `monoplan undo` in the terminal never undoes a window edit, as it never undoes another device's. The window's undo is its own Doc's `UndoManager`, as in the browser. |
| HTTP | The non-doc surface (`api.ts`: devices, members, invites, doc list) keeps calling the server. The app injects the session credential the CLI already holds; the webview never logs in. |
| DEK-touching flows | **Hidden in the window on the first pass**: password change, recovery, doc creation, invite accept (each wraps or unwraps a key the webview does not hold). The tray, Settings and the CLI keep them. |
| iOS | Same shape when it comes: the iOS app embeds core as a host (`daemon-plan.md` "Mobile hosts") and a webview mirrors it through the same FFI. The FFI surface stays small for as long as the UI is web. |

## Why not a view

The 2026-10-21 reasoning in `daemon-plan.md` already rejected typed
projections plus per-view events for live clients: it makes the protocol
the view model and needs incremental-apply code in every GUI. The mirror
it described was read-only, with mutations as intents over the socket,
because its consumers were going to be native views over the FFI. A
webview running `js/web` inverts the trade-off: the store is built on a
locally mutated Doc. Every mutation calls the wasm engine synchronously
and drains AppEvents on the same tick; undo is the Doc's `UndoManager`;
the notes bridge subscribes to the Doc's `LoroText`; the search index
rebuilds from the Doc's events. A read-only mirror would need an intent
enum mirroring the whole core mutation surface, a host-side undo, async
mutation plumbing through the store, and a round trip per keystroke in
notes. The spec called a mirror with its own peer id "the escape hatch,
not the default". For the webview it is the default: latency is the
reason to prefer it, and it is the only shape that leaves the store
alone.

## The mirror

```
 ┌──────────────── app process ─────────────────────────────────────┐
 │  Rust host thread                        WKWebView (JS + wasm)   │
 │  ┌────────────────────────┐              ┌─────────────────────┐ │
 │  │ authoritative Doc      │  snapshot    │ mirror Doc          │ │
 │  │ peer slot 0            │ ───────────► │ peer slot 1         │ │
 │  │ sqlite WAL, DEK,       │  updates     │ no storage, no DEK  │ │
 │  │ WebSocket to server,   │ ───────────► │ UndoManager, notes, │ │
 │  │ serve loop             │  updates     │ search, AppEvents   │ │
 │  │                        │ ◄─────────── │ → the web store     │ │
 │  └───────┬────────────────┘   (FFI)      └─────────────────────┘ │
 │          │ socket: CLI only                                      │
 └──────────┼──────────────────────────────────────────────────────┘
      monoplan CLI                 server ◄── encrypted blobs, one device
```

Three flows and nothing else:

1. **Seed.** On subscribe the host leases a peer slot, exports a
   plaintext snapshot of the workspace's doc, and returns both. The
   webview loads the snapshot into a fresh wasm Doc under that peer.
2. **Host to mirror.** After every apply on the host, whatever its source
   (a CLI command, a remote frame, another subscriber's import, the
   mirror's own import coming back round), the host exports the updates
   since what it last sent this subscriber and delivers them through the
   subscriber's sink. The mirror imports them tagged `remote`. Loro skips
   ops the mirror already has, so its own edits echo harmlessly, and the
   tag keeps host-side changes out of the mirror's undo stack.
3. **Mirror to host.** The store mutates the mirror Doc exactly as in the
   browser and the UI updates on the same tick. On flush the engine
   exports the mirror's ops since its last send and the bridge hands
   them to `Host.import`. The host imports them tagged `mirror`, leaving
   `last_persisted_vv` alone, so its capture tick sees them as
   uncaptured local ops: WAL row, encrypt, push, like anything the CLI
   committed.

What the mirror is not: a device (the server sees one device for the
Mac), durable, a key holder, or a second sync engine.

## Host side (Rust)

- **Internal command enum.** The serve loop reads `Pending =
  (Request, oneshot<Reply>)` from an mpsc that the socket connections
  and `daemon::LocalClient` (phase 1) both feed. For the mirror it
  becomes `HostCmd { Ipc(Request, oneshot<Reply>), Subscribe { .. },
  Import { .. }, Unsubscribe { .. } }`. The socket handler can only
  construct `Ipc`, so the mirror is unreachable over the socket by
  construction, not by policy. `Request` and `Reply` (the serde IPC
  types) do not change.
- **`Host::subscribe(workspace, sink) -> Subscription`** (uniffi object).
  `sink` is a uniffi callback interface with `update(bytes)` and
  `stopped()`. The reply carries the snapshot bytes and the peer id.
  Dropping the `Subscription` (or `Host` shutting down) releases the
  slot lease and removes the subscriber.
- **`Host::import(subscription, bytes)`.** Applies a plaintext update
  from the mirror's peer. New `Doc` method, `import_mirror_update`:
  `import_with(bytes, "mirror")`, no change to `last_persisted_vv`, then
  the usual AppEvent drain so socket watchers get `Changed` and other
  subscribers get the update.
- **Fan-out.** Where the loop sends `WatchEvent::Changed` today (after a
  local command, after a remote frame), it also walks the subscribers:
  `export(updates(&last_sent))`, deliver if non-empty, `last_sent =
  oplog_vv()`. One VV per subscriber is the whole state.
- **Peer lease.** `peer::claim` a second time: the host holds slot 0, the
  flock scan gives the subscriber slot 1 (or the next free one with two
  windows). Lease lives in the subscriber record.
- **Tray** (built, phase 1). `Host.request(bytes) -> bytes` and
  `Host.watch(sink)`, the socket's msgpack payloads over a function
  call. The tray's `DaemonClient` keeps its codec and swaps its
  transport; the connect, backoff and build-id dance left the app.
  Liveness comes from the sink's `stopped()` instead of a dropped
  socket. Typed uniffi request and reply enums can replace the bytes
  later if the Swift side ever wants them.

## Wasm side (`core/web`)

- `SyncEngine::mirror(doc, doc_id)`: no DEK, no `EngineStorage`.
  Internally the class holds an enum of the synced engine or a mirror
  engine with one shared doc accessor, so the forwarded mutation methods
  stay single.
- In mirror mode `flush()` exports updates since the last send into an
  outbox read by `takeMirrorOutbox(): Uint8Array | undefined`;
  `persistLocal()` is a no-op; `applyMirrorUpdate(bytes)` imports tagged
  `remote` and queues AppEvents the way `apply_remote` does.
- `Doc.loadWithPeer(bytes, peer)`: load the snapshot, then set the peer
  id before any local op.

## JS side (`js/web`)

- `MirrorBridge` beside `SyncBridge`, same callbacks (`onServerFrame`,
  `onChange`, `pumpOutbox`), over a transport interface: `subscribe(docId)
  → {snapshot, peer}`, `send(docId, bytes)`, `onUpdate(cb)`. The app
  implementation posts to a WebKit message handler and receives through
  a global the Swift side calls. Bytes cross as base64.
- `runtime.ts` picks the browser stack (IndexedDB, DEK vault, WebSocket
  bridge, login) or the app stack (mirror) from the presence of the
  message handler. The app stack skips the IDB boot gate entirely: the
  snapshot is the boot.
- Session comes from the host (account id, email, device id, the doc
  list), not from `Login.tsx`.
- Same source, a second Vite entry for the app, so the browser bundle is
  untouched.

## Swift side (`apple/MonoplanMac`)

- A window controller owning the `WKWebView`, one `Host.subscribe` per
  open doc, and the bridge: sink updates go in with `evaluateJavaScript`,
  message-handler bytes go out to `Host.import`.
- A `WKURLSchemeHandler` serving the Vite build out of the app bundle
  with correct MIME types, so wasm streams, `localStorage` prefs have a
  stable origin, and no `file://` sandbox rules apply.
- `api.ts` gets the server URL and the session credential at boot from
  the host.

## Deferred

- Typed uniffi request and reply for the tray.
- The DEK-touching flows in the window (see decisions).
- Several windows: each is a subscriber; nothing here precludes it.
- iOS, which reuses all of this once the embedded host exists.

## Testing

- `cli/tests/daemon.rs`: a second `Doc` subscribing through the internal
  channel converges after a CLI command and after a remote frame; an
  import from the subscriber's peer is WALed and pushed (the harness
  server sees the blob); the subscriber's own ops echoing back are a
  no-op; the slot lease is released on drop; the socket cannot subscribe.
- `core/web` under Bun: mirror-mode `flush` yields bytes a second Doc
  imports to equality; `applyMirrorUpdate` emits the same AppEvents as
  `apply_remote`; imported ops are excluded from undo.
- `MonoplanMacKit`: the bridge framing and the scheme handler's MIME map.
- Browser or app smoke only when asked.

## Phases

0. **Spec.** This file; amendments to `daemon-plan.md`, `cli.md`,
   `apple/README.md`, `CLAUDE.md`.
1. **Tray over the FFI** (built 2026-10-29). `daemon::Serving` wires
   the loop once and hands out `daemon::LocalClient`, the in-process
   twin of a socket connection; `monoplan-ffi` exposes it as
   `Host.request` (msgpack payload in, payload out, blocking) and
   `Host.watch` (a `HostSink` callback: `Watching`, then `Event`s,
   `stopped` when the loop ends). `DaemonClient` speaks the same
   payloads over a `HostTransport` protocol; the app's is
   `FFITransport`, tests use an in-memory one. The client no longer
   connects, backs off or hellos a socket: it asks `HostController`
   for a host (`HostProvision`) and hears the loop end through the
   watch. Tested in `cli/tests/daemon.rs`
   (`local_client_shares_the_loop_with_the_socket`) and
   `DaemonClientTests`. `HostCmd` is deferred to phase 2, where the
   first variant the socket must not reach appears. Socket untouched.
2. **Mirror on the host.** `subscribe` / `import` / lease / fan-out,
   `Doc::import_mirror_update`, tests.
3. **Mirror in wasm and JS.** Mirror mode, `MirrorBridge`, app runtime,
   Bun tests.
4. **The window.** Scheme handler, bridge, session injection, hidden
   flows.
5. Amend `daemon-plan.md` "Process topology" and `apple/README.md` as
   each lands.
