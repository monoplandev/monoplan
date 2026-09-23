# Sync Protocol

WebSocket per device. Frames are binary, MessagePack-encoded (`rmp-serde`). Each frame is a tagged enum (serde-internal-tag style) so the discriminator is part of the encoded value.

## Terminology

**Status:** Implemented.

The wire calls them "ops" (`PushOps`, `OpsAck`, `seq`, `since_seq`, `last_acked_seq`) but each one is an **encrypted op blob** — a single `EncryptedBlob` carrying a Loro update-pack that contains *1..N* CRDT operations. The client engine derives one blob per push cycle (`Updates(server_known_vv)` — everything the server has no proof of, see `spec/vv-wal-separation.md`), so "1 seq" on the server ≈ "one push cycle" on the client, not "one user action".

Consequences worth remembering:

- Server-assigned `seq` enumerates *blobs*, not user actions. A user typing 1000 characters in one session and pressing flush once = 1 seq.
- `snapshot_threshold_blobs` (below) is in blob count, not action count or bytes.
- Compaction by `seq ≤ snapshot.compaction_floor_seq` deletes *whole blobs* — a single fat blob is uncompactable.
- `PushOps { ops: [PushBlob] }` accepts a vector, so the wire supports >1 blob per push even though the current engine sends one.

A future move to byte-based eligibility / compaction would close the asymmetry; tracked in `roadmap.md`.

## Per-account `seq`

**Status:** Implemented.

Server-assigned `seq` is **per-account** and **dense / gap-free**. The server bumps a per-account counter (`account_sequences.next_seq` in sqlite; the same shape in Postgres) in the same transaction that inserts ops, so consecutive ops for one account get consecutive seqs (1, 2, 3, …). Different accounts have independent counters.

## Version handshake

**Status:** Implemented.

First frame on every WS connection, before any payload exchange:

- Client → Server: `Hello { client: "monoplan-cli", client_version: "0.1.0", supported_protocol_versions: [1] }`
- Server → Client: `HelloAck { server_version: "0.1.0", protocol_version: 1 }` — server picks the highest version it shares with the client. If no overlap → `HelloRejected { reason }` and connection closes.

All subsequent frames are interpreted under the agreed `protocol_version`. Belt-and-braces against breaking changes; MessagePack handles additive evolution within a version on its own.

Note: the per-list order-container CRDT schema (`spec/data-model.md` "Schema versioning & compatibility") shipped without a wire bump — the frames are identical and op blobs are opaque to the protocol. The doc layouts are mutually incompatible, but pre-release with a single user the cutover is handled operationally (export JSON → wipe → import), not fenced at the handshake. If a schema break ever needs to coexist with live old clients, bump this version then.

## Client → Server

**Status:** Implemented.

| Type | Body | Purpose |
|---|---|---|
| `PushOps` | `{ ops: [PushBlob] }` | Append ops. Server assigns per-account seqs. `PushBlob = { push_id: uuid, blob: EncryptedBlob }` — see §Push identity & retry. |
| `PullOps` | `{ since_seq: u64 }` | Request ops with seq > since_seq. |
| `Ack` | `{ last_acked_seq: u64 }` | Advance this device's contiguous-prefix frontier. |
| `PushSnapshot` | `{ up_to_seq: u64, compaction_floor_seq: u64, blob: EncryptedBlob }` | In response to `SnapshotRequest`. `up_to_seq` = encoded state frontier; `compaction_floor_seq` = echo of the server's requested compaction floor (server-side op-blob GC bookkeeping; does not affect the produced blob). |
| `PullSnapshot` | `{}` | Request the latest snapshot blob. |

## Server → Client

**Status:** Implemented.

