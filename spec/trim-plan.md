# Trim plan (shallow compaction as a manual rug pull)

Status: plan, not built. Written 2026-10-16 from a working session that
probed Loro 1.16 directly; every "verified" claim below was checked by
running the code in the appendix against the workspace's Loro.

This is the *reference*: verified Loro facts, probe code, and the
designs that were considered. The *build* plan is `spec/trim-minimal.md`;
read that first. Its design supersedes §Design below (which has every
device wipe; the minimal plan lets devices self-trim when safe). The
facts and the appendix here remain the ground truth both documents cite.

## Vocabulary

- **Seq log.** The server's `updates` rows for a doc plus the per-doc
  `doc_sequences.next_seq` counter. Dense, gap-free, append-only. Seq is
  *delivery order*, assigned by the server, and says nothing about
  causality inside the blobs (the server cannot read them).
- **Chain.** The `snapshots` rows for a doc: one full head plus deltas.
  Bootstrap = import the chain in order, then `PullUpdates` from its frontier.
- **Horizon.** `min(last_acked_seq)` across the doc's devices.
- **Floor** (`compaction_floor_seq`). Seq at/below which op rows may be
  deleted. Also the bootstrap gate: a `PullUpdates` below it gets
  `SnapshotRequired`.
- **VV / frontier / deps.** A Loro version vector is a map
  `peer -> counter` saying which ops a doc contains. A frontier is the
  set of head op ids (usually one). Every Loro change records its
  **deps**: the frontier it was created on. Deps are fixed at creation
  and never change, no matter what the device imports later.
- **Trimmed (shallow) snapshot.** `ExportMode::ShallowSnapshot(root)`.
  Contains the full *state* but only the *history* from `root` onward.
- **Root.** The frontier a trimmed snapshot was cut at. Loro calls the
  corresponding VV `shallow_since_vv`.
- **Epoch.** Server-side label for "which root the chain head has"
  (`storage.md` §Epochs). Born 0, advances by one per accepted trim.

## Verified facts about Loro 1.16

1. **Trimming never shrinks the VV.** A doc with 51 peers, shallowed at
   its own frontier and reloaded, still has 51 peers in both `oplog_vv`
   and `shallow_since_vv`. Only the op history shrinks (2683 -> 1163
   bytes in the probe). A VV answers "which ops do I contain", and a
   trimmed doc still contains every pre-root op in materialised form, so
   no entry can be dropped. The old random-per-invocation peers from
   before `peer-id-plan.md` are therefore a permanent, small cost
   (roughly 4 bytes per peer encoded). Trimming is not the fix for VV
   width; the peer id plan is.
2. **A trimmed doc hard-rejects any op whose deps predate the root.**
   Import returns
   `Err("The dependencies of the importing updates are not included in the shallow history of the doc.")`.
   Not pending, not skipped: an error. The same op imports fine into a
   full-history doc. "Deps inside the trimmed VV" is *not* enough; the
   op must be causally *after* the root.
3. **Importing a trimmed snapshot over a doc with pre-root unsent work
   silently pends.** Import returns `Ok` with an empty success range and
   the whole snapshot parked as `pending`. The doc stays untrimmed, never
   sees the new state, and never will, because the deps it waits for were
   trimmed. No error is raised. This is the trap the bootstrap path must
   never walk into.
4. **Importing already-contained history into a trimmed doc is a no-op.**
   Full old history imported into a trimmed doc returns `Ok` with an empty
   success range. Op ids inside `shallow_since_vv` are recognised and
   skipped, so stray re-delivery of old blobs is harmless.
5. **Once trimmed, always trimmed.** A "full" export from a trimmed doc is
   shallow at the same root. The root only moves on another explicit
   trim.
6. **Exporting a shallow snapshot at a root that is not a causal cut of
   the doc produces an unloadable blob, with no error at export time.**
   A device holding an unsent op concurrent with the root (deps below it,
   VV otherwise containing it) exports `ShallowSnapshot(root)` with `Ok`.
   Loading that blob into a fresh doc fails with
   `You cannot switch a document to a version before the shallow history's start version`
   and leaves an empty doc. "Causal cut" means every op in the doc is
   either in the root's past or in its future; the failing case has an
   op that is sideways (concurrent) to the root, which can be neither
   part of the root state nor a retained op. Any shallow export must
   therefore be taken from a doc in which the root *is* a cut: not the
   live doc, but a `fork_at` of a frontier that is after the root and
   whose past excludes the sideways ops. For a device that is its
   `server_known_vv` frontier, since the producer validated that history
   (`trim-minimal.md` Part 1 step 4, Part 2 step 1). `fork_at(root)`
   itself also works but drops everything after the root too.
