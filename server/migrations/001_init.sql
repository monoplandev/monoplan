-- Monoplan initial schema. Sqlite-only for now. Multi-account from day one so the
-- SaaS Postgres path can reuse the shape.

-- Each account owns exactly one `docs` row today (its primary doc / Home).
-- The entity exists as a first-class row to make the eventual multi-doc /
-- sharing migration incremental rather than a flag day; see sharing-plan.md.
CREATE TABLE docs (
  id           BLOB PRIMARY KEY,                              -- uuid v7 bytes
  created_at   INTEGER NOT NULL
);

CREATE TABLE accounts (
  id                          BLOB PRIMARY KEY,                -- uuid v7 bytes
  email                       TEXT UNIQUE NOT NULL,
  password_hash               BLOB NOT NULL,                   -- SHA-256(client auth_secret)
  password_salt               BLOB NOT NULL,                   -- master_salt for client KDF
  kdf_m_kib                   INTEGER NOT NULL,                -- argon2 memory (KiB)
  kdf_t                       INTEGER NOT NULL,                -- argon2 iterations
  kdf_p                       INTEGER NOT NULL,                -- argon2 parallelism
  primary_doc_id              BLOB NOT NULL REFERENCES docs(id), -- the account's Home doc
  wrapped_dek                 BLOB NOT NULL,
  wrapped_dek_nonce           BLOB NOT NULL,
  recovery_salt               BLOB,                            -- present iff recovery code opted in
  recovery_auth_hash          BLOB,                            -- SHA-256(client recovery_auth_secret)
  recovery_wrapped_dek        BLOB,
  recovery_wrapped_dek_nonce  BLOB,
  created_at                  INTEGER NOT NULL                 -- unix millis
);

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
  last_acked_seq      INTEGER NOT NULL DEFAULT 0,              -- per-account contiguous-prefix frontier
  last_seen_at        INTEGER NOT NULL,
  last_seen_ip        TEXT,                                    -- client IP at last_seen_at; overwritten, never a history
  created_at          INTEGER NOT NULL
);
CREATE INDEX devices_account_id_idx ON devices (account_id);
CREATE INDEX devices_token_hash_idx ON devices (auth_token_hash);

-- Per-doc monotonic counter. Bumped inside the same transaction
-- that inserts ops so seqs are dense, contiguous, and gap-free for any
-- single doc — clients spot real holes (replica lag, dropped frames)
-- instead of confusing them with "another doc got that id".
CREATE TABLE doc_sequences (
  doc_id       BLOB PRIMARY KEY REFERENCES docs(id),
  next_seq     INTEGER NOT NULL DEFAULT 1
);

CREATE TABLE ops (
  doc_id          BLOB NOT NULL REFERENCES docs(id),
  seq             INTEGER NOT NULL,                            -- per-doc monotonic, gap-free
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
  payload               BLOB NOT NULL,
  payload_nonce         BLOB NOT NULL,
  created_at            INTEGER NOT NULL
);
CREATE INDEX snapshots_doc_id_idx ON snapshots (doc_id, id DESC);
