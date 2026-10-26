# Local Storage

Per-account local persistence for the doc — the **same logical model on every client**, behind one Rust trait (`core::LocalStorage`, `core/src/storage.rs`). The model separates two concerns that used to share one mechanism (`spec/vv-wal-separation.md`):

1. **Crash recovery** is snapshot + WAL: one encrypted full-history Loro snapshot plus a bounded encrypted WAL of updates applied after it. Every commit — local mutation or applied remote update — appends a WAL row; when the WAL crosses a threshold it is folded into a fresh snapshot and the folded prefix pruned. Folding runs **regardless of online/sync status**.
2. **Outbound sync** is derived from Loro history against `server_known_vv` — the operations proven to exist in the server update stream — never from which WAL rows still exist. There is no outbox of rows; a full Loro snapshot retains operation history, so even unsent local operations survive folding and re-derive at push time.

The substrate differs by platform — **CLI: sqlite on disk; web: IndexedDB on the main thread** — but the schema, boot semantics, and fold policy are identical (the engine only ever sees the trait).

## Persisted state (per doc)

- One encrypted full-history **Loro snapshot**.
- A bounded encrypted **WAL** of updates applied after that snapshot.
- **`server_known_vv`** — encoded Loro VersionVector of operations proven to exist in the server update stream (merged from every applied server blob's declared range and every acked push's `to_vv`).
- **`last_acked_server_seq`** — the durable server-log delivery frontier (resume `PullUpdates` cursor).
- At most one durable **in-flight push**: `{ push_id, encrypted blob, from_vv, to_vv }`, written *before* the blob's first send and cleared on ack, so a crash between server insert and client ack retries the exact same `push_id` (the server deduplicates — `spec/sync-protocol.md`).

The in-memory WAL capture cursor is the doc's `last_persisted_vv`: it advances when an update is durably appended to the WAL, **not** on server acknowledgement. `pending_export` = `Updates(last_persisted_vv)` is the capture delta; `export_updates_since(server_known_vv)` is the push delta. The two cursors are independent.

## Storage substrate