| Type | Body | Purpose |
|---|---|---|
| `OpsAck` | `{ acks: [PushAck] }` | Response to `PushOps`. One `PushAck = { push_id: uuid, seq: u64 }` per pushed blob, in input order — self-describing across retries (see §Push identity & retry). |
| `OpsBatch` | `{ ops: [(u64, EncryptedBlob)], complete: bool }` | Response to `PullOps`; may chunk. Each tuple's `u64` is the per-account `seq`. |
| `OpsBroadcast` | `{ ops: [(u64, EncryptedBlob)] }` | Pushed when another device sends ops. |
| `SnapshotRequest` | `{ up_to_seq: u64, compaction_floor_seq: u64 }` | Server asks a connected client to produce a snapshot. `up_to_seq` = state frontier to encode at (= the producer's `last_acked_seq`); `compaction_floor_seq` = seq at/below which op blobs become eligible for GC once this snapshot lands (= `max(horizon, prev compaction_floor_seq)`). |
| `Snapshot` | `{ up_to_seq: u64, blob: EncryptedBlob }` | Response to `PullSnapshot`. `up_to_seq` is the encoded state frontier; the bootstrapping client uses it as its next `since_seq`. |
| `SnapshotRequired` | `{ up_to_seq: u64 }` | Sent in lieu of `OpsBatch` when the client's `since_seq` is below the latest snapshot's `compaction_floor_seq` — server can't serve the missing ops. Client must bootstrap from snapshot before resuming ops. `up_to_seq` is informational; the authoritative state frontier is the one returned in `Snapshot`. |

`EncryptedBlob = { nonce: bytes, ciphertext: bytes }`.

## Push identity & retry

**Status:** Implemented.

Every pushed blob carries a client-minted **`push_id`** (uuid, 16 raw bytes under MessagePack). It is the crash-retry idempotency key:

- The client persists `{push_id, blob, from_vv, to_vv}` durably **before** the first send (`spec/local-storage.md` §Outbound sync). At most one push is in flight per doc at a time.
- The server stores `(device_id, push_id)` on the inserted op row. A repeat of an already-inserted `(device, push_id)` returns the **original** seq in `OpsAck` without inserting a second row, and is **not** re-broadcast to peers (they received it on the original insert).
- On reconnect the client always pulls before retrying the in-flight push. The pull may deliver the client's own op back (proving server possession via the blob's declared Loro range); the retry is still sent and deduplicated — durable push ids are the primary crash-safety mechanism, pulled copies are corroboration.
- A deduplicated ack's seq may be at or below the client's contiguous frontier (the pull already covered it); the client treats that as a duplicate seq, not an error.
- Dedup records live on the op rows themselves, so they persist until compaction — and compaction only deletes rows below the horizon, i.e. rows every device (including the originator) has acked past, at which point the originator can no longer legitimately retry that push id.

## Ordering & ack flow