7. **Importing a trimmed head into a doc that already holds every op it
   carries is a no-op.** `Ok`, nothing applied, nothing pending, and the
   receiving doc stays full-history. A device does not become trimmed by
   receiving the server's new head; trimming locally is an explicit
   export-and-reload. Such a device keeps any concurrent unsent op and
   will poison the log on its next push unless it checks deps first.
8. **"VV contains the root" does not imply "unsent ops sit above the
   root".** The two-device probe has B's VV `{1: 2, 2: 1}` containing the
   root VV `{1: 2}` while B's unsent op `0@2` has deps `[0@1]`, below the
   root. Pushing it into a trimmed replica is the fact 2 hard error. The
   deps check is a separate, necessary condition.
9. **Loro cannot remove a change from its oplog.** There is no retract
   API. The only subtraction is `fork_at(frontiers)`, which the snapshot
   producer already uses (`core/src/doc.rs`, `fork_at` of the
   `server_known_vv` frontiers). Undo/revert add new ops that *depend on*
   the stale ones and are therefore equally stale.

## The two horizons are not the same problem

**Blob GC by seq is sound.** GC deletes rows every device has already
received and durably applied (seq <= horizon) and that a snapshot
already covers (seq <= chain frontier). Both are about *delivery*. What
the ops inside a blob depend on causally is irrelevant: a delivered blob
is never needed for delivery again, and a bootstrapping device gets its
contents via the snapshot's state regardless of causal age. The seq
proxy is exact here.

Note that covered-by-snapshot alone is already *sufficient* for
correctness: a device at seq 300 can bootstrap from a chain at 1000 and
merge its unsent work into the full-history doc. Holding GC at the
horizon as well is a UX policy (keep the cheap delta path for every live
device), not a safety rule, and it has the known cost that one dead
device pins the horizon until revoked. That trade is unchanged by this
plan and is a separate decision.

**The trim root is where seq stops being a proxy for causality.** By
fact 2, an op is importable into a trimmed doc only if its deps are at
or after the root. Deps are fixed at creation. Seqs are assigned at
push. A device can create ops at frontier(500), go offline, come back
after a trim, pull to 1000, ack 1000, and push ops whose deps are at
500. Its acked row says 1000. No seq-based rule can tell this device
from a fresh one. Only the client knows its own deps.

### Why the existing compatibility rule does not hold

`storage.md` §Epochs previously stated that a device acked at/above the
floor is compatible because of an "ack-after-push client invariant". The
engine has no such invariant. In `core/src/sync.rs`, acks fire from
`notify_oplog_durable` as each pulled batch becomes durable, during
`Pulling`; the post-reconnect push cannot start until `Pulling` ends
(`try_start_push` returns unless `Idle`). Acks therefore *precede* the
push. The seq check on push would pass for exactly the device described
above. That paragraph is superseded by this plan.

### The two races

- **Pull-then-push across a trim.** Device opened in epoch 0, trim lands,
  device pushes stale ops. Caught by carrying the epoch on the push.
- **Push lands before the trim is accepted.** Producer exports at seq N.
  Before the server accepts, a stale push lands at N+1 in the old epoch,
  legitimately. Trim is accepted with `up_to_seq = N`. Every device that
  bootstraps from the new head then pulls N+1 and hits fact 2. Nothing
  was out of order; no epoch check fires. Caught only by the server
  refusing a trim unless the log is still at `up_to_seq`.

## Design: manual trim, every device wipes

The honest framing: trimming cuts history, and a cut history means every
replica re-lists. This is what Dropbox does on a cursor reset. We say it
out loud instead of hiding it behind "sync", and we make it a deliberate
user action because it can discard work on devices that are behind,
which breaks the offline promise. Making it manual also lets the user
pick a moment when all devices are synced, which collapses most of the
machinery a continuous design would need (see §Rejected).

### Rules

