# macOS window: plan

Status: decided 2026-10-29; phase 1 (tray over the FFI) built the same
day, phase 2 (the mirror on the host: `Host.subscribe` / `Host.import`,
`Doc::import_mirror_update`, `HostCmd`) built 2026-11-03, phase 3 (the
mirror in wasm and JS: `SyncEngine.mirror`, `MirrorBridge`, the app
runtime and its Vite entry) built 2026-11-04, phase 4 (the window
itself, in Swift) not built. The macOS app's window is a
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

Built 2026-11-03 (phase 2). `cli/src/daemon.rs`, `core/ffi/src/lib.rs`,
`Doc::import_mirror_update` in `core/src/doc.rs`.

- **Internal command enum.** The serve loop reads `HostCmd { Ipc(Request,
  oneshot<Reply>), Subscribe { workspace, sink, reply }, Import { id,
  bytes, reply }, Unsubscribe { id } }` from one unbounded mpsc that the
  socket connections and `daemon::LocalClient` (phase 1) both feed. The
  socket's connection task deserialises `Request`s and builds only
  `Ipc`, so the mirror is unreachable over the socket by construction,
  not by policy. `Request` and `Reply` (the serde IPC types) did not
  change. The channel went unbounded (it was bounded at 16) so a
  `Subscription`'s `Drop` can send `Unsubscribe` from any thread without
  a runtime; every client awaits its reply before its next command, so
  the queue depth is bounded by the client count anyway.
- **`LocalClient::subscribe(workspace, sink) -> Subscription`**, and on
  the FFI **`Host.subscribe(workspace, sink) -> Subscription`** (uniffi
  object with `docId()`, `peerId()`, `snapshot()`, `close()`). `sink` is
  `MirrorSink { update(bytes), stopped() }`, a uniffi callback interface
  on the FFI, a plain trait in the daemon. The seed carries the
  plaintext snapshot and the leased peer id. Dropping the
  `Subscription` (or `close`) sends `Unsubscribe`, which drops the
  subscriber record and with it the lease; the loop ending calls
  `stopped()` on every live sink first, and a doc forgotten under a
  mirror (`DocClosed`) does the same for that doc's sinks.
- **`Subscription::import(bytes)`**, on the FFI **`Host.import(subscription,
  bytes)`**. `Doc::import_mirror_update`: `import_with(bytes, "mirror")`
  under the same diff capture and translation as `apply_remote` (shared
  `import_translated`), no change to `last_persisted_vv`, returns the
  blob's declared range. The host then persists (WAL capture picks the
  ops up as uncaptured local), flushes the engines, pumps, and runs the
  usual `notify_if_changed`: socket watchers get `Changed`, mirrors get
  the update. `MIRROR_ORIGIN` is an undo-excluded prefix beside
  `remote`, `notes:` and `sweep`.
- **Fan-out.** `notify_if_changed` (after a local command, a remote
  frame, a mirror import, and on the persist tick) walks the
  subscribers: skip if `last_sent` already includes `oplog_vv()`, else
  `export(updates(&last_sent))`, deliver if non-empty, `last_sent =
  oplog_vv()`. One VV per subscriber is the whole state. After an import
  the importing subscriber's `last_sent` is merged with the blob's
  declared range, so a mirror's own ops are not echoed back to it (the
  echo would have been harmless; this just saves the bytes). A blob
  that also carries host ops the mirror had imported is fine too: the
  import is idempotent and the merge is still truthful.
- **Peer lease.** `Session::claim_peer` is `peer::claim` a second time:
  the host holds slot 0, the flock scan gives the subscriber slot 1 (or
  the next free one with two windows). The lease lives in the
  subscriber record.
- **Sink contract.** `update` and `stopped` run on the host loop's
  thread while it is delivering; a sink hands the bytes off and never
  calls `Host.import` synchronously from inside `update` (it would wait
  on the loop that is calling it).
- **Tray** (built, phase 1). `Host.request(bytes) -> bytes` and
  `Host.watch(sink)`, the socket's msgpack payloads over a function
  call. The tray's `DaemonClient` keeps its codec and swaps its
  transport; the connect, backoff and build-id dance left the app.
  Liveness comes from the sink's `stopped()` instead of a dropped
  socket. Typed uniffi request and reply enums can replace the bytes
  later if the Swift side ever wants them.

## Wasm side (`core/web`)

Built 2026-11-04 (phase 3). `core/web/src/lib.rs`;
`Doc::import_host_update` in `core/src/doc.rs`.

- `SyncEngine.mirror(doc, docId)`: no DEK, no `EngineStorage`. The
  class holds an enum (`Synced(CoreSyncEngine)` or `Mirror { doc,
  doc_id, last_sent: VersionVector, outbox }`) behind one `doc()`
  accessor, so the sixty forwarded mutation and read methods stay
  single. `isMirror()` tells them apart.
