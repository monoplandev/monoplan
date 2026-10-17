# Storage

Sqlite for now. One database file per server instance. Single-tenant (one account per server) is *not* assumed — the schema is multi-account from day one because the SaaS Postgres path will reuse the same shape.

Every account owns exactly one `docs` row — its **primary doc** (the Home) — and holds an `owner` row for it in `doc_members`. Access to a doc *is* an active `doc_members` row, and the doc's DEK is wrapped per membership (`encryption.md`): the key model is already multi-doc even though today no account has a second membership. See `sharing-plan.md` for what remains (invites, multi-open connections, member management).

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
  kdf_m_kib, kdf_t, kdf_p     INTEGER NOT NULL,            -- argon2 params (three columns)
  primary_doc_id              BLOB NOT NULL REFERENCES docs(id),  -- the account's Home doc
  recovery_salt               BLOB,                        -- present iff recovery code opted in
  recovery_auth_hash          BLOB,                        -- SHA-256(client recovery_auth_secret)
  created_at                  INTEGER NOT NULL             -- unix millis
);
-- Password material only. No key wraps: the DEK is a doc key, held per
-- membership below. The recovery *code* is an account credential (salt +
-- auth hash live here); the wraps it opens are per-membership.

CREATE TABLE doc_members (
  doc_id                       BLOB NOT NULL REFERENCES docs(id),
  account_id                   BLOB NOT NULL REFERENCES accounts(id),
  role                         TEXT NOT NULL,              -- 'owner' | 'member'
  wrapped_dek                  BLOB NOT NULL,              -- this doc's DEK under this member's password KEK
  wrapped_dek_nonce            BLOB NOT NULL,
  recovery_wrapped_dek         BLOB,                       -- same DEK under the member's recovery KEK, iff enrolled
  recovery_wrapped_dek_nonce   BLOB,
  added_at                     INTEGER NOT NULL,
  removed_at                   INTEGER,                    -- NULL = active; set = forward-only revocation
  PRIMARY KEY (doc_id, account_id)
);
CREATE INDEX doc_members_account_id_idx ON doc_members (account_id) WHERE removed_at IS NULL;
-- One `owner` row per account for its primary doc, written at signup. An
-- active row is the access decision for `OpenDoc` and the device set for
-- the compaction horizon. Password change / reset rewrite `wrapped_dek` on
-- every active row in one transaction (encryption.md).

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
  last_seen_at    INTEGER NOT NULL,
  last_seen_ip    TEXT,                                    -- client IP at last_seen_at; overwritten, never a history
  created_at      INTEGER NOT NULL
);
CREATE INDEX devices_account_id_idx ON devices (account_id);

