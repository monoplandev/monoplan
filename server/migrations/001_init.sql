-- Monoplan initial schema. Sqlite-only for now. Multi-account from day one so the
-- SaaS Postgres path can reuse the shape.

-- Each account owns exactly one `docs` row today (its primary doc / Home).
-- The entity exists as a first-class row to make the eventual multi-doc /
-- sharing migration incremental rather than a flag day; see sharing-plan.md.
CREATE TABLE docs (
  id           BLOB PRIMARY KEY,                              -- uuid v7 bytes
  created_at   INTEGER NOT NULL
);

-- Password material only. Key wraps live on `doc_members`: the DEK is
-- a per-doc key, not an account key, so the account row never holds
-- one. `recovery_salt` / `recovery_auth_hash` stay here because the
-- recovery *code* is an account credential; the wraps it opens are
-- per-membership.
CREATE TABLE accounts (
  id                          BLOB PRIMARY KEY,                -- uuid v7 bytes
  email                       TEXT UNIQUE NOT NULL
                              CHECK (email = lower(trim(email))), -- canonical form, see server/src/auth/email.rs
  password_hash               BLOB NOT NULL,                   -- SHA-256(client auth_secret)
  password_salt               BLOB NOT NULL,                   -- master_salt for client KDF
  kdf_m_kib                   INTEGER NOT NULL,                -- argon2 memory (KiB)
  kdf_t                       INTEGER NOT NULL,                -- argon2 iterations
  kdf_p                       INTEGER NOT NULL,                -- argon2 parallelism
  primary_doc_id              BLOB NOT NULL REFERENCES docs(id), -- the account's Home doc
  recovery_salt               BLOB,                            -- present iff recovery code opted in
  recovery_auth_hash          BLOB,                            -- SHA-256(client recovery_auth_secret)
  created_at                  INTEGER NOT NULL                 -- unix millis
);

-- Membership = access to a doc + this member's copy of the doc's DEK,
-- wrapped under their own password KEK (and recovery KEK iff they
-- enrolled a recovery code). Every account has exactly one `owner`
-- row for its primary doc, inserted at signup; sharing adds rows per
-- invitee without touching `accounts`. Password change / reset re-wrap
-- every active membership in one transaction. `removed_at` is a
-- forward-only revocation marker (sharing-plan.md): the server stops
-- serving the doc, but the member keeps whatever they already pulled.
CREATE TABLE doc_members (
  doc_id                       BLOB NOT NULL REFERENCES docs(id),
  account_id                   BLOB NOT NULL REFERENCES accounts(id),
  role                         TEXT NOT NULL,                  -- 'owner' | 'member'
  wrapped_dek                  BLOB NOT NULL,
  wrapped_dek_nonce            BLOB NOT NULL,
  recovery_wrapped_dek         BLOB,
  recovery_wrapped_dek_nonce   BLOB,
  added_at                     INTEGER NOT NULL,               -- unix millis
  removed_at                   INTEGER,                        -- NULL = active
  PRIMARY KEY (doc_id, account_id)
);
CREATE INDEX doc_members_account_id_idx ON doc_members (account_id) WHERE removed_at IS NULL;

CREATE TABLE recovery_sessions (
  token_hash      BLOB PRIMARY KEY,                            -- SHA-256(token)
  account_id      BLOB NOT NULL REFERENCES accounts(id),
  expires_at      INTEGER NOT NULL,
  consumed_at     INTEGER                                      -- nullable; set on /password/reset success
);
CREATE INDEX recovery_sessions_account_id_idx ON recovery_sessions (account_id);

CREATE TABLE devices (
  id                  BLOB PRIMARY KEY,                        -- uuid v7 bytes
  account_id          BLOB NOT NULL REFERENCES accounts(id),
  name                TEXT NOT NULL,
  auth_token_hash     BLOB NOT NULL,                           -- SHA-256(device_token bytes)
  last_seen_at        INTEGER NOT NULL,
  last_seen_ip        TEXT,                                    -- client IP at last_seen_at; overwritten, never a history
  created_at          INTEGER NOT NULL
);
CREATE INDEX devices_account_id_idx ON devices (account_id);
CREATE INDEX devices_token_hash_idx ON devices (auth_token_hash);