- In mirror mode `flush()` exports `updates(last_sent)` into the mirror
  outbox and advances `last_sent`; `takeMirrorOutbox(): Uint8Array |
  undefined` pops it, oldest first. `applyMirrorUpdate(bytes)` is
  `Doc::import_host_update`: `import_with(bytes, "remote")` under the
  same diff capture and translation as `apply_remote` (shared
  `import_translated`), so it queues the same AppEvents, never lands
  on the mirror's undo stack, and advances `last_persisted_vv` like a
  server frame; the engine then folds the update's declared range into
  `last_sent`, so the host's ops are never exported back to it.
  `hasUnsyncedOps()` on a mirror is "committed but not yet flushed".
- The WebSocket verbs (`handleConnected`, `handleServerBytes`,
  `popOutbox`, `popEvent`, ...) and the persistence verbs
  (`captureLocalOps`, the snapshot folds, the seeds) are no-ops in
  mirror mode; the cursors read as zero / unknown and `isOnline()` as
  false (the window's status is the host's to report).
- `Doc.loadWithPeer(bytes, peer)`: `empty_with_peer` then
  `import_oplog_updates` (the boot replay path: tagged remote, index
  rebuilt once, events cleared) then `mark_persisted`, so the seed is
  history, not undo steps and not uncaptured work, and every later
  local op carries the leased peer.

## JS side (`js/core`, `js/web`)

Built 2026-11-04 (phase 3).

- `MirrorBridge` (`js/core/src/mirror-bridge.ts`, beside `SyncBridge`)
  over a `MirrorTransport`: `subscribe(docId) → {snapshot, peer}`,
  `unsubscribe(docId)`, `send(docId, bytes)`, `onUpdate(cb)`,
  `onStopped(cb)`. `open(docId)` subscribes, builds the engine through
  the injected `createEngine(seed)` and routes it; updates that land
  while a subscribe is in flight wait and apply once the engine exists.
  Host updates apply tagged remote and pulse `onServerFrame(docId)`;
  `pumpOutbox()` drains every engine's mirror outbox to `send`, one
  import at a time, in order; `onChange` is `online` on the first open,
  `offline` once the host has stopped every doc, `drain` after each
  pump; `onStopped(docId)` relays the host's end-of-stream. No wasm
  import at load time, as for `SyncBridge`.
- `js/web/src/sync/mirrorHost.ts`: the app's transport over the WebKit
  message handler, and the host's session (`HostSessionInfo`:
  account id, email, device id, primary doc id, server URL, doc list).
  `detectAppHost()` is the presence of the handler. The contract the
  Swift side implements is in the file header and under "Swift side".
- `js/web/src/sync/mirrorRuntime.ts` (`createMirrorRuntime`): the twin
  of `createWorkspaceRuntime` with the IO removed. Same
  `createSyncedApp`, docs layer and view state (`viewState.ts`, now
  shared with the browser runtime); `setOnFlush` pumps the bridge,
  there is no `setBeforeFlush` (no capture); `drain(docId)` is what
  the bridge's frame pulse routes to; `handOff()` flushes notes bursts
  and every engine to the host when the window hides or unloads.
  `createDoc` / `joinDoc` / `deleteDoc` reject with a message (the
  DEK-touching flows, hidden in phase 4); `presence` is empty.
- `Session` gained `mirror: boolean` and `dek: Dek | null` (null only
  on a mirror session; the five browser-side uses guard it).
  `sessionFromHost(info)` builds the window's session: not anonymous,
  mirror, no key, empty `docKeys`.
- `MirrorApp.tsx` replaces `BrowserTabGate` → `App` → `BootGate`: ask
  the host for the session, `bridge.open` Home then every other doc (a
  non-Home doc that will not open is skipped, not fatal), read the
  prefs row, then `createMirrorRuntime` and `Workspace` under a
  `SessionProvider` whose `online` is the bridge's, `lastSyncAt` null
  (the host's sync status is phase 4), `logout` a console warning (the
  tray signs out).
- Entry: `js/web/mirror.html` + `src/mirror.tsx`, a second Vite input
  beside `index.html`; the browser bundle imports nothing of it. (Not
  `app.tsx`: that name collides with `App.tsx` on a case-insensitive
  filesystem.)

## Swift side (`apple/MonoplanMac`)

Phase 4, not built.

- A window controller owning the `WKWebView`, one `Host.subscribe` per
  open doc, and the bridge: sink updates go in with `evaluateJavaScript`,
  message-handler bytes go out to `Host.import`.
- The JS contract (`mirrorHost.ts`): a `WKScriptMessageHandlerWithReply`
  named `monoplan`, so `postMessage` resolves with the reply or rejects
  with the error text. Requests and replies, bytes as base64, the peer
  as a decimal string (a u64 does not fit a JSON number), the doc id
  as the `workspace` selector `Host.subscribe` resolves:
  - `{kind: "session"}` → `{accountId, email, deviceId, primaryDocId,
    serverUrl, docs: [{docId, name}]}`
  - `{kind: "subscribe", docId}` → `{snapshot, peer}`
  - `{kind: "unsubscribe", docId}` → `null`
  - `{kind: "import", docId, bytes}` → `null` (`Host.import`)
  - inbound, on `window.__monoplanHost`: `update(docId, bytes)` for
    `MirrorSink.update`, `stopped(docId)` for `MirrorSink.stopped`.
  The sink contract holds: deliver `update` to the webview
  asynchronously and never call `Host.import` from inside it.
