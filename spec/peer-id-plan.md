# Peer ids

Status: built. One stable Loro peer id per doc per device, stored on the
doc's own local row (2026-11-19). This replaced the slot pool described
under "History" at the end.

Note for existing native profiles: `peer_id` was added to the `docs`
table in `crates/storage-sqlite/migrations/001_init.sql` in place, and
`peer_slots` removed from `cli/migrations/001_init.sql` (pre-release
rule). A profile created before that lacks the column and fails on
open: sign out and in again (`monoplan auth logout`, or the app's sign
out) rather than adding a bridge migration. The web upgrades itself
(IndexedDB v11).

## Problem

A Loro doc constructed without a peer id gets a random one. Every peer
that ever commits adds a permanent entry to the doc's version vector,
and VVs are carried forever in retained history and shipped and compared
throughout sync (`server_known_vv`, snapshot metadata, push `from_vv` /
`to_vv`). A device that minted a fresh peer on every process start or
page load would add thousands of entries over a doc's life.

So a device reuses one peer id per doc. That is only sound under two
rules.

## The two rules

1. **One live instance.** Two live docs committing under the same peer
   mint duplicate `(peer, counter)` op ids. Loro states this in its
   `set_peer_id` docs: a fixed peer id requires single ownership.
2. **The peer never outlives its local history.** Loro resumes a peer's
   counter from the ops it has imported. A device that still holds
   every op its peer wrote resumes past all of them. A device that lost
   them (local copy wiped, doc left and rejoined) and is fetching them
   back resumes from wherever the download has got to, and a local
   write in that window reuses an op id the server already holds.
   Loro 1.17 rejects the conflicting import (`UsedOpID`), the replica
   cannot finish its bootstrap, and the doc stops syncing.

Rule 2 is the one the slot pool broke: see "History".

## Design

The peer id is a field of the doc's local row. It is minted when the row
is created and it goes when the row goes, so rule 2 holds by
construction instead of by every wipe path remembering it.

| | Where | Minted | Dropped |
|---|---|---|---|
| Native | `docs.peer_id` (`crates/storage-sqlite/migrations/001_init.sql`), read by `SqliteStorage::doc_peer_id` | column default, on row insert | `forget_doc` deletes the row; `clear_db` keeps the row and re-mints in the same transaction |
| Web | `DocRow.peerIdHex` in the `docs` store, read by `IdbStorage.peerId()` | in the transaction that creates the row (`IdbStorage.open`) | `IdbStorage.wipe` and `wipeMonoplanDb` delete the row |

Anything new that deletes a doc's WAL and snapshot while keeping its row
must re-mint the peer in the same transaction, as `clear_db` does.

The id is a random value: 63 bits on native (sqlite's `random()` masked
non-negative), 64 on web. Loro reserves `u64::MAX`; neither can produce
it. It is device-local state and never enters the CRDT doc.

Core takes the peer as an input: `Doc::new_with_peer` /
`Doc::empty_with_peer`, and `boot_doc(.., Some(peer))`. It must be bound
at construction, before the builtin seed commits and before the replay
import, because the UndoManager binds to the local peer at construction
and the seed is real ops. There is no set-peer-later API.

### Rule 1: who holds the docs

- **Native.** `engine.lock` (`cli/src/daemon.rs`): the daemon, and the
  Apple apps' in-process engine, hold it for as long as the docs are
  open. The pre-daemon writers that commit a seed (`signup`, the
  local-only bootstrap, `db clear` on a local-only workspace) hold the
  same lock while they write. `Engine::open_with_profile` only checks
  that no daemon is running and takes no lock of its own; it is used by
  tests alone.
- **Web.** The `monoplan-single-tab` Web Lock (`BrowserTabGate.tsx`). A
  tab that renders without it (dev second tab, no `navigator.locks`)
  boots every doc on a random peer and never reads the stored one.

### What this costs

One VV entry per doc per device per lifetime of that device's local
copy. A sign-out and sign-in, a leave and rejoin, or a `db clear` adds
one.

## Known gap

Rule 2 assumes every op the peer pushed is also in local storage. On
native the WAL append is synchronous and precedes the send. On web the
IndexedDB write is queued and can still be in flight when the frame goes
out, so a crash in that window leaves the server holding ops the local
copy lacks. The next boot pulls them back, and a local write before that
pull completes would collide. Not fixed.

## Not built

- Multi-tab web. Each extra tab needs a peer of its own for the docs it
  holds; a random peer per tab session is the simple answer.
- Trimming ancient peers out of the VV (`spec/trim-plan.md`).

## History

Until 2026-11-19 the peer id belonged to the device, not the doc: a
`peer_slots` table (sqlite) or store (IndexedDB) mapped a slot number to
a peer id, and a process claimed the lowest free slot with a flock on
`peer-<slot>.lock` (web: slot 0, claimed by the single-tab lock). The
pool existed because the CLI was then one process per command and two
could run at once; the daemon (`spec/daemon-plan.md`) ended that, and
nothing has claimed a slot above 0 since.

The mapping deliberately survived a local wipe, on the reasoning that
rebuilding from the server replays every op the peer wrote, so its
counter resumes past them. That holds only once the rebuild has
finished. On 2026-11-18 a web sign-out and sign-in wiped the docs and
kept the slot; the snapshot chain arrived as twelve frames, a local
write landed partway through, and it took op id 52140 on a peer
whose real op 52140 was in a later frame. The Home doc could no longer
bootstrap in that browser until its site data was cleared.
