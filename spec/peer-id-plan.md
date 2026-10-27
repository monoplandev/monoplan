# Peer id plan (CLI minimum version)

Status: built (CLI; web single-tab slot 0, Sep 2026). The web interim
version is described under "Web (built, interim)" at the end; the
multi-slot pool and multi-tab remain future work.

Note for existing dev profiles: `peer_slots` was added to
`cli/migrations/001_init.sql` in place (pre-release rule), so a profile
whose `_migrations` ledger already contains `001_cli` lacks the table -
wipe local dev state (`bun run wipe` / `monoplan auth logout`) rather than
adding a bridge migration.

## Problem

Every CLI invocation currently mints a fresh random Loro peer id:
`Session::open` (`cli/src/sync.rs`) -> `boot_doc` (`core/src/storage.rs`)
-> `Doc::empty()` -> `LoroDoc::new()`. The CLI is a one-shot process per
command, so every *mutating* invocation adds one permanent entry to the
doc's version vector. VVs are carried forever in retained history and
shipped/compared throughout sync: `server_known_vv`, `partial_end_vv`
snapshot metadata, push `from_vv`/`to_vv`, server frontier tracking. A
multi-year db accumulates thousands of peers -> hundreds of KB of VV,
plus per-peer oplog change-store overhead that directly worsens the boot
replay cost measured in `spec/tui-plan.md`.

## Design: stable per-device peer id + flock-leased slots

Reuse one peer id per device for as long as only one process is live.
Concurrent invocations are broken by a small slot pool; slots are
*reused*, so VV width is bounded by max historical concurrency on the
device (realistically 1-2 entries), not invocation count.

Loro's own `set_peer_id` docs state the safety rule this design
satisfies: a fixed peer id requires strict single-ownership locking.
Two live docs holding the same peer id concurrently is the one
unrecoverable corruption (duplicate `(peer, counter)` op ids); sqlite
serialises the *writes* but cannot prevent the *semantic* collision, so
the lease must be process-scoped, not db-scoped.

### The lease: flock, not sqlite

One lock file per slot in the profile directory, next to the db:

```
<profile dir>/peer-<slot>.lock      # slot = 0, 1, 2, ...
```

- Claim: `open(O_CREAT)` then `flock(LOCK_EX | LOCK_NB)`. On `EWOULDBLOCK`
  try the next slot. Slot 0 succeeds in the overwhelmingly common case.
- Hold: keep the `File` handle alive for the whole process (store it in
  `Session`; for pre-session commands, in the command scope). Dropping it
  at exit releases the lock; on crash the kernel releases it
  unconditionally. Zero staleness bookkeeping - this is the whole reason
  flock beats a lease row in sqlite (a claimed row goes stale on crash
  and forces liveness heuristics).
- Never unlink lock files. Deleting a held lock file lets a second
  process lock a fresh inode under the same name while the first still
  holds the old one. They are zero-byte files; leave them.
- The pool is unbounded and lazy: slots (and their peer ids) are minted
  only when contention actually happens, so claiming always succeeds.

Alternatives considered and rejected for the minimum: fcntl/OFD locks
(same property, worse portability/footguns), a `BEGIN IMMEDIATE` held on
a per-slot sentinel sqlite db (portable-flock fallback, keep in back
pocket for Windows), PID/expiry heuristics (lie window after crash).

### The mapping: slot -> peer id in sqlite

New table in the CLI migration (`cli/migrations/001_init.sql`, edited in
place per the pre-release one-migration rule):

```sql
CREATE TABLE peer_slots (
  slot     INTEGER PRIMARY KEY,   -- 0, 1, 2, ...
  peer_id  INTEGER NOT NULL       -- random u64 stored as i64 cast
);
```

Mint-on-first-claim: after winning the flock for slot N, read the row;
if absent, generate a random u64 (any value; Loro accepts the full
range) and `INSERT OR IGNORE`, then re-read. The flock already excludes
same-slot races, and sqlite's write serialisation covers the rest.
Store as `i64` bit-cast (`peer as i64`), read back with `as u64`.

Device-local, not CRDT state: the mapping never enters the Loro doc.
It also survives `monoplan cache clear` deliberately - see counter
continuity below.

### Core API change: peer id is an input, not a callback

The platform acquires; core consumes. No `get_peer_id()` hook out of
core - acquisition is async on the web (Web Locks) and sync construction
across wasm/FFI wants a plain `u64` in, so the arrow points inward:

- `Doc::empty_with_peer(peer: u64)` and `Doc::new_with_peer(peer: u64)`:
  identical to the existing constructors except `inner.set_peer_id(peer)`
  is called immediately after `LoroDoc::new()`, *before* `seed_builtins`
  and *before* `make_undo_manager` (the UndoManager binds to the local
  peer at construction, and signup's seed commit must land under the
  leased peer).
