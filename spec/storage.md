# Storage

Sqlite for now. One database file per server instance. Single-tenant (one account per server) is *not* assumed — the schema is multi-account from day one because the SaaS Postgres path will reuse the same shape.

Every account owns exactly one `docs` row — its **primary doc** (the Home). Today that's the only doc an account ever has; the `docs` entity and `accounts.primary_doc_id` pointer exist to make the eventual multi-doc/sharing migration incremental rather than a flag day. See `sharing-plan.md` for what's planned but not built.

## Schema (`migrations/001_init.sql`)

```sql
CREATE TABLE docs (
  id           BLOB PRIMARY KEY,                            -- uuid v7
  created_at   INTEGER NOT NULL
);
-- Standalone entity; no FK back to accounts. Today every doc is some account's
-- primary doc (1:1); shared docs (sharing-plan.md) would attach via a future
-- doc_members table without changing this row.

CREATE TABLE accounts (
  id                          BLOB PRIMARY KEY,            -- uuid v7
  email                       TEXT UNIQUE NOT NULL,
  password_hash               BLOB NOT NULL,               -- SHA-256(client auth_secret)
  password_salt               BLOB NOT NULL,               -- master_salt for client KDF
  primary_doc_id              BLOB NOT NULL REFERENCES docs(id),  -- the account's Home doc
  wrapped_dek                 BLOB NOT NULL,
  wrapped_dek_nonce           BLOB NOT NULL,
  recovery_salt               BLOB,                        -- present iff recovery code opted in
  recovery_auth_hash          BLOB,                        -- SHA-256(client recovery_auth_secret)
  recovery_wrapped_dek        BLOB,
  recovery_wrapped_dek_nonce  BLOB,
  created_at                  INTEGER NOT NULL             -- unix millis
);
-- wrapped_dek here is the primary doc's DEK (1:1 with accounts today). When
-- sharing lands it moves to per-membership wraps; see sharing-plan.md.

CREATE TABLE recovery_sessions (
  token_hash      BLOB PRIMARY KEY,                        -- SHA-256(token)
  account_id      BLOB NOT NULL REFERENCES accounts(id),
  expires_at      INTEGER NOT NULL,
  consumed_at     INTEGER                                  -- nullable; set on /password/reset success
);
CREATE INDEX recovery_sessions_account_id_idx ON recovery_sessions (account_id);

CREATE TABLE devices (
  id              BLOB PRIMARY KEY,                        -- uuid v7
  account_id      BLOB NOT NULL REFERENCES accounts(id),
  name            TEXT NOT NULL,
  auth_token_hash TEXT NOT NULL,
  last_acked_seq  INTEGER NOT NULL DEFAULT 0,              -- per-account contiguous-prefix frontier
  last_seen_at    INTEGER NOT NULL,
  last_seen_ip    TEXT,                                    -- client IP at last_seen_at; overwritten, never a history
  created_at      INTEGER NOT NULL
);
CREATE INDEX devices_account_id_idx ON devices (account_id);

-- Per-doc monotonic counter. UPDATEd in the same tx as the op
-- insert so seqs are dense and gap-free for any single doc.
-- `epoch` is the doc's current history epoch (see §Epochs).
CREATE TABLE doc_sequences (
  doc_id       BLOB PRIMARY KEY REFERENCES docs(id),
  next_seq     INTEGER NOT NULL DEFAULT 1,
  epoch        INTEGER NOT NULL DEFAULT 0
);

CREATE TABLE ops (
  doc_id          BLOB NOT NULL REFERENCES docs(id),
  seq             INTEGER NOT NULL,                        -- per-doc monotonic, gap-free
  epoch           INTEGER NOT NULL,                        -- doc's epoch at insert, stamped server-side (see §Epochs)
  device_id       BLOB,                                    -- originating device (uuid bytes)
  push_id         BLOB,                                    -- client-minted durable push id (uuid bytes)
  payload         BLOB NOT NULL,
  payload_nonce   BLOB NOT NULL,
  created_at      INTEGER NOT NULL,
  PRIMARY KEY (doc_id, seq)
);
-- Push idempotency (spec/sync-protocol.md §"Push identity & retry"): a
-- crash-retry of the same (device, push_id) acks its original seq
-- without inserting a second row. Dedup records ride on the ops rows,
-- so they persist until compaction — which only deletes rows every
-- device (the originator included) has acked past.
CREATE UNIQUE INDEX ops_device_push_idx ON ops (doc_id, device_id, push_id)
  WHERE device_id IS NOT NULL AND push_id IS NOT NULL;

CREATE TABLE snapshots (
  id                    INTEGER PRIMARY KEY AUTOINCREMENT,
  doc_id                BLOB NOT NULL REFERENCES docs(id),
  up_to_seq             INTEGER NOT NULL,    -- snapshot's encoded state frontier (per-doc)
  compaction_floor_seq  INTEGER NOT NULL,    -- seq at/below which op blobs are eligible for GC once this snapshot lands (= max(horizon, prev snapshot's compaction_floor_seq) at snapshot time). Doubles as the bootstrap gate.
  epoch                 INTEGER NOT NULL,    -- epoch this snapshot belongs to; a trimmed snapshot is the first row of its epoch
  payload               BLOB NOT NULL,
  payload_nonce         BLOB NOT NULL,
  created_at            INTEGER NOT NULL
);
CREATE INDEX snapshots_doc_id_idx ON snapshots (doc_id, id DESC);
```