-- Per-(device, doc) contiguous-prefix frontier. Rows appear on first
-- `Ack`; a device with no row is at 0. Revoking a device cascades its
-- rows away, dropping it from every horizon it was pinning.
CREATE TABLE device_doc_frontiers (
  device_id        BLOB NOT NULL REFERENCES devices(id) ON DELETE CASCADE,
  doc_id           BLOB NOT NULL REFERENCES docs(id),
  last_acked_seq   INTEGER NOT NULL DEFAULT 0,
  PRIMARY KEY (device_id, doc_id)
);
CREATE INDEX device_doc_frontiers_doc_id_idx ON device_doc_frontiers (doc_id);

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
  from_seq              INTEGER,             -- NULL = full snapshot; else a delta starting after the previous row's up_to_seq (chain link, must equal it)
  up_to_vv              BLOB NOT NULL,       -- producer's Loro VV at up_to_seq, sealed under the DEK; opaque here, handed back as the next delta's base
  up_to_vv_nonce        BLOB NOT NULL,
  payload               BLOB NOT NULL,
  payload_nonce         BLOB NOT NULL,
  created_at            INTEGER NOT NULL
);
CREATE INDEX snapshots_doc_id_idx ON snapshots (doc_id, id DESC);
```

Snapshot rows form one linear **chain** per doc: a full snapshot (`from_seq IS NULL`) followed by zero or more deltas, each linking to the row before it (`from_seq = previous.up_to_seq`, enforced in the insert transaction). The bootstrap payload is the chain from the latest full row onward, in `id` order. The latest row's `compaction_floor_seq` is the doc's floor. See `sync-protocol.md` §"Snapshot orchestration" for shapes and cadence.

`ops.seq` is per-doc, dense, and gap-free. The client relies on this: seqs arrive contiguous and in order over a single connection, so the engine only ever advances its frontier over the contiguous next seq (no reorder buffering). The composite `(doc_id, seq)` PK doubles as the per-doc ordering index — no separate index needed.

The `doc_sequences` row for a doc is created on first `insert_ops` via `INSERT … ON CONFLICT DO UPDATE`, so signup doesn't need a separate seed step. Reads (`SELECT seq … ORDER BY seq`) never touch the counter; only writers contend on the per-doc row.

Device frontiers are per-(device, doc) in `device_doc_frontiers`, keyed to match the per-doc `seq` space they live in. The row is upserted on `Ack` and only ever moves forward. `GET /api/devices` reports each device's frontier on the account's primary doc.

## Insertion order at signup

`docs` has no FK back to `accounts`, but `accounts.primary_doc_id` references `docs(id)`. The signup transaction is therefore:

1. `INSERT INTO docs(id, created_at) …` — mint the primary doc id.
2. `INSERT INTO accounts(…, primary_doc_id = <doc id from step 1>)`.
3. `INSERT INTO doc_members(doc_id, account_id, role = 'owner', wrapped_dek, …, recovery_wrapped_dek, …)` — the wraps the client posted.
4. `INSERT INTO devices(…)` for the signup device.

All in one tx with `BEGIN IMMEDIATE`. The invariant "`accounts.primary_doc_id` always points to a real `docs` row" is enforced by the FK; "every doc has at least one owner membership" is held by application code.

## Compaction

After a snapshot lands, a background job may, per doc:
1. Delete `ops` rows where `doc_id = X AND seq ≤ snapshot.compaction_floor_seq`. (`compaction_floor_seq` is set to `max(horizon, prev snapshot's compaction_floor_seq)` at snapshot creation time by the orchestrator — see `sync-protocol.md` §"Snapshot orchestration" — so it's safe by construction.)
2. Snapshot rows are never deleted. The chain is one full head plus every delta since; superseding it is the trimmed snapshot's job (§Epochs, not built), and the pruning rule will be written with it.

Horizon is `min(last_acked_seq)` over the doc's `device_doc_frontiers` rows across the devices of every account with an active `doc_members` row on it (the same set `can_open` admits), joined so that a device with no row counts as 0.

Run on a timer, not synchronous with snapshot upload. The `doc_sequences.next_seq` counter is **not** rewound by compaction — `next_seq` keeps climbing even when the rows below the snapshot floor are pruned.

## Epochs

An **epoch** is a span of a doc's history that shares one Loro root. A doc is born in epoch 0. The epoch advances only when a *trimmed* (Loro shallow) snapshot is accepted: that snapshot discards CRDT history before the trim point, so it is the first row of the new epoch and the only lossy step in the storage story. Today every snapshot is full-history or a lossless delta on one, so `epoch` stays at 0 everywhere; the columns are reserved so rows are stamped from day one and trimming can land without a schema change.

Where it lives and what each copy is for:

- `doc_sequences.epoch` is the doc's **current** epoch. It is the only copy that is written independently; `insert_ops` reads it under the same row lock as `next_seq`.
- `ops.epoch` is the doc's epoch at insert time, stamped by the server. It is derivable from `seq` plus the snapshot rows, and is denormalised for cheap per-op lookup and as the natural key for a future hot/cold split. It is **not** the compaction predicate: ops pushed between a trimmed snapshot's `up_to_seq` and its acceptance belong to the old epoch but are in no snapshot, so GC stays `seq <= compaction_floor_seq`.
- `snapshots.epoch` is the epoch the snapshot belongs to. A trimmed snapshot's `compaction_floor_seq` is its epoch's compatibility line.

`seq` never resets at an epoch boundary; `since_seq`, acks and the push_id dedup index all keep working across epochs.

Compatibility rule: superseded by `spec/trim-plan.md`. The rule previously written here assumed an "ack-after-push" client invariant that the engine does not have (acks fire during `Pulling`, before the post-reconnect push), so a seq-based check on push cannot tell a stale device from a fresh one. Under the trim plan every device wipes and bootstraps on an epoch change, pushes carry the epoch, and the server rejects mismatches.

## Sqlite settings

- WAL mode (`PRAGMA journal_mode=WAL`).
- `PRAGMA synchronous=NORMAL` for write throughput; durability is per-WAL-checkpoint.
- `PRAGMA busy_timeout=5000`.
- `PRAGMA foreign_keys=ON`.

## Migrations

- `001_init.sql` creates the current schema for fresh servers.

## Open questions

- Backup story for self-hosted (online backup via `VACUUM INTO`? out of scope for now).