- A `WKURLSchemeHandler` serving the Vite build out of the app bundle
  with correct MIME types, loading `mirror.html`, so wasm streams,
  `localStorage` prefs have a stable origin, and no `file://` sandbox
  rules apply.
- `api.ts` gets the server URL (`HostSessionInfo.serverUrl`) and the
  session credential at boot from the host; the host's sync status
  (online, last synced) over the same handler for the footer.

## Deferred

- Typed uniffi request and reply for the tray.
- The DEK-touching flows in the window (see decisions).
- Several windows: each is a subscriber; nothing here precludes it.
- iOS, which reuses all of this once the embedded host exists.

## Testing

- `cli/tests/daemon.rs` (`mirror_converges_both_ways_through_the_host`,
  built): a second `Doc` subscribing through the internal channel is
  seeded under slot 1 and converges after a CLI command on the socket
  and after a remote device's push; an op it commits and imports is
  visible to a CLI command, pushed (the harness server gets the blob and
  a second device pulls it), and not echoed back; a duplicate import is
  a no-op; a second mirror takes slot 2 and is seeded with everything
  so far; the slot is released when the subscription drops; a mirror
  outliving the loop hears `stopped` and its import fails `Stopped`.
  The socket cannot subscribe by construction (`Request` has no such
  frame), so there is nothing to test there.
- `core/src/doc.rs` (`mirror_import_is_uncaptured_local_and_excluded_from_undo`,
  built): the import emits the remote-style `AppEvent`s, leaves
  `last_persisted_vv` behind so the next capture exports exactly the
  mirror's op, never lands on the host's undo stack, and is idempotent.
- `core/src/doc.rs` (`host_update_into_mirror_is_remote_for_undo_and_events`,
  built): the mirror side; a host update is remote for events and undo,
  not uncaptured work, idempotent, and the mirror's own echo is a no-op.
- `js/core/test/mirror-engine.test.ts` (built): `Doc.loadWithPeer`
  binds the peer and seeds as history; mirror-mode `flush` yields bytes
  a host doc imports to fingerprint equality, one update per flush,
  queued oldest first; `applyMirrorUpdate` emits exactly the AppEvents
  `applyRemote` does for the same op and is idempotent; host updates
  and the echo never enter the mirror's undo; the WebSocket and
  persistence verbs are inert.
- `js/core/test/mirror-bridge.test.ts` (built), over
  `test/fake-mirror-host.ts` (an in-memory host that leases peers, fans
  out and deliberately echoes): both directions converge through the
  bridge, an update during an in-flight subscribe is not lost, two
  docs route independently and `close` releases one, a stopped host
  ends routing and refused imports are reported.
- `js/web/test/mirrorRuntime.test.ts` (built): `createMirrorRuntime`
  renders the host's docs, a store mutation reaches the host on the
  flush pump with no capture, a host change reaches the store through
  the frame pulse, undo reverses only the window's op, notes on the
  idle timer reach the host on hand-off, an update queued before the
  store existed is not dispatched twice; the WebKit transport's
  framing (base64 both ways, a u64 peer as `BigInt`, request order).
- `MonoplanMacKit` (phase 4): the bridge framing and the scheme
  handler's MIME map.
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
2. **Mirror on the host** (built 2026-11-03). `HostCmd`,
   `LocalClient::subscribe` / `Subscription::import` / lease / fan-out
   in `cli/src/daemon.rs`; `Doc::import_mirror_update` and
   `MIRROR_ORIGIN` in core; `Host.subscribe` / `Host.import`,
   `MirrorSink` and `Subscription` on the FFI; tests above. No Swift
   consumer yet; the generated bindings carry the new surface.
3. **Mirror in wasm and JS** (built 2026-11-04). `SyncEngine.mirror`,
   `takeMirrorOutbox`, `applyMirrorUpdate`, `Doc.loadWithPeer` and
   `Doc::import_host_update`; `MirrorBridge` in `js/core`; the WebKit
   transport, `createMirrorRuntime`, `MirrorApp` and the `mirror.html`
   entry in `js/web`; the Bun tests above. `Session.mirror` /
   `dek: Dek | null`; `createViewState` shared with the browser
   runtime. No Swift consumer yet; the browser bundle is unchanged.
4. **The window.** Window controller, scheme handler, the message
   handler and inbound global per the contract above, session and
   credential injection into `api.ts`, the host's sync status in the
   footer, hidden DEK-touching flows (`Session.mirror`).
5. Amend `daemon-plan.md` "Process topology" and `apple/README.md` as
   each lands.