**Status:** Implemented. `last_contiguous_seq` advances only over the contiguous next seq. Because per-account seqs are dense and gap-free at the server and delivered in order over a single connection, that always equals the max seq seen — so no reorder buffering or gap detection is needed. (A forward gap is structurally impossible here; the engine `debug_assert!`s against one rather than carrying recovery machinery for a case that can't arise under this deployment.)

- Server orders ops by per-account `seq` of arrival. Per-account FIFO is the only ordering that matters — different accounts' streams are independent.
- Client decrypts ops and applies via Loro **immediately on receipt**, regardless of seq contiguity. Server `seq` is a *delivery* sequence, not a *causal* one; Loro's CRDT handles real causal ordering off the encoded VV. Applying out-of-order is safe and keeps the UI responsive under replica-lag conditions.
- Client sends `Ack { last_acked_seq: last_contiguous_seq }` — the contiguous prefix from the persisted start. Server stores in `devices.last_acked_seq`.
- **Horizon** = `min(last_acked_seq)` across all non-revoked devices for the account. Drives the **op-blob compaction floor** (`compaction_floor_seq`): every device has every op blob at or below the horizon, so those blobs are safe for server-side GC. This is purely seq-level bookkeeping — the server doesn't see Loro VVs and doesn't need to. Time-based eviction is deliberately *not* used to advance the horizon: deleting blobs a stale device hasn't acked would force a full bootstrap on its next reconnect even when the lag is small. The user-facing escape hatch is explicit revoke via `DELETE /api/devices/:id` — revoking a stale device immediately drops it from the horizon calc, unblocking compaction. (Loro-level *shallow snapshotting* — trimming CRDT history rather than op blobs — is a separate, future concern; see §"Shallow snapshots (future)".)
- **`OpsBroadcast` is post-commit only.** Server fans out to other devices only after the originating `PushOps` is durable in storage. Otherwise a crash between broadcast and fsync could leave peers holding ops the sender will re-push (under new seqs) on reconnect.

## Commit origin tagging

**Status:** Implemented. `apply_remote` and oplog replay set origin `"remote"`; notes delta writes set `"notes:<item id>"`; `UndoManager` excludes both prefixes.

Every Loro commit carries an origin string. The engine uses three values:

- **`""`** (Loro default) — local mutations from the user. No explicit tag needed; `LoroDoc::commit()` already passes empty.
- **`"remote"`** — ops applied via `apply_remote()` after decrypting an inbound `OpsBatch`/`OpsBroadcast`. Set with `LoroDoc::import_with(bytes, "remote")`.
- **`"notes:<item id>"`** — `Doc::apply_notes_delta`, the open notes editor's per-keystroke writes (`spec/notes-plan.md` Phase 2). Excluded from workspace undo so typing never lands on the undo stack; the editor's own history owns notes undo while it is focused, and the web store records one session-level step at blur (`spec/notes-plan.md` "Undo consequence"). Clients write all notes this way, whole-value writes included, so no notes commit carries the default origin.

This exists so a future `UndoManager` can `exclude_origin_prefixes(["remote"])` and undo only the local user's edits, not concurrent remote ones. Origins are not synced; they're a local-only event filter (cf. Loro `set_next_commit_origin` docs). New non-local sources (snapshot bootstrap replay, schema migrations, etc.) get their own prefix as they appear — keep them disjoint from `"remote"` so undo policy can target them independently.

## Backpressure

**Status:** Implemented. Chunk caps (500 ops / 256 KiB) are enforced in `server::sync::queries::fetch_ops_batch`; ack decoupling is the engine's normal behavior.

`OpsBatch` chunks are sent **fire-and-forget** — the server emits them back-to-back without waiting for client acknowledgement. WebSocket sits on TCP, which has its own flow control: if the client can't drain its receive buffer fast enough (slow decrypt, low memory, paused tab), the TCP window closes and the server's `send` blocks. No application-level pacing required. This avoids paying RTT × chunk-count on catch-up — for a 10-chunk catch-up at 100 ms RTT, request-ack-request would burn 1 s of pure waiting; fire-and-forget burns 0.

The application-level `Ack { last_acked_seq }` is **decoupled from chunk delivery**. It's emitted by the client *after* successfully applying ops to the local Loro doc, not on receipt of bytes. Server uses it only for horizon tracking; it does not gate further sends. This decoupling makes resume-after-disconnect correct: the client persists `last_acked_seq` only after Loro accepts the op, so reconnecting with `since_seq = last_acked_seq` replays from the last *applied* op, not the last *delivered* one. No double-apply, no skipped ops.

The `complete: bool` on `OpsBatch` signals "no more chunks for this `PullOps`" — the client flushes its progress UI and considers the pull finished.

**Chunk size:** **500 ops or 256 KiB per `OpsBatch`, whichever hits first.** Sized to single-frame the common case (a session's worth of ops, or hours-away reconnect), give chunked progress UX for catch-up (thousands of ops → 5–10 frames), and stay well under WS frame caps that intermediaries enforce (~1 MiB at common proxies). The byte cap is a safety net against a future op type larger than expected.

## Snapshot orchestration

**Status:** Implemented. `SnapshotCoordinator` enforces the lease/trigger model; `push_snapshot` in `ws.rs` accepts only matching in-flight requests; opportunistic compaction runs after each accepted snapshot.

Snapshotting is **server-orchestrated, best-effort, and must never wedge sync**. It exists to bound bootstrap / compaction cost, not to gate normal operation.

A snapshot carries **two seqs** that serve unrelated jobs:

- **`up_to_seq`** — the snapshot's encoded *state* frontier. Determines what current state a bootstrapping client sees, and the value the client uses as `since_seq` for the post-bootstrap `PullOps`. Wants to be as recent as possible for bootstrap perf — ideally equal to the producer's `last_acked_seq` (and hence equal to `server_last_seq` when the producer is fully caught up).
- **`compaction_floor_seq`** — the seq at/below which op blobs become eligible for server-side GC once this snapshot lands. Server-only bookkeeping; the producing client echoes it verbatim and the produced blob's contents do not depend on it. Doubles as the **bootstrap gate**: a `PullOps { since_seq }` with `since_seq < compaction_floor_seq` is answered with `SnapshotRequired` because the requested ops may no longer exist. Monotonic across snapshots — never regresses (see below).

> Note: today's "snapshot" is a *full* Loro snapshot — no CRDT history is trimmed. The two seqs are about op-blob storage on the server, not Loro shallow-snapshot semantics. True shallow snapshotting (history trimming, with a VV horizon contributed by clients) is tracked in §"Shallow snapshots (future)".

Orchestration:

- Trigger: snapshot when **both**
  - `(server_last_seq − latest_snapshot.up_to_seq) > snapshot_threshold_blobs` (default `500`, configurable via `snapshot_threshold_blobs` / `MONOPLAN_SNAPSHOT_THRESHOLD_BLOBS`) — enough new state has accumulated that a new snapshot materially shortens a bootstrapping client's `PullOps` catch-up, and
  - the triggering device is caught up to `server_last_seq` — that's what we set `up_to_seq` to, so the producer must be at that point to encode it. Lagging connections are skipped as producers but still contribute to horizon.
  Counts blobs, not user actions or bytes — see §"Terminology" for why that matters.
  Horizon is intentionally **not** a trigger condition. Snapshotting is valuable for bootstrap perf independent of compaction — a single snapshot row replaces an arbitrarily long `OpsBatch` replay. If horizon hasn't moved, the new snapshot's `compaction_floor_seq` is the same as the previous one, so compaction doesn't advance — but the new snapshot still cuts bootstrap cost.
- `up_to_seq` = the producer's `last_acked_seq` = `server_last_seq` (since the producer is caught up).
- `compaction_floor_seq` = `max(horizon, previous_snapshot.compaction_floor_seq)`. Normally just horizon; the `max` enforces monotonicity for the edge case where a fresh device's join drops horizon below the existing floor — compaction is one-way, so the floor stays put.
- Production: server sends `SnapshotRequest { up_to_seq, compaction_floor_seq }`. The client exports a full Loro snapshot **at the frontiers of its `server_known_vv`** (`fork_at`) — exactly the state/history the server op stream represents, never the current doc, which may contain unsent local operations that must not leak into a blob other devices bootstrap from (`spec/vv-wal-separation.md` §Server snapshots). It encrypts with the DEK and uploads via `PushSnapshot { up_to_seq, compaction_floor_seq, blob }`, tagging `up_to_seq` with its `last_contiguous_seq` (the seq-coordinate twin of `server_known_vv`). `compaction_floor_seq` is echoed unchanged — the client does not interpret it.
- Compaction: after a snapshot is durable, compaction may delete ops with `seq ≤ snapshot.compaction_floor_seq`.

Server keeps **at most one in-flight snapshot request per account**:

- State:
  - `Idle`
  - `Requested { device_id, up_to_seq, deadline }`
- Transition to `Requested` only when no request is already in flight.
- Snapshot orchestration is per-account; one stuck or slow account must not block others.

Request lifecycle:

- When threshold is crossed and the account is `Idle`, server picks the best currently connected candidate and sends `SnapshotRequest`.
- If no eligible connected candidate exists, server stays `Idle`. Snapshotting is deferred until a later account event (connect, ack advance, new op, explicit retry tick, etc.) gives the server a reason to try again.
- While `Requested`, server does **not** open a second concurrent request for that account.
- On successful `PushSnapshot`, server durably stores the snapshot, clears the in-flight request, then re-evaluates whether the account is still above threshold.

Failure handling:

- If the assigned connection disconnects before a matching `PushSnapshot` arrives, server clears that in-flight request immediately and tries the next-best currently connected candidate, if any.
- If the request deadline expires (start with a coarse fixed timeout such as 5 minutes), server clears that in-flight request and tries the next-best currently connected candidate, if any.
- If no replacement candidate exists after disconnect/timeout, server returns to `Idle`. Sync continues normally; snapshotting waits for a later retry opportunity.
- Snapshotting is therefore **retryable but not blocking**: inability to get a snapshot may delay compaction, but must not break push/pull/ack.

Correctness / acceptance rules:

- `PushSnapshot` is accepted only if it matches the **current** in-flight request for that account/device.
- Late snapshots from a timed-out, disconnected, or superseded assignee are ignored.
- Unsolicited snapshots (no in-flight request) are ignored.
- Server stores only the latest durable snapshot; no multi-snapshot chain is required.
- The uploaded `up_to_seq` must be **at least** the requested `up_to_seq`, and `compaction_floor_seq` must equal the requested value (the server is asserting a specific compaction floor, not a range). Mismatched snapshots are ignored.
- A newer snapshot may replace an older one atomically once durable.

Operational notes:

- Keep the policy coarse. One candidate at a time, one deadline, one retry decision per failure is enough.
- Large accounts increase snapshot upload/download cost, but do **not** change the orchestration model.
- Server should log: request issued, request timed out, assignee disconnected, snapshot accepted, snapshot ignored as stale/mismatched, and retry abandoned for lack of candidates.

## Shallow snapshots (future)

**Status:** Not implemented. The server's `compaction_floor_seq` exists today (it gates op-blob GC) but it does **not** drive Loro history trimming. The "snapshot" the client produces is a full Loro snapshot.

Loro shallow snapshotting — `ExportMode::shallow_snapshot(frontier)` — trims CRDT history at a *version vector* (Loro Frontiers), not at a server seq. A server seq enumerates encrypted blobs in delivery order and has no exposed mapping to peer/counter pairs inside any blob (the server can't decrypt). So a seq cannot be handed to Loro as a shallow boundary.

The planned design decouples the two horizons rather than trying to translate between them:

- **Op-blob horizon (existing).** `min(last_acked_seq)` across devices → `compaction_floor_seq`. Drives server-side op-blob GC. Server reads this directly from `devices.last_acked_seq`.
- **VV horizon (planned).** Each device periodically reports its current Loro VV/Frontiers to the server (e.g. piggybacked on `Ack`, or via a dedicated `ReportVV` frame on a coarse cadence — single-digit minutes is fine; this isn't on the hot path). Server stores the latest VV per device and computes the **VV meet** across devices when it needs to issue a shallow snapshot. The meet is sent in `SnapshotRequest` alongside the existing fields; the producing client passes it to `ExportMode::shallow_snapshot(frontier)`.

Open questions, parked until this lands:

- Encoding of Loro Frontiers on the wire — Loro provides a canonical byte form; we just need to confirm it round-trips through MessagePack as opaque `bytes`.
- Report cadence: ack-piggyback vs. dedicated frame, and whether to throttle further when the doc is idle.
- Whether the snapshot envelope persists the producer's VV alongside the blob, or relies on Loro to recover it from the blob on import.

Until shallow snapshotting ships, the producer's snapshot blob is full-history and `compaction_floor_seq` only affects whether/when op rows get deleted on the server.

## Bootstrap from snapshot

**Status:** Implemented. The server-prompted path (`SnapshotRequired` → `PullSnapshot` → `Snapshot` → resume `PullOps`) is covered by the `produce_then_bootstrap_round_trip` test. This is the only way into bootstrap — there is no client-driven entry.

A device whose `since_seq` is below the latest snapshot's `compaction_floor_seq` cannot resume from ops alone — the ops it needs have been compacted. (Devices whose `since_seq` is between `compaction_floor_seq` and `up_to_seq` *can* still delta-pull, because those ops are preserved by horizon-bounded compaction.) On receiving a `PullOps` with `since_seq < compaction_floor_seq`, the server replies `SnapshotRequired { up_to_seq }` instead of `OpsBatch`. The client then:

1. `PullSnapshot` → server returns `Snapshot { up_to_seq, blob }`.
2. Decrypt the blob and apply it to the local doc (Loro merges — local-only commits not yet pushed are preserved automatically; CRDT op-id reconciliation handles overlap with the device's own prior contributions). Merge the blob's declared Loro range into `server_known_vv` and persist it.
3. Write the local checkpoint: a fresh full-history export of the **merged** doc (server state ∪ any unsent local work), replacing the local snapshot and pruning the whole WAL prefix. The server blob alone would not be a valid checkpoint — it lacks unsent local operations the pruned WAL rows carried; the merged export contains them and the next push still derives them from `server_known_vv` (see `spec/local-storage.md` §Server snapshots vs local snapshots).
4. Emit one application-level `FullResync` control event; consumers materialize current state once rather than processing one synthetic event per item.
5. `PullOps { since_seq: up_to_seq }` to catch up on any ops written after the snapshot was taken.

The `up_to_seq` carried by `SnapshotRequired` is informational; the authoritative value is the one returned in the `Snapshot` frame, since the latest snapshot may advance between the two round trips.

`SnapshotRequired` and `Snapshot` are only valid in the bootstrap path. Up-to-date devices in steady state never receive either — `OpsBatch` / `OpsBroadcast` covers them. The `Snapshot` frame is therefore only accepted by a client in its bootstrap state, never mid-pull.

## Reconnect

**Status:** Partial. Resume-from-`last_acked_seq` is implemented in both CLI and web engines. Exponential backoff with jitter is implemented in the JS sync bridge (`js/core/src/sync-bridge.ts`); the CLI runs one-shot so backoff doesn't apply there.

Client maintains `last_acked_seq` in local state. On reconnect: WS upgrade with token → `PullOps { since_seq: last_acked_seq }` → resume.

Exponential backoff 1s → 30s.