- **T1. Trim at the top.** The producing device must be fully caught up:
  `last_acked_seq == server_last_seq` and nothing unsent. The root is the
  producer's `server_known_vv` frontier, `up_to_seq` is the latest seq,
  and the floor equals `up_to_seq`. Nothing sits above the floor, so
  there is no causal-cut computation.
- **T2. Server accepts only if the log is still at `up_to_seq`.**
  `server_last_seq == up_to_seq` at accept time, checked under the same
  row lock as `insert_updates`. Otherwise reject; the client re-pulls and
  retries. This is the whole defence against the second race.
- **T3. Accept = new epoch.** In one transaction: insert the trimmed row
  (`from_seq NULL`, `epoch = old + 1`, `compaction_floor_seq = up_to_seq`,
  `up_to_vv` = sealed root VV as today), set `doc_sequences.epoch`, and
  broadcast `EpochAdvanced { doc_id, epoch }` to every connection with the
  doc open. Op rows at/below `up_to_seq` are GC-eligible immediately: no
  device will ever delta-pull across the boundary (T5), and in-flight
  pushes from behind devices are rejected by T4 rather than deduplicated.
  The horizon plays no part in a trim.
- **T4. Pushes carry the epoch; mismatches are rejected.** `PushUpdates`
  gains `epoch`. The server rejects with `EpochMismatch { epoch }` when it
  differs from `doc_sequences.epoch`. There is no partial acceptance.
- **T5. Every device wipes on epoch change.** On `DocOpened` with an
  epoch above the stored one, on `EpochAdvanced`, or on `EpochMismatch`:
  discard the local doc, WAL, in-flight push and sync cursor (the same
  wipe as `monoplan db clear --force`), persist the new epoch, and run
  the existing bootstrap path from an empty doc. Not just the behind
  devices: all of them. One uniform rule, and the existing bootstrap code
  works unchanged because importing a chain into an *empty* doc cannot
  hit fact 3.
- **T6. Unsent work on a behind device is discarded, loudly.** The wipe
  emits an event the host surfaces ("N unsynced changes were discarded
  because the doc was trimmed"). Semantic replay (materialise the diff
  between `fork_at(server_known_vv)` and the current doc, replay as fresh
  ops after bootstrap) is a possible later refinement, not part of this
  plan.
- **T7. Ids survive.** Item ids are data and live in the snapshot. Loro op
  ids and peer ids survive because a shallow snapshot keeps the id space
  (fact 4 makes stray old blobs harmless). Peer slots are untouched.
- **T8. Automatic snapshotting continues inside the epoch.** The trimmed
  row is the chain head; the orchestrator chains deltas onto it exactly
  as it does onto a full head today. Later "full" heads produced inside
  the epoch are shallow at the same root (fact 5) and that is fine.

### Trigger and UX

A CLI command, `monoplan trim`, run on a device that is fully synced:

1. Sync to the top. Refuse if anything is unsent.
2. Fetch `GET /api/devices` and list every device whose frontier is
   below `server_last_seq`, with its last-seen time. Say plainly that any
   unsynced changes on those devices will be discarded. The server cannot
   know whether they *have* unsynced changes, so the warning is
   conservative.
3. Confirm on a TTY; refuse on non-TTY without `--force`.
4. `fork_at(server_known_vv frontiers)`, export
   `ShallowSnapshot(frontiers)`, seal, send
   `TrimDoc { doc_id, up_to_seq, up_to_vv, blob }`.
5. On `TrimAccepted { epoch }` the sender wipes and bootstraps like
   everyone else (T5). On `TrimRejected` because the log moved, pull and
   go to step 1.

No automatic trigger. No admin endpoint: the server cannot produce the
blob, so the trigger has to be a client that holds the DEK.

### Wire additions (edit in place, no version bump)

| Frame | Direction | Shape |
|---|---|---|
| `PushUpdates` | c->s | gains `epoch: u64` (T4) |
| `EpochMismatch` | s->c | `{ doc_id, epoch }` — reply to a stale `PushUpdates` |
| `TrimDoc` | c->s | `{ doc_id, up_to_seq, up_to_vv: EncryptedBlob, blob: EncryptedBlob }` |
| `TrimAccepted` / `TrimRejected` | s->c | `{ doc_id, epoch }` / `{ doc_id, reason }` |
| `EpochAdvanced` | s->c | `{ doc_id, epoch }` broadcast on accept (T3) |