- `boot_doc` / `load_doc` gain a `peer_id: Option<u64>` parameter
  (`None` = today's random behaviour, used by tests and any caller that
  hasn't adopted leasing yet). Internally: `Some(p)` ->
  `Doc::empty_with_peer(p)` before the snapshot/WAL import.
- Existing `Doc::new()` / `Doc::empty()` remain (tests, wasm wrapper
  unchanged for now).

Counter continuity: setting the peer id before importing the snapshot +
WAL is safe; Loro resumes that peer's counter from the imported oplog
high-water. `cache clear` is also safe: rebuilding from the server
replays every op the server has for this peer, so counters resume past
them; ops the server never saw also never reached any other replica, so
their counters are unobserved and reusable.

### CLI wiring (order matters)

In `Session::open_with_profile` (`cli/src/sync.rs`):

1. `open_storage(&profile)` (unchanged).
2. Claim a slot: flock scan in the profile dir, then read-or-mint the
   peer id from `peer_slots`. New module `cli/src/peer.rs`, roughly
   `fn claim(profile: &Profile, store: &SqliteStorage) -> Result<PeerLease>`
   where `PeerLease { peer_id: u64, slot: u32, _lock: std::fs::File }`.
3. `boot_doc(&storage, &dek, doc_id, Some(lease.peer_id))`.
4. `Session` holds the `PeerLease` so the lock lives as long as the doc.

Other sites:

- `signup.rs`: claim before `Doc::new()`; use `Doc::new_with_peer` (the
  builtin-seed commit is real ops and must carry the leased peer).
- `login.rs` / `recover.rs`: `Doc::empty()` seeds carry no ops, so no
  peer needed; leave as-is.
- `logout` purges the profile dir including lock files - fine, any held
  lock dies with its process and the files are recreated on demand.

### Failure/edge notes

- flock on network filesystems is unreliable, but so is sqlite itself;
  no new constraint.
- Read-only commands never commit, so their peer never enters the VV
  regardless; claiming for them anyway (via `Session::open`) is
  harmless and keeps one code path.
- Two processes, one slot each, both mutating: fine by construction -
  that is exactly the CRDT's job; the slots only guarantee the peers
  are distinct.

### Testing

- Unit (cli): two claims in one process -> distinct slots and distinct
  peer ids (flock conflicts across separate opens even within one
  process); drop the first lease -> next claim gets slot 0 again.
- Unit (core): `boot_doc` with `Some(peer)` -> `doc.peer_id() == peer`;
  mutate after boot-with-replay -> new op ids continue the peer's
  counter (no overlap with replayed history).
- Integration (`spec/testing.md` CLI driver): run N sequential mutating
  commands, assert the doc's VV has exactly one CLI peer entry.

## Web (built, interim: single-tab slot 0)

Same shape as the CLI, holding the pool at one slot until multi-tab
lands:

- Slot -> peer mapping in IndexedDB: `peer_slots` store (`web-db.ts`
  v10, a pure additive upgrade - the v9 WAL survives; the v9 `ops`
  drop/recreate is now gated on `oldVersion < 9`). Read-or-mint in one
  readwrite transaction (`peer-slots.ts`), mirroring the CLI's
  `read_or_mint_peer_slot`.
- The claim IS the single-tab Web Lock: `BrowserTabGate.tsx` now
  always requests `monoplan-single-tab` (ifAvailable) when the API
  exists, gates rendering only when enforcing, and passes `lockHeld`
  into `App`. Boot (`App.tsx`) reads the slot-0 peer concurrently with
  `IdbStorage.open` and constructs via `Doc.createWithPeer` /
  `Doc.emptyWithPeer` (new wasm exports, u64 as BigInt) only when the
  lock is held; no lock -> random peer, exactly the pre-slot behavior.
  So a gate-disabled tab (DEV second tab, `VITE_ENFORCE_SINGLE_TAB=0`,
  missing `navigator.locks`) can never share a peer with the holder.
  The `?multiTab=1` escape hatch is removed.
- The peer binds at construction - before `create`'s builtin seed
  commits and before the replay import (counter resume). There is
  deliberately no set-peer-later API: the UndoManager binds to the
  local peer at construction, and fresh-signup seed ops would otherwise
  mint under a throwaway random peer on every new device.

## Out of scope (future)
- Multi-tab web (later, decided direction): NOT a SharedWorker owning
  the db (fights the platform: OPFS sync handles are dedicated-worker
  only, SharedWorker support history is poor, and it turns every doc op
  into RPC - tried before, rejected). Instead: every tab owns its own
  wasm doc + slot peer; IndexedDB readwrite transactions serialise
  cross-tab appends (derive `local_seq` inside the txn, not from
  memory); commits fan out tab-to-tab over BroadcastChannel into
  `apply_remote` (idempotent import, remote-origin undo exclusion both
  already handle it); one leader per browser - Web Lock *without*
  `ifAvailable`, auto-handover on close - owns the single WS, snapshot
  folding, and acks. VV-derived push means the leader ships follower
  ops with no forwarding protocol; the server still sees one device.
- Hoisting the slot table/claim helper into `monoplan-storage-sqlite` for
  the Apple FFI build (trivial move under the one-migration rule).
- Shallow-snapshot/gc trimming of ancient peers as a compaction-era
  backstop for the historical peers already minted by today's
  random-per-invocation behaviour.
