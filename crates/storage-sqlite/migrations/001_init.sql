-- Shared local doc storage (spec/local-storage.md): snapshot + WAL
-- crash recovery plus the VV-based sync cursors, per
-- spec/vv-wal-separation.md. This is the generic, client-agnostic
-- portion of the schema; callers that need their own tables (e.g. the
-- CLI's `account` identity row) supply them as extra migrations
-- through `open_with_extra`, sharing the same db file and
-- `_migrations` ledger.

CREATE TABLE docs (
  id                    BLOB PRIMARY KEY,    -- uuid v7 bytes; matches server-side docs.id
  created_at            INTEGER NOT NULL,    -- unix seconds (unixepoch())
  -- Durable server-log delivery frontier: the highest *contiguous*
  -- server_seq this device has durably applied — the resume `PullUpdates`
  -- cursor. Persisted explicitly (never derived from
  -- MAX(wal.server_seq)) so it survives snapshot folding pruning the
  -- very rows it was derived from, and never jumps a gap.
  last_acked_server_seq INTEGER NOT NULL DEFAULT 0,
  -- Encoded Loro VersionVector of the operations proven to exist in
  -- the server update stream. The outbound delta is derived from Loro
  -- history against this — never from which WAL rows still exist.
  -- NULL = never synced.
  server_known_vv       BLOB,
  -- Unix millis of the last successful online flush; NULL = never.
  -- Observability only — nothing in the sync path reads it.
  last_sync_at          INTEGER
);

-- The local WAL: encrypted Loro updates applied after the snapshot,
-- replayed in `local_seq` order on boot. Pure crash recovery — rows
-- are pruned whenever a snapshot contains them (full Loro snapshots
-- retain history, so even unsent local operations survive folding and
-- re-derive from `server_known_vv`).
CREATE TABLE wal (
  doc_id        BLOB NOT NULL REFERENCES docs(id),
  local_seq     INTEGER NOT NULL,          -- dense, per-doc, storage-assigned
  server_seq    INTEGER,                    -- NULL for local-origin rows; set for server-delivered rows (idempotent re-delivery detection)
  payload       BLOB NOT NULL,              -- EncryptedBlob.ciphertext (DEK-sealed)
  payload_nonce BLOB NOT NULL,              -- EncryptedBlob.nonce
  created_at    INTEGER NOT NULL,
  PRIMARY KEY (doc_id, local_seq)
);

CREATE UNIQUE INDEX wal_server_seq_idx ON wal (doc_id, server_seq) WHERE server_seq IS NOT NULL;

CREATE TABLE snapshots (
  doc_id          BLOB PRIMARY KEY REFERENCES docs(id),
  up_to_local_seq INTEGER NOT NULL,         -- local-counter high-water at write time (keeps local_seq monotonic after a prune); NOT a replay cutoff
  payload         BLOB NOT NULL,            -- EncryptedBlob.ciphertext (full-history snapshot)
  payload_nonce   BLOB NOT NULL,
  created_at      INTEGER NOT NULL
);

-- At most one durable in-flight push per doc: the exact encrypted
-- update blob currently (or last) on the wire. Written *before* the
-- first send; cleared on ack. A crash between server insert and
-- client ack retries the same push_id and the server deduplicates.
CREATE TABLE in_flight_push (
  doc_id        BLOB PRIMARY KEY REFERENCES docs(id),
  push_id       BLOB NOT NULL,              -- uuid bytes (retry/idempotency key)
  payload       BLOB NOT NULL,              -- EncryptedBlob.ciphertext
  payload_nonce BLOB NOT NULL,              -- EncryptedBlob.nonce
  from_vv       BLOB NOT NULL,              -- encoded VV the delta was exported from (forensics)
  to_vv         BLOB NOT NULL,              -- encoded oplog VV at export time; merged into server_known_vv on ack
  created_at    INTEGER NOT NULL
);