`ops.seq` is per-doc, dense, and gap-free. The client relies on this: seqs arrive contiguous and in order over a single connection, so the engine only ever advances its frontier over the contiguous next seq (no reorder buffering). The composite `(doc_id, seq)` PK doubles as the per-doc ordering index — no separate index needed.

The `doc_sequences` row for a doc is created on first `insert_ops` via `INSERT … ON CONFLICT DO UPDATE`, so signup doesn't need a separate seed step. Reads (`SELECT seq … ORDER BY seq`) never touch the counter; only writers contend on the per-doc row.

`devices.last_acked_seq` stays per-device. While each account has exactly one doc (today's 1:1 between account and doc), per-device equals per-(device, doc); the field's value lives in the doc's seq space. Per-(device, doc) frontiers are needed when an account joins multiple docs; deferred — see `sharing-plan.md`.

## Insertion order at signup

`docs` has no FK back to `accounts`, but `accounts.primary_doc_id` references `docs(id)`. The signup transaction is therefore:

1. `INSERT INTO docs(id, created_at) …` — mint the primary doc id.
2. `INSERT INTO accounts(…, primary_doc_id = <doc id from step 1>, wrapped_dek, …)`.
3. `INSERT INTO devices(…)` for the signup device.

All in one tx with `BEGIN IMMEDIATE`. The invariant "`accounts.primary_doc_id` always points to a real `docs` row" is enforced by the FK; the inverse "every doc belongs to some account" is held by application code today (will relax under sharing).

## Compaction

After a snapshot lands, a background job may, per doc:
1. Delete `ops` rows where `doc_id = X AND seq ≤ snapshot.compaction_floor_seq`. (`compaction_floor_seq` is set to `max(horizon, prev snapshot's compaction_floor_seq)` at snapshot creation time by the orchestrator — see `sync-protocol.md` §"Snapshot orchestration" — so it's safe by construction.)
2. Keep at most M=2 snapshots per doc; delete older.

Horizon is computed from `devices.last_acked_seq` for the account that owns the doc — same value as the per-doc horizon while the 1:1 holds.

Run on a timer, not synchronous with snapshot upload. The `doc_sequences.next_seq` counter is **not** rewound by compaction — `next_seq` keeps climbing even when the rows below the snapshot floor are pruned.

## Epochs

An **epoch** is a span of a doc's history that shares one Loro root. A doc is born in epoch 0. The epoch advances only when a *trimmed* (Loro shallow) snapshot is accepted: that snapshot discards CRDT history before the trim point, so it is the first row of the new epoch and the only lossy step in the storage story. Today every snapshot is full-history, so `epoch` stays at 0 everywhere; the columns are reserved so rows are stamped from day one and trimming can land without a schema change.

Where it lives and what each copy is for:

- `doc_sequences.epoch` is the doc's **current** epoch. It is the only copy that is written independently; `insert_ops` reads it under the same row lock as `next_seq`.
- `ops.epoch` is the doc's epoch at insert time, stamped by the server. It is derivable from `seq` plus the snapshot rows, and is denormalised for cheap per-op lookup and as the natural key for a future hot/cold split. It is **not** the compaction predicate: ops pushed between a trimmed snapshot's `up_to_seq` and its acceptance belong to the old epoch but are in no snapshot, so GC stays `seq <= compaction_floor_seq`.
- `snapshots.epoch` is the epoch the snapshot belongs to. A trimmed snapshot's `compaction_floor_seq` is its epoch's compatibility line.

`seq` never resets at an epoch boundary; `since_seq`, acks and the push_id dedup index all keep working across epochs.

Compatibility rule (for when trimming lands): a device whose `last_acked_seq` is at or above the current epoch's trim floor stays compatible and keeps delta-pulling and pushing. Its unsent work was created against a frontier it had already pushed past (the ack-after-push client invariant), so it sits at or after the shallow root. A device below the floor must bootstrap from the trimmed snapshot and rebase unsent work. The server enforces this on push from `devices.last_acked_seq` against the floor; it needs no client declaration, though the wire will also carry `epoch` for clarity. Rejecting on push matters because a stale-history blob that lands would stall every device's import at that seq.

## Sqlite settings

- WAL mode (`PRAGMA journal_mode=WAL`).
- `PRAGMA synchronous=NORMAL` for write throughput; durability is per-WAL-checkpoint.
- `PRAGMA busy_timeout=5000`.
- `PRAGMA foreign_keys=ON`.

## Migrations

- `001_init.sql` creates the current schema for fresh servers.

## Open questions

- Backup story for self-hosted (online backup via `VACUUM INTO`? out of scope for now).
