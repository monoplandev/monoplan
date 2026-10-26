-- CLI-only schema (spec/cli.md "Local state"). The generic doc-storage
-- tables (`docs`, `wal`, `snapshots`) live in `monoplan-storage-sqlite`
-- and are applied first against the same db file; this migration adds
-- only the CLI's identity row. Registered in the shared `_migrations`
-- ledger under a distinct name (see `open_storage`) so it doesn't
-- collide with the storage crate's own `001_init`.

-- Singleton (id pinned to 1): the account + device identity this
-- install is logged in as. Server-assigned, written once at
-- signup/login/recover. `primary_doc_id` is the Home doc's uuid bytes
-- (matches docs.id). Lives in the db rather than a config file so
-- identity and the doc cache share one transactional store; the
-- presence of `secrets.toml` (not this row) is the "logged in" marker.
CREATE TABLE account (
  id             INTEGER PRIMARY KEY CHECK (id = 1),
  account_id     TEXT NOT NULL,
  email          TEXT NOT NULL,
  device_id      TEXT NOT NULL,
  primary_doc_id BLOB NOT NULL
);

-- Loro peer id per lease slot (spec/peer-id-plan.md). Slot N's peer id
-- is minted (random u64, bit-cast to i64) on the slot's first claim and
-- reused for every later claim, so the doc's version vector stays as
-- wide as this device's max historical concurrency, not its invocation
-- count. Which slot a process may *use* is decided by the flock lease
-- on `peer-<slot>.lock` in the profile dir, never by this table —
-- device-local state, deliberately outside the CRDT doc, and
-- deliberately surviving `monoplan cache clear` (counters resume safely
-- from replayed history).
CREATE TABLE peer_slots (
  slot     INTEGER PRIMARY KEY,
  peer_id  INTEGER NOT NULL
);
