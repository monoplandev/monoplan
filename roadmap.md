# Roadmap
- onAuthFailed → logout() → dekVault.clear() - a little bit blunt and not quite in local-first spirit, wipe-dek-to-anonymous the wrong response

## Sync & persistence
- Report catch-up volume in `HelloAck` so clients can show progress and we can observe snapshot-vs-tail sync weight.
- `status.pending_changes` is currently bool-like; exact pending-op counting can come later by walking the Loro VV diff.
- Roll devices.json / secret.json from cli into sqlite & rename db ext to .sqlite

## Compaction *wait until sync / oplog / storage is settled
One latent thing remains, but it was already scoped out in your handoff:
  core/src/doc.rs::snapshot_blob still calls ExportMode::Snapshot (full), so the
  snapshot payload doesn't trim Loro's internal history even though the ops table does.
  That means bootstrap downloads are bigger than they need to be, but the ops-table
  storage win is fully realized. Switching to ExportMode::shallow_snapshot(frontier) means VV tracking.

## Wasm storage boundary — harden against retained `&[u8]` views (hardening, not urgent)
The `EngineStorage` extern (`core/web/src/lib.rs`) passes `ciphertext`/`nonce`/`clientOpId` to JS as
`&[u8]`. wasm-bindgen turns these into `Uint8Array` **views into wasm linear memory** (a `.subarray`
over `WebAssembly.Memory`), valid only for that one synchronous call. Any JS `EngineStorage` impl that
**retains** the bytes past the call (mirror + deferred IDB write, or shipping them later) reads corrupted
data once wasm reuses that memory — or a zero-length array if `memory.grow` detached the buffer. Symptom:
silent, intermittent, reads as data loss ("items vanished on refresh"). Invisible to synchronous unit tests
(view still valid same-tick) — only async + heap pressure exposes it. The `outbox()` *return* path is already
safe (Rust copies JS→Rust via `to_vec()`); the hazard is purely inbound Rust→JS.

Current state is correct: both impls copy on entry (`IdbStorage` always did; `MemEngineStorage` now does via
`.slice()`). But the copy is a *discipline* every future impl must remember.

Fix to make it impossible by construction: copy on the **Rust** side — in `WebStorage`, build owned
`js_sys::Uint8Array` copies (`Uint8Array::from(&slice[..])` allocates in the JS heap) and change the extern
to take those instead of `&[u8]`. Then every JS impl receives a stable owned array regardless of what it does;
the per-impl `.slice()` discipline (and the calls themselves) can go. Cost: one small heap copy per byte-arg
per call (negligible vs crypto + IDB). Before committing, verify wasm-bindgen's generated glue for the new
signature actually yields an owned copy, not another view. Cross-ref: `spec/local-storage.md`
§"Web boot + the bytes-copy gotcha".

## Native clients
- UniFFI bridge for iOS / Android over the existing `core` crate.
- Password-derivation flow exposed over the same bindings.

## Testing
- E2E gaps vs. `spec/testing.md`:
  - two long-lived clients observe live mutations and converge
  - one client mutates offline, reconnects, and the other client observes the changes
  - two clients mutate independently while offline, then reconnect and converge
- hardening pass

## Postgresql version
- ensure single snapshot per account across replicas
- ensure deletion/cleanup doesn't run too often and under contention across replicas?!
- migration strategy

## CI
- sqlite migrations

## Maybe/later
- Encoding habits?
- vi keys (as an option)
- Consider bounding sizes of client blobs (by KiB or op count)
- Multi-tab single-engine sharing via SharedWorker to avoid duplication of resources, data - while this seems like a good idea, in practice it slows things down enough it is important to have client-side optimistic changes which is of course, slightly harder than it looks
- Corruption detection - later
- fuck, when i was dragging a kanban item, i couldn't see my drop target easily in the list on the left
- Should delete from focus only delete the focus record?
- Should we set an external url as a cache to quick open?
- Emojis in other languages
- Gaming controls?
- Location based tasks
- take checkbox off kanban board - show on hover or show below card?: I tried this and there are heavy tradeoffs with every strategy i tried (hover top left & right corners), above or text move aside
- Known bug - checking and unchecking contiguous tasks allowing them to linger causes confusion
- Themes + change theme palette
- Location based tasks?
- ui idea - you should probably be able to click on the first empty slot of a list to add an item?
- Consider SVG emojis that fit the theme
- habit tracking?!
- foreign keyboard shortcuts?
- indexeddb smoke tests? do we have an in-mem storage adapter? via playwright?
- dependency/dependent links? e.g. "blocked by", "solved by", "duplicate of"
- in sidebar mode, the open item overrules the current list in history, always (it used to close on enter and push history of the list)
- show keyboard shortcuts when hovered over an item somehow?
- cancelled status (child of done?!)
- Icons for lifecycle statuses
- dnd week view
- Conduct bug hunt: fine-grained non-realtime issues (text not updating on same device in different representations and cross-device)
- In the done section, show what's a weekend vs what's a week, preferably use days of the week
- consider the problem of spaced repetition - target without explicit 'when' - i.e. how long since the last time?
- Consider blocked items..?
- bug: mark multiple items as done then mark one of those as not done and list position changes!
