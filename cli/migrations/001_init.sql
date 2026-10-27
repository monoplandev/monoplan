-- CLI-only schema (spec/cli.md "Local state"). The generic doc-storage
-- tables (`docs`, `wal`, `snapshots`) live in `monoplan-storage-sqlite`
-- and are applied first against the same db file; this migration adds
-- only the CLI's identity row. Registered in the shared `_migrations`
-- ledger under a distinct name (see `open_storage`) so it doesn't
-- collide with the storage crate's own `001_init`.

-- Singleton (id pinned to 1): the doc this install runs and, when it
-- has an account, the account + device identity it is logged in as.
-- `primary_doc_id` is the Home doc's uuid bytes (matches docs.id):
-- server-assigned at login/recover, minted locally for a local-only
-- workspace (the first doc command on a fresh install) and kept when
-- signup adopts that workspace. The three identity columns are all
-- NULL for a local-only workspace and all set once an account exists.
-- Lives in the db rather than a config file so identity and the doc
-- cache share one transactional store; the presence of `secrets.toml`
-- (not this row) is the "profile exists" marker, and its device token
-- the "has an account" one.
CREATE TABLE account (
  id             INTEGER PRIMARY KEY CHECK (id = 1),
  account_id     TEXT,
  email          TEXT,
  device_id      TEXT,
  primary_doc_id BLOB NOT NULL,
  CHECK ((account_id IS NULL) = (email IS NULL) AND (account_id IS NULL) = (device_id IS NULL))
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