-- Per-(device, doc) contiguous-prefix frontier: the highest seq the
-- device has acked on that doc. Rows appear on first `Ack`; a device
-- with no row is at 0. The horizon for a doc is the minimum over the
-- devices of every account with an active `doc_members` row on it,
-- which gates op-blob compaction. Revoking a device drops
-- its rows (cascade) and so drops it from the horizon.
CREATE TABLE device_doc_frontiers (
  device_id        BLOB NOT NULL REFERENCES devices(id) ON DELETE CASCADE,
  doc_id           BLOB NOT NULL REFERENCES docs(id),
  last_acked_seq   INTEGER NOT NULL DEFAULT 0,
  PRIMARY KEY (device_id, doc_id)
);
CREATE INDEX device_doc_frontiers_doc_id_idx ON device_doc_frontiers (doc_id);

-- Per-doc monotonic counter. Bumped inside the same transaction
-- that inserts ops so seqs are dense, contiguous, and gap-free for any
-- single doc — clients spot real holes (replica lag, dropped frames)
-- instead of confusing them with "another doc got that id".
--
-- `epoch` is the doc's current history epoch. A doc is born in epoch 0
-- and the epoch advances only when a *trimmed* (Loro shallow) snapshot
-- is accepted, the one lossy step in the storage story. Today every
-- snapshot is full-history, so the value never moves; the column is
-- reserved so ops and snapshots are stamped from day one. It lives
-- here rather than on `docs` because `insert_ops` already holds this
-- row's lock and reads both values in one statement. `seq` never
-- resets at an epoch boundary.
CREATE TABLE doc_sequences (
  doc_id       BLOB PRIMARY KEY REFERENCES docs(id),
  next_seq     INTEGER NOT NULL DEFAULT 1,
  epoch        INTEGER NOT NULL DEFAULT 0
);

CREATE TABLE ops (
  doc_id          BLOB NOT NULL REFERENCES docs(id),
  seq             INTEGER NOT NULL,                            -- per-doc monotonic, gap-free
  epoch           INTEGER NOT NULL,                            -- doc's epoch at insert (stamped server-side; NOT the GC predicate, see storage.md §Epochs)
  device_id       BLOB,                                        -- originating device (uuid bytes)
  push_id         BLOB,                                        -- client-minted durable push id (uuid bytes)
  payload         BLOB NOT NULL,
  payload_nonce   BLOB NOT NULL,
  created_at      INTEGER NOT NULL,
  PRIMARY KEY (doc_id, seq)
);
-- Push idempotency: a client that crashed (or lost the ack) re-sends
-- the same (device_id, push_id); the lookup below returns the original
-- seq instead of inserting a duplicate row. Dedup records live on the
-- ops rows themselves, so they persist at least until compaction — and
-- compaction only deletes rows below the horizon, i.e. rows every
-- device (including the originator) has already acked past, at which
-- point the originator can no longer legitimately retry that push_id.
CREATE UNIQUE INDEX ops_device_push_idx ON ops (doc_id, device_id, push_id)
  WHERE device_id IS NOT NULL AND push_id IS NOT NULL;

CREATE TABLE snapshots (
  id                    INTEGER PRIMARY KEY AUTOINCREMENT,
  doc_id                BLOB NOT NULL REFERENCES docs(id),
  up_to_seq             INTEGER NOT NULL,                        -- encoded state frontier (per-doc)
  compaction_floor_seq  INTEGER NOT NULL,                        -- seq at/below which op blobs are eligible for GC once this snapshot lands
  epoch                 INTEGER NOT NULL,                        -- epoch this snapshot belongs to; a trimmed snapshot is the first row of its epoch
  from_seq              INTEGER,                                 -- NULL = full snapshot; else a delta starting after the previous row's up_to_seq (chain link, must equal it)
  up_to_vv              BLOB NOT NULL,                           -- producer's Loro VV at up_to_seq, sealed under the DEK; opaque here, handed back as the next delta's base
  up_to_vv_nonce        BLOB NOT NULL,
  payload               BLOB NOT NULL,
  payload_nonce         BLOB NOT NULL,
  created_at            INTEGER NOT NULL
);
CREATE INDEX snapshots_doc_id_idx ON snapshots (doc_id, id DESC);