`DocOpened` already carries `epoch`. The client persists the epoch next
to its sync cursor.

### What this does and does not buy

Buys: a chain head of bounded size, bootstrap cost proportional to
state rather than history, a seq log that can be emptied below the head,
and an engine with no causal reasoning in it at all.

Does not buy: a smaller VV (fact 1), or continuity for up-to-date
devices (they re-download once per trim). For one user with small docs
and a trim every year or so, that is one chain download per device per
year.

## Rejected alternatives

### A. Continuous trimming with continuity for up-to-date devices

The design previously sketched in `sync-protocol.md` §Shallow snapshots:
server computes a VV meet from reported device VVs, trims at a floor
below the head, devices above the floor keep going. To be sound it needs
four guards, each covering what the others cannot:

1. Producer picks a root that is a *causal cut* of every op it knows.
   VV-at-floor is not one: an old op already in the log above the floor
   is concurrent with it, not after it. Compute: start from VV-at-floor,
   intersect with the deps of every change not inside it.
2. Server accepts the trim only if the log is still at `up_to_seq` (T2).
3. Client checks the deps of all unsent changes against the root before
   its first push after any epoch change; rebase or discard on failure.
   Needs the sealed root handed to the client on `DocOpened`.
4. Push carries the epoch it checked against; server rejects mismatches
   (T4).

Guards 2 and 4 are seq/epoch bookkeeping a dumb server can do. Guards 1
and 3 are causal and only a client can do them. It is tractable, but it
is four invariants that must all hold, for the benefit of saving
up-to-date devices one download per trim. Not worth it for this product.
The question of what Loro does when asked to shallow-export at a root
that is not a causal cut is answered by fact 6: it produces a blob that
cannot be loaded. Any producer in design A would have to export from
`fork_at(root)` and compute the cut first.

A side finding from this analysis: the `ReportVV` mechanism is
unnecessary even for A. The chain already stores a sealed `up_to_vv` at a
known seq per row; the root can be taken from the latest row at/below
the floor and then cut as in guard 1.

### B. Doc rebirth

Export materialised state into a brand-new `LoroDoc` with one peer and
call that the new epoch. It makes "anything before the cut can go to
hell" literally true: old-doc ops are meaningless by construction, no
causal checks anywhere, and the VV resets to one entry. Cost: the same
wipe-everyone rule as this plan, plus a real hazard. Same peer ids with
counters restarting at zero means an old-doc update reaching a new-doc
replica is not rejected but silently misread as different ops with the
same ids, which is the one unrecoverable corruption in Loro. Mitigation
would be epoch-namespaced local storage and re-minted peer slots per
epoch. Rejected for now because the VV win is small (fact 1 sizes it in
bytes) and the collision hazard is a new class of bug. Keep in mind if
VV width ever becomes measurable.

## Edits this plan supersedes

- `storage.md` §Epochs, the "Compatibility rule" paragraph: replaced by a
  pointer here. The invariant it relied on does not exist.
- `sync-protocol.md` §Shallow snapshots (future): the VV-meet / `ReportVV`
  design is replaced by this plan.
