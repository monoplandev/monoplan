# Trim, minimal version

Status: plan, not built. Written 2026-10-17. This is the *build* plan.
The *reference* (verified Loro facts, probe code, rejected designs) is
`spec/trim-plan.md`; this document only cites it. Read this one first.

The plan does three things, in order of build:

1. Pick a trim point that is safe for most devices and still removes
   most history, produce the trimmed snapshot, upload it.
2. Let every other device trim itself locally when that is safe.
3. When it is not safe, have the device load the trimmed snapshot from
   scratch, and either discard its stranded changes or show them to the
   user first.

## Mental model in one example

One device, one value, written three times: `a`, `b`, `c`. Three ops,
ids `0@1`, `1@1`, `2@1`. The frontier (the head) is `2@1`.

A **trim at `2@1`** produces a doc that holds:

- the **state**, `v = c`, as a given, not derived from anything;
- the **retained head op** `2@1`, a real op, so later ops can name it in
  their deps;
- the **ids** of `0@1` and `1@1`, with no content, in a version vector
  called `shallow_since_vv`. The doc knows they happened, not what they
  were.

The frontier the trim was cut at is the **root**. Everything below the
root is gone. Everything from the root up is ordinary ops.

The one rule that makes trimming dangerous: **an op can only be imported
into a trimmed doc if it is causally after the root.** An op's deps are
fixed when it is created and never change. If a device created an op
while it was at `1@1`, that op depends on `1@1`, which is below the
root, and every trimmed replica will refuse it with a hard error, even
though the op itself may be about a completely unrelated item.
(`trim-plan.md` facts 2 and 8.)

Two more facts that shape everything below:

- Receiving the trimmed snapshot does **not** trim you. A full-history
  doc that already holds the snapshot's ops treats it as a no-op. Trimming
  yourself is an explicit step. (fact 7)
- Asking Loro to shallow-export at a root while your doc holds an op
  concurrent with that root gives a blob that exports fine and **cannot
  be loaded**. Always export from a `fork_at(root)`-derived doc and
  always load what you exported before trusting it. (fact 6)

## Vocabulary you need

- **seq**: server-assigned delivery number of an encrypted update blob.
- **horizon**: the smallest `last_acked_seq` across the doc's devices.
  Every device has everything at or below it.
- **chain**: the server's snapshot rows: one full head, then deltas. Each
  row stores `up_to_seq` and the producer's sealed version vector at that
  seq, `up_to_vv`.
- **root**: the frontier a trim is cut at. Stored as a sealed version
  vector; a device holding the history converts it to frontiers with
  `vv_to_frontiers`.
- **epoch**: server-side counter, +1 per accepted trim. Already on the
  wire in `DocOpened`.
- **fork_at(frontiers)**: a new independent doc containing exactly the
  causal past of those frontiers. Ops after or concurrent with them are
  absent. The producer already uses it to keep unsent work out of
  snapshots (`core/src/doc.rs`, `snapshot_blob_at`).

## Part 1: trim point and upload

### Where to cut

The dial has two ends.

