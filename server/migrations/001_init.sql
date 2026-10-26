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
-- a per-doc key, so the account row never holds one. `recovery_salt` /
-- `recovery_auth_hash` stay here because the recovery *code* is an
-- account credential; the wrap it opens sits on the primary membership.
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
  recovery_salt               BLOB,                            -- set at signup (mandatory); NULL only after a future opt-out
  recovery_auth_hash          BLOB,                            -- SHA-256(client recovery_auth_secret)
  created_at                  INTEGER NOT NULL                 -- unix millis
);

-- Membership = access to a doc + this member's copy of the doc's DEK.
-- The row whose doc_id = accounts.primary_doc_id is the member's
-- *primary membership*: its `wrapped_dek` is the Home DEK under the
-- password KEK and `recovery_wrapped_dek` (iff enrolled) the Home DEK
-- under the recovery KEK. On every other row `wrapped_dek` is that
-- doc's DEK under the member's Home DEK and the recovery columns are
-- NULL (spec/encryption.md §Wrap-states). Every account has exactly
-- one `owner` row for its primary doc, inserted at signup; `POST
-- /api/docs` and (later) sharing add rows without touching `accounts`.
-- Password change / reset rewrite the primary membership only; the
-- server cannot tell which key a wrap is under and does not try.
-- `removed_at` is a forward-only revocation marker (sharing-plan.md):
-- the server stops serving the doc, but the member keeps whatever they
-- already pulled.
CREATE TABLE doc_members (
  doc_id                       BLOB NOT NULL REFERENCES docs(id),
  account_id                   BLOB NOT NULL REFERENCES accounts(id),
  role                         TEXT NOT NULL,                  -- 'owner' | 'member'
  wrapped_dek                  BLOB NOT NULL,                  -- primary: Home DEK under password KEK; else: doc DEK under Home DEK
  wrapped_dek_nonce            BLOB NOT NULL,
  recovery_wrapped_dek         BLOB,                           -- primary membership only, iff recovery enrolled
  recovery_wrapped_dek_nonce   BLOB,
  added_at                     INTEGER NOT NULL,               -- unix millis
  removed_at                   INTEGER,                        -- NULL = active
  PRIMARY KEY (doc_id, account_id)
);
CREATE INDEX doc_members_account_id_idx ON doc_members (account_id) WHERE removed_at IS NULL;

-- A sharing link (sharing-plan.md §Invites). The link's 32-byte secret
-- never reaches the server; `id` is an HKDF subkey of it (one-way), and
-- `wrapped_dek` is the doc's DEK under a second, independent subkey, so
-- this row on its own opens nothing. Single use: `consumed_at` is set in
-- the same transaction that inserts the acceptor's `doc_members` row.
-- Revoking deletes the row. Never issued for an account's primary doc.
CREATE TABLE doc_invites (
  id                        BLOB PRIMARY KEY,                -- HKDF(secret, "monoplan/invite_id/v1"), 32 bytes
  doc_id                    BLOB NOT NULL REFERENCES docs(id),
  issued_by_account_id      BLOB NOT NULL REFERENCES accounts(id),
  wrapped_dek               BLOB NOT NULL,                   -- doc DEK under the invite KEK
  wrapped_dek_nonce         BLOB NOT NULL,
  created_at                INTEGER NOT NULL,                -- unix millis
  expires_at                INTEGER NOT NULL,
  consumed_at               INTEGER,                         -- NULL = open
  consumed_by_account_id    BLOB REFERENCES accounts(id)
);
CREATE INDEX doc_invites_doc_id_idx ON doc_invites (doc_id);

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
  kind                TEXT NOT NULL,                           -- client-reported: web | macos | linux | windows | ios | android
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
-- which gates update-blob compaction. Revoking a device drops
-- its rows (cascade) and so drops it from the horizon.
CREATE TABLE device_doc_frontiers (
  device_id        BLOB NOT NULL REFERENCES devices(id) ON DELETE CASCADE,
  doc_id           BLOB NOT NULL REFERENCES docs(id),
  last_acked_seq   INTEGER NOT NULL DEFAULT 0,
  PRIMARY KEY (device_id, doc_id)
);
CREATE INDEX device_doc_frontiers_doc_id_idx ON device_doc_frontiers (doc_id);

-- Per-doc monotonic counter. Bumped inside the same transaction
-- that inserts updates so seqs are dense, contiguous, and gap-free for any
-- single doc — clients spot real holes (replica lag, dropped frames)
-- instead of confusing them with "another doc got that id".
--
-- `epoch` is the doc's current history epoch. A doc is born in epoch 0
-- and the epoch advances only when a *trimmed* (Loro shallow) snapshot
-- is accepted, the one lossy step in the storage story. Today every
-- snapshot is full-history, so the value never moves; the column is
-- reserved so updates and snapshots are stamped from day one. It lives
-- here rather than on `docs` because `insert_updates` already holds this
-- row's lock and reads both values in one statement. `seq` never
-- resets at an epoch boundary.
CREATE TABLE doc_sequences (
  doc_id       BLOB PRIMARY KEY REFERENCES docs(id),
  next_seq     INTEGER NOT NULL DEFAULT 1,
  epoch        INTEGER NOT NULL DEFAULT 0
);

CREATE TABLE updates (
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
-- updates rows themselves, so they persist at least until compaction — and
-- compaction only deletes rows below the horizon, i.e. rows every
-- device (including the originator) has already acked past, at which
-- point the originator can no longer legitimately retry that push_id.
CREATE UNIQUE INDEX updates_device_push_idx ON updates (doc_id, device_id, push_id)
  WHERE device_id IS NOT NULL AND push_id IS NOT NULL;

CREATE TABLE snapshots (
  id                    INTEGER PRIMARY KEY AUTOINCREMENT,
  doc_id                BLOB NOT NULL REFERENCES docs(id),
  up_to_seq             INTEGER NOT NULL,                        -- encoded state frontier (per-doc)
  compaction_floor_seq  INTEGER NOT NULL,                        -- seq at/below which update blobs are eligible for GC once this snapshot lands
  epoch                 INTEGER NOT NULL,                        -- epoch this snapshot belongs to; a trimmed snapshot is the first row of its epoch
  from_seq              INTEGER,                                 -- NULL = full snapshot; else a delta starting after the previous row's up_to_seq (chain link, must equal it)
  up_to_vv              BLOB NOT NULL,                           -- producer's Loro VV at up_to_seq, sealed under the DEK; opaque here, handed back as the next delta's base
  up_to_vv_nonce        BLOB NOT NULL,
  payload               BLOB NOT NULL,
  payload_nonce         BLOB NOT NULL,
  created_at            INTEGER NOT NULL
);
CREATE INDEX snapshots_doc_id_idx ON snapshots (doc_id, id DESC);