- `sync-protocol.md` §Bootstrap step 2 ("Loro merges — local-only commits
  not yet pushed are preserved automatically"): true for full heads,
  false for trimmed heads (fact 3). Under T5 the client bootstraps into
  an empty doc after an epoch change, so the step is reached only with a
  compatible doc.

## Appendix: probes

Run from the workspace with
`cargo test -p monoplan-core --test zz_probe -- --nocapture` after
saving as `core/tests/zz_probe.rs`; delete the file afterwards.

```rust
use loro::{ExportMode, LoroDoc};
use std::borrow::Cow;

#[test]
fn vv_width_after_shallow() {
    let hub = LoroDoc::new();
    hub.set_peer_id(1).unwrap();
    hub.get_map("m").insert("seed", 0).unwrap();
    hub.commit();
    for i in 0..50u64 {
        let d = LoroDoc::new();
        d.set_peer_id(1_000_000 + i).unwrap();
        d.import(&hub.export(ExportMode::Snapshot).unwrap()).unwrap();
        d.get_map("m").insert(&format!("k{i}"), i as i64).unwrap();
        d.commit();
        hub.import(&d.export(ExportMode::all_updates()).unwrap()).unwrap();
    }
    let shallow = hub
        .export(ExportMode::ShallowSnapshot(Cow::Owned(hub.oplog_frontiers())))
        .unwrap();
    let re = LoroDoc::new();
    re.import(&shallow).unwrap();
    // Expect: 51 peers before and after; only the byte size drops.
    println!("full peers={} shallow peers={} shallow_since={} bytes {}->{}",
        hub.oplog_vv().len(), re.oplog_vv().len(), re.shallow_since_vv().len(),
        hub.export(ExportMode::Snapshot).unwrap().len(), shallow.len());
}

#[test]
fn stale_push_into_shallow() {
    let hub = LoroDoc::new();
    hub.set_peer_id(1).unwrap();
    hub.get_map("m").insert("a", 1).unwrap();
    hub.commit();

    // Stale device forks at an early version.
    let stale = LoroDoc::new();
    stale.set_peer_id(99).unwrap();
    stale.import(&hub.export(ExportMode::Snapshot).unwrap()).unwrap();
    let stale_base = stale.oplog_vv();

    for i in 0..5 {
        hub.get_map("m").insert(&format!("b{i}"), i).unwrap();
        hub.commit();
    }
    let shallow = hub
        .export(ExportMode::ShallowSnapshot(Cow::Owned(hub.oplog_frontiers())))
        .unwrap();
    let trimmed = LoroDoc::new();
    trimmed.import(&shallow).unwrap();

    stale.get_map("m").insert("from_stale", 42).unwrap();
    stale.commit();
    let stale_ops = stale.export(ExportMode::updates(&stale_base)).unwrap();

    // Fact 2: hard error.
    println!("trimmed <- stale: {:?}", trimmed.import(&stale_ops).map_err(|e| e.to_string()));
    // Full-history doc: fine.
    println!("hub <- stale: {:?}", hub.import(&stale_ops).map(|s| format!("{s:?}")));
    // Fact 3: Ok, but everything pending; doc never advances.
    println!("stale <- shallow: {:?}", stale.import(&shallow).map(|s| format!("{s:?}")));
    println!("stale has b4: {} is_shallow: {}",
        stale.get_map("m").get("b4").is_some(), stale.is_shallow());
}
```

Two-device probe (2026-10-17): B holds everything A has plus an unsent op
concurrent with A's trim root.

```rust
#[test]
fn two_devices_concurrent_under_trim() {
    use loro::VersionVector;
    let a = LoroDoc::new(); a.set_peer_id(1).unwrap();
    a.get_map("m").insert("v", "a").unwrap(); a.commit();          // 0@1
    let b = LoroDoc::new(); b.set_peer_id(2).unwrap();
    b.import(&a.export(ExportMode::Snapshot).unwrap()).unwrap();
    let b_server_known = b.oplog_vv();
    b.get_map("m").insert("v", "b").unwrap(); b.commit();          // 0@2, deps [0@1], unsent
    a.get_map("m").insert("v", "c").unwrap(); a.commit();          // 1@1
    b.import(&a.export(ExportMode::all_updates()).unwrap()).unwrap(); // B is up to date
    let root = a.oplog_frontiers();                                // [1@1]
    let trim_blob = a.export(ExportMode::ShallowSnapshot(Cow::Owned(root.clone()))).unwrap();
    let at = LoroDoc::new(); at.import(&trim_blob).unwrap();
    // (1) fact 2 with an up-to-date pusher: Err(dependencies ... not included)
    println!("{:?}", at.import(&b.export(ExportMode::updates(&b_server_known)).unwrap()).map_err(|e| e.to_string()));
    // (2) fact 7: Ok, nothing applied, B stays full-history
    println!("{:?} shallow={}", b.import(&trim_blob).map(|s| format!("{s:?}")), b.is_shallow());
    // (3) fact 6: export Ok, load fails, doc empty
    let blob = b.export(ExportMode::ShallowSnapshot(Cow::Owned(root))).unwrap();
    let bt = LoroDoc::new();
    println!("{:?} since={:?}", bt.import(&blob).map_err(|e| e.to_string()), bt.shallow_since_vv());
}
```

Observed:

```
Err("Import Failed: The dependencies of the importing updates are not included in the shallow history of the doc.")
Ok("ImportStatus { success: VersionRange({}), pending: None }") shallow=false
Err("You cannot switch a document to a version before the shallow history's start version.") since=ImVersionVector({})
```

Part 2 oracle probe (2026-10-17), for `trim-minimal.md`: a device
self-trims by forking at its server-known frontier, shallow-exporting at
the root, loading that, and importing its unsent work. Safe device
(unsent op created above the root) gets `Ok` and a trimmed doc with its
work intact. Unsafe device (unsent op created below the root) gets the
fact 2 error and the trimmed fork is left untouched and valid.

```rust
fn self_trim(live: &LoroDoc, server_known: &loro::VersionVector, root_vv: &loro::VersionVector) -> (String, LoroDoc) {
    let fork = live.fork_at(&live.vv_to_frontiers(server_known)).unwrap();
    let root = fork.vv_to_frontiers(root_vv);
    let blob = fork.export(ExportMode::ShallowSnapshot(Cow::Owned(root))).unwrap();
    let f = LoroDoc::new();
    f.import(&blob).unwrap();
    let unsent = live.export(ExportMode::updates(server_known)).unwrap();
    let res = match f.import(&unsent) { Ok(s) => format!("Ok({s:?})"), Err(e) => format!("Err({e})") };
    (res, f)
}

#[test]
fn part2_oracle_both_branches() {
    let a = LoroDoc::new(); a.set_peer_id(1).unwrap();
    a.get_map("m").insert("v", "a").unwrap(); a.commit();          // 0@1
    let root_vv = a.oplog_vv();                                     // root for the safe case: [0@1]
    a.get_map("m").insert("v", "b").unwrap(); a.commit();          // 1@1

    // Safe: pulled everything, then wrote offline (deps [1@1], above root).
    let safe = LoroDoc::new(); safe.set_peer_id(2).unwrap();
    safe.import(&a.export(ExportMode::Snapshot).unwrap()).unwrap();
    let safe_known = safe.oplog_vv();
    safe.get_map("m").insert("v", "safe").unwrap(); safe.commit();
    let (res, f) = self_trim(&safe, &safe_known, &root_vv);
    println!("safe: {res} v={:?} shallow={}", f.get_map("m").get("v").map(|x| x.into_value().unwrap()), f.is_shallow());

    // Unsafe: wrote offline at 0@1 (deps [0@1]), then pulled 1@1. Root is [1@1].
    let root2_vv = a.oplog_vv();
    let early = { let t = LoroDoc::new(); t.set_peer_id(1).unwrap(); t.get_map("m").insert("v", "a").unwrap(); t.commit(); t.export(ExportMode::Snapshot).unwrap() };
    let unsafe_ = LoroDoc::new(); unsafe_.set_peer_id(3).unwrap();
    unsafe_.import(&early).unwrap();
    unsafe_.get_map("m").insert("v", "unsafe").unwrap(); unsafe_.commit();
    unsafe_.import(&a.export(ExportMode::all_updates()).unwrap()).unwrap();
    let unsafe_known = { let mut v = unsafe_.oplog_vv(); v.remove(&3); v };
    let (res, f) = self_trim(&unsafe_, &unsafe_known, &root2_vv);
    println!("unsafe: {res} v={:?}", f.get_map("m").get("v").map(|x| x.into_value().unwrap()));
}
```

Observed:

```
safe: Ok(ImportStatus { success: VersionRange({2: (0, 1)}), pending: None }) v=Some(String(LoroStringValue("safe"))) shallow=true
unsafe: Err(Import Failed: The dependencies of the importing updates are not included in the shallow history of the doc.) v=Some(String(LoroStringValue("b")))
```

Observed 2026-10-16 on Loro 1.16 (first two probes):

```
full peers=51 shallow peers=51 shallow_since=51 bytes 2683->1163
trimmed <- stale: Err("Import Failed: The dependencies of the importing updates are not included in the shallow history of the doc.")
hub <- stale: Ok("ImportStatus { success: VersionRange({99: (0, 1)}), pending: None }")
stale <- shallow: Ok("ImportStatus { success: VersionRange({}), pending: Some(VersionRange({1: (5, 6)})) }")
stale has b4: false is_shallow: false
```