- **At the head** (the producer's current frontier): removes the most
  history. Any device holding unsent work at that moment is unsafe,
  because its unsent ops are concurrent with the root.
- **At a point every device has already seen**: removes less history. A
  device is unsafe only if it has unsent work *older than that point*,
  which for a point a few hundred ops back is rare.

Default: **the newest chain row whose `up_to_seq` is at or below the
horizon.** Its `up_to_vv` is the root. Every device has acked past it, so
every device's version vector contains it, and only unsent work created
before the device pulled past that row is stranded. This is "safe and
effective": the trim lags the head by at most one snapshot interval, and
the horizon already exists.

**Safety property of the default.** The horizon counts every device,
dead or new (no frontier row reads as 0). A device's unsent work was
created on a frontier that included everything it had acked, so it sits at
or above any point at or below the horizon. Under the default trim point
no device's unsent work is ever sideways, with one exception: a device
that pulled, acked, and crashed before pushing, whose ack claims more
than its unsent work's deps do. That window is the gap between the end of
`Pulling` and the first `PushUpdates`. Everything else that can strand work
requires `--at-head`, an explicit act. Offline use is therefore protected
by default, not by a setting: a device in a drawer pins the horizon until
the owner revokes it or trims at the head, and both prompts name it.

If no chain row sits at or below the horizon (a device is far behind and
pinning it), the trim refuses and names the devices. The user can revoke
them, or pass `--at-head` to trim at the producer's frontier and accept
that behind devices will be unsafe. Both are explicit choices.

### Producing the blob

On the producing device, which must be fully synced (`last_acked_seq ==
server_last_seq`):

1. Get the root: decrypt the chosen chain row's `up_to_vv`, convert to
   frontiers against the local doc.
2. `fork_at(server_known_vv frontiers)`. This drops the producer's own
   unsent work, exactly as today's snapshot production does.
3. On the fork, `export(ShallowSnapshot(root))`. State at the head,
   history from the root up.
4. **Load the blob into a fresh doc.** If loading fails, the root is not
   a causal cut of the history (an old op above the root in the log is
   concurrent with it, fact 6). Fall back to the next older chain row
   and retry from step 1. If no row works, refuse with a message.
5. Seal and send `TrimDoc { doc_id, up_to_seq, up_to_vv, root_vv, blob }`
   where `up_to_seq` is the producer's `last_acked_seq`, `up_to_vv` its
   `server_known_vv`, and `root_vv` the root.

### Server acceptance

Two checks, both cheap, both under the per-doc row lock:

- `server_last_seq == up_to_seq`. If any op landed while the producer was
  exporting, reject with `TrimRejected { reason: "log moved" }`; the
  producer pulls and retries. This is what stops an op that landed in
  the window from being below a trim nobody validated it against.
- The sending device is fully acked.

On accept, in one transaction: insert the snapshot row (`from_seq NULL`,
`epoch = old + 1`, `compaction_floor_seq = horizon` as for any snapshot,
`up_to_vv`, plus the new sealed `root_vv`), set `doc_sequences.epoch`,
broadcast `EpochAdvanced { doc_id, epoch, root_vv }`. Reply
`TrimAccepted { epoch }`.

Op-row GC is unchanged: horizon-based, as today. The trim does not touch
it. Old chain rows may be pruned once the trimmed head exists; not
required for v1.

### The one guard on the hot path

`PushUpdates` gains `epoch`. The server rejects a push whose epoch is not the
doc's current one with `EpochMismatch { epoch, root_vv }`. The device then
runs Part 2 before pushing again. This is the only trim-related check on
the normal push path, and it is a single integer compare.

A device learns of a new epoch three ways: `DocOpened` (epoch above the
one it stored), `EpochAdvanced`, or `EpochMismatch`. All three carry the
root. All three lead to Part 2. **A device never pushes after learning
of a new epoch until Part 2 has run.**

## Part 2: self-trim when safe

A device is safe to self-trim when two things hold:

- its version vector contains the root (true for every device at or
  above the horizon when the default trim point is used);
- every unsent change is causally after the root.

Checking the second condition by inspecting deps is fiddly, because Loro
records deps per change, not per op, and unsent ops can share a change
with ops the server already has. So do not inspect. **Use Loro's import
rule as the oracle:**

1. `fork_at(server_known_vv frontiers)`. Server-known history only.
2. `export(ShallowSnapshot(root))` from the fork, load into a fresh doc
   `F`. Loading succeeds, because the producer validated this exact
   history in Part 1 step 4 and the device holds a prefix of it.
3. From the live doc, `export(updates since server_known_vv)`: the
   unsent work.
4. Import that into `F`.
   - **Ok**: safe. `F` is now the device's doc, trimmed at the root, with
     its unsent work intact. Set the leased peer id on `F` (forks get a
     random one). Write `F` as the local checkpoint, prune the WAL, store
     the epoch. Carry on: pull from `last_acked_seq`, push with the new
     epoch.
   - **Err** (the fact 2 message): unsafe. `F` is untouched and still
     valid. Keep it; go to Part 3.

Cost: one fork and one export of the local doc. Runs once per trim per
device, which is rare. Both branches verified (`trim-plan.md` appendix,
"Part 2 oracle").

## Part 3: when unsafe

The device holds unsent changes that cannot exist in the new epoch. Two
options, both start the same way.

### v1: log what would be lost, then discard

At the moment Part 2 returns `Err`, before adopting the fork, both states
are in hand. Write one **stranded-changes record**:

- the raw unsent blob, the bytes from `export(updates since
  server_known_vv)`. Meaningless without the pre-trim history, so this is
  forensics, not recovery; it is cheap, keep it;
- the item-level diff as data: materialise the fork (`fork_at(server_known)`,
  the state the server has) and the live doc (the state the user saw),
  and serialise the touched `Item`s before and after. Created items,
  changed fields, lifecycle and list moves, deletions. This is the part a
  human can read and the part a later replay would consume;
- context: timestamp, old and new epoch, device id, change count.

Store it sealed under the DEK as a row in the existing local store
(sqlite on the CLI, IndexedDB on the web), like the WAL rows. A plaintext
file in the profile dir would leak item titles at rest, which nothing
else in the local store does.

Then **discard**: adopt `F` from Part 2 as the live doc. It already *is*
the server-known state trimmed at the root, so no download is needed.
Set the leased peer id on it, write it as the checkpoint, drop the WAL and
the in-flight push, store the new epoch, then `PullUpdates` from
`last_acked_seq` as normal. (A device below the compaction floor never
reaches Part 2; it gets `SnapshotRequired` and bootstraps from the chain
into an empty doc as today.)

User-facing: one line. "N local changes could not be kept after a trim.
Run `monoplan stranded` to see them." That command decrypts and prints
the records. On the web, a console warning and a count badge.

### Later: re-apply

After the discard above, replay the diff as fresh ops: set the fields,
create the items, move the lifecycle, using the same commands the CLI
already exposes. Last-writer-wins against anything that happened in
between; an item deleted meanwhile is skipped and reported. Not v1. The
diff in the stranded record is shaped so it can drive this.

Either way the device ends up trimmed at the root, in the new epoch, with
no stranded ops, and the user was told.

## What you need to remember

Five sentences.

1. An op is importable into a trimmed doc only if it is causally after
   the root; deps never change after creation.
2. The producer picks the root from a chain row at or below the horizon,
   exports from a fork, and loads what it exported before uploading.
3. The server accepts a trim only if the log has not moved, then bumps
   the epoch; pushes carry the epoch and mismatches are rejected.
4. Every device, on learning of a new epoch, tries to self-trim by
   importing its unsent work into a freshly trimmed fork; Ok means safe.
5. If that import fails, the device logs what is stranded (sealed, with
   an item-level diff), tells the user in one line, adopts the trimmed
   fork anyway and carries on.

## Deliberately left out

- Semantic re-apply of stranded changes (Part 3, "Later"). v1 logs them.
- Pruning old chain rows after a trim.
- Any change to op-row GC.
- Doc rebirth (fresh `LoroDoc`, VV reset). See `trim-plan.md` §Rejected.
- Trimming the VV itself. Not possible with shallow snapshots
  (`trim-plan.md` fact 1); the peer id plan is the fix for VV width.
- Automatic trim scheduling. v1 is a CLI command, `monoplan trim`, with
  `--at-head` as the only option.

## Wire additions (edit in place, no version bump)

| Frame | Dir | Shape |
|---|---|---|
| `PushUpdates` | c→s | gains `epoch: u64` |
| `EpochMismatch` | s→c | `{ doc_id, epoch, root_vv: EncryptedBlob }` |
| `TrimDoc` | c→s | `{ doc_id, up_to_seq, up_to_vv, root_vv, blob }` (all blobs sealed) |
| `TrimAccepted` / `TrimRejected` | s→c | `{ doc_id, epoch }` / `{ doc_id, reason }` |
| `EpochAdvanced` | s→c | `{ doc_id, epoch, root_vv: EncryptedBlob }` |
| `DocOpened` | s→c | gains `root_vv: Option<EncryptedBlob>` (`None` in epoch 0) |

Storage: `snapshots` gains `root_vv BLOB, root_vv_nonce BLOB` (NULL on
untrimmed rows). Edit `001_init.sql` in place per the pre-release rule.
