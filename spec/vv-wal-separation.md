# Plan: separate local WAL from VV-based sync

**Status: Implemented.** Landed across `core/` (Doc VV cursors, storage trait, sync engine), `crates/storage-sqlite`, `crates/protocol` (`PushBlob`/`PushAck`), `server/` (push_id dedup), `cli/`, and the web stack (`core/web`, `js/core`, `js/web`). The test list below is covered by `core/src/sync.rs`, `crates/storage-sqlite/tests/wal.rs`, `server/tests/sync.rs`, and `js/core/test/*`. Terminology note: the implementation renamed the capture cursor to `last_persisted_vv` (per §Core model) and the local snapshot trigger is `snapshot_if_wal_exceeds(max_rows, max_bytes)` / `force_snapshot`.

  ## Objective

  Refactor local persistence so that:

  - Snapshot + WAL provide local crash recovery.
  - server_known_vv determines what needs uploading.
  - Local snapshots occur regardless of online/sync status.
  - Server seq remains the opaque delivery, resume, and compaction coordinate.
  - E2EE and the dumb-server architecture remain intact.

  ## Core model

  Persist per document:

  - One encrypted full-history Loro snapshot.
  - A bounded encrypted WAL containing updates applied after that snapshot.
  - server_known_vv: the Loro operations proven to exist in the server update stream.
  - last_acked_server_seq: the durable server-log delivery frontier.
  - At most one durable in-flight push containing:
      - push_id
      - encrypted update blob
      - from_vv
      - to_vv

  The in-memory WAL capture cursor is last_persisted_vv. It advances after an update is durably appended to the
  WAL, not after server acknowledgement.

  ## Local write path

  For a local mutation:

  1. Commit the Loro mutation.
  2. Export updates after last_persisted_vv.
  3. Encrypt and append the update to the WAL.
  4. Advance last_persisted_vv.
  5. Do not modify server_known_vv.
  6. If the WAL crosses the snapshot threshold, schedule/write a local snapshot.

  For a remote server update:

  1. Decrypt and validate/decode the Loro update.
  2. Apply it to the document.
  3. Append the encrypted server blob to the WAL with its server_seq.
  4. Merge the update’s proven Loro ranges into server_known_vv.
  5. Persist the WAL append and VV advancement atomically.
  6. Only after durability may last_acked_server_seq advance.
  7. Count the remote WAL row toward the same snapshot threshold.

  Use the update blob’s decoded range metadata, not only ImportStatus.success. A duplicate import may be a no-op while still proving that the server possesses those operations.

  ## Local snapshot path

  Trigger when either:

  - WAL row count reaches 100 for CLI; or
  - WAL bytes exceed a configurable safety threshold; initially measure before choosing the exact value.

  For interactive/web clients, use a larger or idle-triggered threshold if snapshot export causes visible latency.

  Snapshot procedure:

  1. Ensure every current mutation has been appended to the WAL.
  2. Capture the current local_seq cutoff.
  3. Export a full ExportMode::Snapshot.
  4. Encrypt it.
  5. Atomically replace the local snapshot and delete every WAL row through the captured cutoff.
  6. Preserve server_known_vv, last_acked_server_seq, and any in-flight push.
  7. WAL entries appended after the captured cutoff must survive.

  The local snapshot intentionally contains both:

  - Unsent local operations.
  - Server-originated operations.

  That is safe because full Loro snapshots retain operation history, allowing unsent operations to be regenerated later.

  For CLI, evaluate the threshold after each command’s WAL append. Also check after boot so an interrupted previous snapshot attempt self-heals.

  For a multi-blob pull, append/apply the entire pull first and snapshot at most once after the completed batch.

  ## Outbound sync path

  After the initial server pull completes:

  1. Read the current server_known_vv.
  2. Capture to_vv = doc.oplog_vv().
  3. Export Updates(server_known_vv).
  4. If empty, do nothing.
  5. Encrypt the update.
  6. Persist { push_id, blob, from_vv, to_vv } before sending.
  7. Send the durable in-flight blob.
  8. On acknowledgement:
      - merge to_vv into server_known_vv;
      - persist the corresponding server seq;
      - clear the in-flight record.

  9. If the document advanced beyond to_vv while pushing, export the next delta afterward.

  Never assign server_known_vv = to_vv; merge it, because remote updates may have advanced other peer ranges while the push was in flight.

  ## Retry/idempotency

  Add a durable push identifier to the protocol. The current local client_op_id is not transmitted to or deduplicated by the server.

  Required behavior:

  - PushUpdates carries a push_id per blob.
  - Server stores the originating device and push_id.
  - Repeating the same push returns its original server seq without inserting another blob.
  - UpdatesAck identifies the acknowledged push_id, not only positional rows.
  - Keep deduplication records at least while the originating device frontier could still require the corresponding op.

  On reconnect, always pull before retrying the in-flight push. A pulled copy of the operation can also prove server possession, but durable push IDs should be the primary crash-
  safety mechanism.

  ## Server snapshots

  Distinguish them from local snapshots:

  - Local snapshot: current full document, including unsent work.
  - Server snapshot: exactly the state/history represented by the requested server frontier.

  A snapshot producer must not include pending local operations beyond server_known_vv. Produce the server snapshot at the frontiers corresponding to server_known_vv, rather than
  blindly snapshotting the current document.

  When bootstrapping from a server snapshot:

  1. Import it as the new local checkpoint.
  2. Set/merge server_known_vv from its exact encoded frontier.
  3. Persist its up_to_seq.
  4. Pull and WAL-log the server tail after that seq.

  ## Storage changes

  Update LocalStorage and both implementations:

  - CLI SQLite.
  - Web IndexedDB bridge.

  Add:

  - Encoded server_known_vv to per-document metadata.
  - WAL row/byte statistics.
  - Durable in-flight push storage.
  - Atomic “append remote WAL + advance server-known VV.”
  - Atomic “replace snapshot + prune local prefix.”
  - Atomic or correctly ordered VV/seq advancement.

  The old rule “pending rows cannot be pruned” should be removed. WAL rows may now be pruned whenever a full local snapshot contains them because upload is derived from Loro
  history.

  Edit the existing 001_init.sql files in place; do not add incremental migrations.

  ## Tests

  At minimum:

  1. Thousands of offline CLI mutations periodically snapshot; WAL remains bounded.
  2. Restart from snapshot + WAL preserves exact state.
  3. After pending WAL rows are folded and deleted, sync still exports every unsent operation from server_known_vv.
  4. Remote updates enter the WAL and trigger local snapshotting.
  5. Mixed local and remote history exports only operations beyond server_known_vv.
  6. A duplicate server update advances server knowledge even when Loro import is a no-op.
  7. Mutation during an in-flight push remains pending after its earlier to_vv is acknowledged.
  8. Crash before snapshot transaction commit leaves old snapshot + WAL valid.
  9. Crash after snapshot commit leaves new snapshot + surviving tail valid.
  10. Crash after server insert but before client ack retries the same push_id without a duplicate server row.
  11. last_acked_server_seq never persists ahead of its corresponding server_known_vv.
  12. A server snapshot excludes the producer’s unsent local operations.
  13. CLI SQLite and web IndexedDB satisfy identical semantics.

  ## Non-goals

  - Do not replace server seq with VVs.
  - Do not expose plaintext VVs to the server.
  - Do not introduce shallow snapshots yet.
  - Do not change peer-ID lifetime in this refactor.
  - Do not remove E2EE.