- **CLI**: `SqliteStorage` (`crates/storage-sqlite`, CLI newtype in `cli/src/storage.rs`) — a file on disk under the profile dir. Same pragmas as the server (`spec/storage.md` §Sqlite settings). Writes are synchronously durable: the trait method returns only after the transaction commits.
- **Web**: `IdbStorage` (`js/core/src/storage/idb-storage.ts`) — IndexedDB on the **main thread**, behind a wasm-bindgen `EngineStorage` extern (`core/web/src/lib.rs`). The trait is synchronous but IDB is async, so `IdbStorage` keeps a synchronous in-memory mirror that the extern methods read/write immediately and flushes the real IDB transaction on a background promise chain; durability is signalled back out-of-band (`whenFlushed()` → the host's `notify_oplog_durable`) so an `Ack` isn't shipped until the bytes are on disk. Writes the trait requires to be atomic run as one queued IDB transaction spanning the involved stores. IndexedDB is **hard-required** — a session that can't open it surfaces a "Failed to start" screen rather than booting on a storage-less engine.

The engine sees a single `LocalStorage` trait; storage is mandatory — there is no storage-less engine mode.

## Schema (sqlite; the IDB stores mirror it)

```sql
CREATE TABLE docs (
  id                    BLOB PRIMARY KEY,           -- uuid v7, matches server-side docs.id
  created_at            INTEGER NOT NULL,
  last_acked_server_seq INTEGER NOT NULL DEFAULT 0, -- resume PullUpdates cursor; persisted, never derived
  server_known_vv       BLOB,                       -- encoded Loro VV; NULL = never synced
  last_sync_at          INTEGER                     -- unix millis of last ONLINE sync; observability only
);

CREATE TABLE wal (
  doc_id        BLOB    NOT NULL REFERENCES docs(id),
  local_seq     INTEGER NOT NULL,                   -- storage-assigned, dense, per doc
  server_seq    INTEGER,                            -- set on server-delivered rows; NULL for local-origin
  payload       BLOB    NOT NULL,                   -- encrypted Loro update bytes (DEK)
  payload_nonce BLOB    NOT NULL,
  created_at    INTEGER NOT NULL,
  PRIMARY KEY (doc_id, local_seq)
);
CREATE UNIQUE INDEX wal_server_seq_idx ON wal (doc_id, server_seq) WHERE server_seq IS NOT NULL;

CREATE TABLE snapshots (
  doc_id          BLOB    PRIMARY KEY REFERENCES docs(id),
  up_to_local_seq INTEGER NOT NULL,                 -- local-counter high-water at write time; NOT a replay cutoff
  payload         BLOB    NOT NULL,                 -- encrypted full-history Loro snapshot (DEK)
  payload_nonce   BLOB    NOT NULL,
  created_at      INTEGER NOT NULL
);

CREATE TABLE in_flight_push (
  doc_id        BLOB PRIMARY KEY REFERENCES docs(id),
  push_id       BLOB NOT NULL,                      -- uuid bytes; retry/idempotency key
  payload       BLOB NOT NULL,
  payload_nonce BLOB NOT NULL,
  from_vv       BLOB NOT NULL,                      -- encoded VV the delta was exported from (forensics)
  to_vv         BLOB NOT NULL,                      -- encoded oplog VV at export; merged into server_known_vv on ack
  created_at    INTEGER NOT NULL
);
```

Notes:

- `local_seq` is local bookkeeping only: append/replay order and the snapshot-fold cutoff. Not a sync coordinate.
- `wal.server_seq` exists solely for idempotent re-delivery detection (resume re-pull, broadcast overlap) — a duplicate `server_seq` returns the existing row instead of inserting a phantom. It plays no role in deciding what uploads.
- `docs.last_acked_server_seq` is the **persisted** pull cursor. Never derived from `MAX(wal.server_seq)` — that underestimates once folding prunes the rows it was read from, and overestimates past a gap.
- `docs.server_known_vv` is the upload-derivation base. The engine holds the decoded VV in memory and hands storage the full merged encoding on every advance.
- One snapshot row per doc — replaced on each fold.

## Write paths

Local mutation (engine `capture_local_ops`):

1. Commit the Loro mutation.
2. Export `Updates(last_persisted_vv)`, encrypt with the DEK.
3. Append to the WAL (`append_local_wal`).
4. Advance `last_persisted_vv` to the pre-export oplog VV.
5. `server_known_vv` is untouched.

Remote server update (engine `apply_remote_updates`):

1. Decrypt; decode the blob's **declared** operation range (`decode_import_blob_meta` → `partial_end_vv`); apply to the doc.
2. Merge the declared range into `server_known_vv` — the *declared* range, not `ImportStatus.success`: a duplicate import is a no-op but still proves the server possesses those operations.
3. Append the encrypted server blob to the WAL with its `server_seq`, persisting the advanced `server_known_vv` **atomically with the row** (`append_remote_wal`).
4. `last_acked_server_seq` advances only later, via `notify_oplog_durable`, once the host confirms durability. Because the VV persists with the row and the cursor persists after, the cursor can never run ahead of its `server_known_vv`.

## Fold (local snapshot) policy

Trigger: WAL row count ≥ threshold (CLI: 100, evaluated after every command's append and once at boot so an interrupted fold self-heals; web hot pulse: 250, plus an idle/hidden-tab full fold) **or** WAL payload bytes over a safety cap (CLI 4 MiB, web 8 MiB — initial values pending measurement).

Procedure (engine `snapshot_if_wal_exceeds` / `force_snapshot` → storage `write_snapshot`, one transaction):

1. Capture any uncommitted mutations to the WAL first (so the export provably contains every row at or below the cutoff).
2. Cutoff = current `last_local_seq`.
3. Export a full `ExportMode::Snapshot`, encrypt.
4. Atomically replace the snapshot row (recording the local-counter high-water as `up_to_local_seq`) and delete every WAL row with `local_seq ≤ cutoff`.
5. `server_known_vv`, `last_acked_server_seq`, and any in-flight push are preserved. Rows appended after the cutoff survive.

The snapshot intentionally contains both unsent local operations and server-originated operations: full Loro snapshots retain history, so unsent operations re-derive later from `server_known_vv`. **There is no "pending rows cannot be pruned" rule** — that rule belonged to the outbox model this spec replaced.

For a multi-blob pull, the host applies/appends the entire batch first and evaluates the fold at most once afterwards.

## Outbound sync

After the initial pull completes (always pull before pushing — see retry below):

1. If a durable in-flight push exists, re-send it verbatim (same `push_id`).
2. Otherwise: capture pending commits to the WAL, then export `Updates(server_known_vv)`. Empty → nothing to do.
3. `to_vv = doc.oplog_vv()` at export time. Encrypt; persist `{push_id, blob, from_vv, to_vv}` (`put_in_flight_push`) **before** the first send.
4. On the ack naming this `push_id`: **merge** `to_vv` into `server_known_vv` (never assign — remote updates may have advanced other peers' ranges mid-flight), persist the merged VV and clear the record atomically (`complete_push`), ingest the assigned seq.
5. If the doc advanced beyond `to_vv` while the push was in flight, the next delta exports immediately.

A disconnect mid-push keeps the durable record; the reconnect re-pulls, then retries the same `push_id`. The server deduplicates by `(device, push_id)` and acks the original seq, which may already be at or below the client's contiguous frontier (the pull may have delivered the op) — that is tolerated as a duplicate.

## Server snapshots vs local snapshots

- **Local snapshot** (this spec): the current full document, including unsent work. Crash-recovery baseline.
- **Server snapshot** (produced on `SnapshotRequest`): exactly the state/history the server update stream represents. The producer exports at the frontiers corresponding to `server_known_vv` (`fork_at`), never the current doc — unsent local operations must not leak into a blob other devices bootstrap from.

Bootstrapping **from** a server snapshot: apply it, merge its declared frontier into `server_known_vv` (persisted via `write_server_known_vv`), then write the local checkpoint as a fresh full export of the **merged** doc with a full-prefix prune. The server blob alone is not a valid local checkpoint — it lacks any unsent local work the pruned WAL rows carried. Then pull and WAL-log the server tail after the snapshot's `up_to_seq`.

## Replay / boot

Per doc: read the snapshot row (if any), decrypt and import; then decrypt and import every surviving WAL row in `local_seq` order; `finish_oplog_replay` once. Boot replays **every surviving row** with no cutoff — the fold already pruned exactly what the snapshot contains. The imports advance `last_persisted_vv` (via their declared ranges), so a booted doc reports `has_uncaptured_ops() == false`. `BootState` also hands back `server_known_vv`, `last_acked_server_seq`, the in-flight push, and the WAL row/byte statistics; the engine seeds from them (`seed_boot`).

The same path covers a fresh account (no rows at all), pure-WAL recovery (no snapshot yet), and the steady-state snapshot + tail case.

## Failure semantics

- **Crash during any append/fold**: each is one transaction — either fully visible or not. No torn rows.
- **Crash between WAL append and push**: the ops are in the WAL (or the snapshot); the next session derives them from `server_known_vv` and pushes.
- **Crash after `put_in_flight_push`, at any point up to the ack**: the durable record retries the same `push_id`; the server dedupes, so no duplicate server row regardless of whether the original insert landed.
- **Crash after remote-row insert but before the cursor advance**: cursor is *behind* the stored rows — never ahead. Reboot re-pulls from the lower cursor; `append_remote_wal` dedupes the re-delivered `server_seq`s, and the re-delivery re-proves `server_known_vv`.
- **Crash before a fold's transaction commits**: old snapshot + full WAL remain valid. **After** it commits: new snapshot + the surviving tail are valid.

## Why IDB on web (not sqlite / Worker / OPFS)

An earlier spike (`spike/shared-worker`) ran sqlite-wasm on the OPFS-SAH-pool VFS inside a SharedWorker, with the engine off the main thread. It worked end-to-end and was abandoned. The reasons are durable design constraints, not incidental:

1. **`createSyncAccessHandle` is `DedicatedWorkerGlobalScope`-only by spec** — not a vendor bug. A SharedWorker therefore *cannot* host OPFS-backed sqlite in any browser. ([MDN](https://developer.mozilla.org/en-US/docs/Web/API/FileSystemFileHandle/createSyncAccessHandle), [wa-sqlite #79](https://github.com/rhashimoto/wa-sqlite/discussions/79)). Don't propose this combination again.
2. **sqlite-wasm buys nothing here.** We store opaque encrypted blobs, not queryable data, so we never use SQL's query power — but we'd pay ~1 MB of bundle plus the COOP/COEP header requirement. IDB is exactly the right shape (ordered keyed store with transactions) and ships in every browser for free.
3. **The engine must stay on the main thread.** Moving it into a Worker adds a postMessage round-trip to every mutation; the lag is perceptible in tight UI loops (typing, drag-reorder). Multi-tab coherence and Argon2id-off-main are real wins but not worth that regression.
4. **The trait is the prize, not a unified storage technology.** "Same Rust engine, same boot semantics, same WAL, same fold policy" is the value. sqlite on one side and IDB on the other satisfy it identically — the engine never knows which.

### Web boot + the bytes-copy gotcha

Web boot is **host-driven in JS** and mirrors the CLI's `boot_doc` (it does *not* use `Doc.load`): `Doc.empty()` → replay the decrypted snapshot and every surviving WAL row (`bootRows`, in `local_seq` order) through `replayOplogUpdate` → call `finishOplogReplay()` once → `markPersisted()` so the capture cursor covers the replayed ops. Rebuilding after every row is forbidden: with N items and R replay rows it turns refresh into O(N×R); keeping R small is the fold threshold's job. The host then seeds the engine from `bootRows`: `setLastLocalSeq`, `seedWalStats`, `seedServerKnownVv`, and `seedInFlightPush` (if present). The resume cursor comes from `bootRows().lastAckedSeq` — the engine-persisted `docs.lastAckedServerSeq` (written via `writeAckedSeq`), **not** the `device` row and **not** derived from the WAL.

Initial attachment is not a live mutation stream. The web store materializes once from `workspaceSnapshotJson`; live bulk/opaque changes emit one `FullResync` control event and use the same one-shot materialization path.

⚠️ Any JS-side `EngineStorage` impl that **retains** wasm-passed `&[u8]` bytes must copy them on entry (`.slice()`). wasm-bindgen hands `&[u8]` as a `Uint8Array` view into wasm linear memory valid only for that synchronous call; `IdbStorage` defers the IDB write, so without a copy it persists reused/garbage memory and the next boot fails to decrypt. This cost a real bug and is invisible to synchronous mock tests — only a real browser reload catches it. This applies to payloads, VV encodings, and `pushId` bytes alike.

## Migration

Pre-release rule (see `AGENTS.md`): exactly one migration file per database, edited in place — never incremental migrations, never legacy bridges. Old-layout data is abandoned, not drained.

- **CLI**: `crates/storage-sqlite/migrations/001_init.sql` (+ the CLI's `001_cli` account table). The old outbox-era `ops` schema was replaced in place.
- **Web**: the engine stores live in the single `monoplan-web` IDB database (`docs` / `ops` / `snapshots` / `inflight`) alongside the config stores. The v9 upgrade recreates `ops` as the WAL (v8's outbox-era rows are abandoned; authed devices re-pull and the snapshot baseline carries anonymous docs) and adds the `inflight` store.

## Testing

The authoritative test list is `spec/vv-wal-separation.md` §Tests. Coverage lives in `core/src/sync.rs` (engine semantics: VV-derived export after folding, in-flight retry, duplicate-delivery proof, mixed-history deltas, server-frontier snapshots), `crates/storage-sqlite/tests/wal.rs` (bounded WAL over thousands of offline mutations, exact restart, fold/reopen consistency, idempotent remote append, scoped `complete_push`), `server/tests/sync.rs` (push_id dedup end-to-end), and `js/core/test/*` (the same contract over the `EngineStorage` mirror plus a live browser-stack e2e).

## Out of scope

- **Multi-tab coherence on web.** One engine, one tab; the `navigator.locks` single-tab gate stays.
- **Engine in a Worker.** Stays on the main thread — see *Why IDB on web* §3.
- **Argon2id off the main thread.** Solvable later with a dedicated Worker just for the KDF.
- **sqlite on web.** Not happening unless the OPFS-SAH spec changes — see *Why IDB on web* §1.
- **Shallow snapshots / peer-ID lifetime changes.** Explicit non-goals of the WAL/VV refactor (`spec/vv-wal-separation.md`).
