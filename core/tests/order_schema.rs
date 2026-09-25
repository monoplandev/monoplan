//! Randomised + large-history invariant tests for the v2 per-list
//! ordering schema (`spec/data-model.md`).
//!
//! Two complementary harnesses:
//!
//! - `randomized_multi_peer_convergence` — N docs, a deterministic
//!   pseudo-random stream of mutations (adds, reorders, cross-list
//!   moves, lifecycle flips, deletes, list churn, undo/redo, reconcile)
//!   interleaved with random pairwise syncs, then a full mesh sync.
//!   Properties: every doc's projection invariants hold after every
//!   op, a naive event-consumer mirror (the JS store contract) never
//!   drifts from the doc, and all fingerprints converge at the end.
//! - `large_synthetic_history_many_peers_lists_moves_undos` — the
//!   shape of the retired move-undo repro: sequential sessions (each a
//!   fresh Loro peer booted from the accumulated oplog rows) doing
//!   bulk adds, multi-select moves, undos, then capture; finally a
//!   snapshot save/load round-trip. This is the workload that used to
//!   trap the wasm engine under the v1 global MovableList.

use std::collections::{HashMap, HashSet};

use monoplan_core::crypto::Dek;
use monoplan_core::doc::{Doc, ItemLifecycle, LIST_INBOX};
use monoplan_core::events::AppEvent;

// ---------- deterministic rng ----------

struct Lcg(u64);

impl Lcg {
    fn new(seed: u64) -> Self {
        Lcg(seed
            .wrapping_mul(2862933555777941757)
            .wrapping_add(3037000493))
    }
    fn next(&mut self) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        self.0 >> 33
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() as usize) % n.max(1)
    }
    fn chance(&mut self, pct: usize) -> bool {
        self.below(100) < pct
    }
}

// ---------- naive consumer mirror (the JS store contract) ----------

/// Mirrors `js/web/src/sync/store.ts` dispatch semantics: per-list
/// open arrays, one remove-then-insert-at-open_index per event, in
/// emission order. If this drifts from the doc's own projection, the
/// emitted event stream was wrong even though the CRDT converged.
#[derive(Default)]
struct Mirror {
    /// item id → (list_id, is_live)
    items: HashMap<String, (String, bool)>,
    open: HashMap<String, Vec<String>>,
}

impl Mirror {
    fn materialize(doc: &Doc) -> Self {
        let mut m = Mirror::default();
        for item in doc.all_items() {
            let open = item.is_open();
            if open {
                m.open
                    .entry(item.list_id.clone())
                    .or_default()
                    .push(item.id.clone());
            }
            m.items.insert(item.id, (item.list_id, open));
        }
        m
    }

    fn remove_open(&mut self, list_id: &str, id: &str) {
        if let Some(arr) = self.open.get_mut(list_id) {
            arr.retain(|x| x != id);
            if arr.is_empty() {
                self.open.remove(list_id);
            }
        }
    }

    fn insert_open(&mut self, list_id: &str, id: &str, at: Option<usize>) {
        let arr = self.open.entry(list_id.to_string()).or_default();
        arr.retain(|x| x != id);
        let at = at.unwrap_or(arr.len()).min(arr.len());
        arr.insert(at, id.to_string());
    }

    fn apply(&mut self, doc: &Doc, ev: &AppEvent) {
        match ev {
            AppEvent::FullResync => *self = Mirror::materialize(doc),
            AppEvent::ItemAdded {
                id,
                list_id,
                state,
                binned_at,
                open_index,
                ..
            } => {
                if let Some((old_list, was_live)) = self.items.get(id).cloned()
                    && was_live
                {
                    self.remove_open(&old_list, id);
                }
                let open = binned_at.is_none() && state.is_open();
                if open {
                    self.insert_open(list_id, id, *open_index);
                }
                self.items.insert(id.clone(), (list_id.clone(), open));
            }
            AppEvent::ItemRemoved { id } => {
                if let Some((list, was_live)) = self.items.remove(id)
                    && was_live
                {
                    self.remove_open(&list, id);
                }
            }
            AppEvent::ItemMoved { id, open_index } => {
                let Some((list, open)) = self.items.get(id).cloned() else {
                    return;
                };
                if open && let Some(at) = open_index {
                    self.insert_open(&list, id, Some(*at));
                }
            }
            AppEvent::ItemLifecycleChanged {
                id,
                state,
                binned_at,
                open_index,
                ..
            } => {
                let Some((list, was_live)) = self.items.get(id).cloned() else {
                    return;
                };
                let now_live = binned_at.is_none() && state.is_open();
                if was_live && !now_live {
                    self.remove_open(&list, id);
                }
                if !was_live && now_live {
                    self.insert_open(&list, id, *open_index);
                }
                self.items.insert(id.clone(), (list, now_live));
            }
            AppEvent::ItemListChanged {
                id,
                list_id,
                open_index,
            } => {
                let Some((old_list, open)) = self.items.get(id).cloned() else {
                    return;
                };
                if open {
                    self.remove_open(&old_list, id);
                    self.insert_open(list_id, id, *open_index);
                }
                self.items.insert(id.clone(), (list_id.clone(), open));
            }
            // List / settings events don't affect the open projections.
            _ => {}
        }
    }
}

// ---------- invariant checks (public API only) ----------

/// Every projection invariant observable through the public API:
/// - no duplicate ids within a list's open view
/// - no id visible in two lists' open views
/// - exactly the open items (from `all_items`) are visible, each in
///   its own list — nothing hidden, nothing leaked
/// - done/binned views contain exactly the done/binned items
fn assert_projection_invariants(doc: &Doc, ctx: &str) {
    let items = doc.all_items();
    let mut expected_live: HashMap<String, HashSet<String>> = HashMap::new();
    for it in &items {
        if it.is_open() {
            expected_live
                .entry(it.list_id.clone())
                .or_default()
                .insert(it.id.clone());
        }
    }
    let mut lists: HashSet<String> = expected_live.keys().cloned().collect();
    lists.insert(LIST_INBOX.to_string());
    for l in doc.all_lists() {
        lists.insert(l.id);
    }

    let mut seen_anywhere: HashSet<String> = HashSet::new();
    for list_id in &lists {
        let open = doc.open_item_ids(list_id);
        let unique: HashSet<&String> = open.iter().collect();
        assert_eq!(
            unique.len(),
            open.len(),
            "{ctx}: duplicate visible item in list {list_id}: {open:?}"
        );
        for id in &open {
            assert!(
                seen_anywhere.insert(id.clone()),
                "{ctx}: item {id} visible in two lists at once"
            );
        }
        let expected = expected_live.remove(list_id).unwrap_or_default();
        let got: HashSet<String> = open.into_iter().collect();
        assert_eq!(
            got, expected,
            "{ctx}: open view of {list_id} disagrees with item locations"
        );
    }
    assert!(
        expected_live.is_empty(),
        "{ctx}: open items whose list has no projection: {expected_live:?}"
    );

    let done: HashSet<String> = doc.done_item_ids().into_iter().collect();
    let binned: HashSet<String> = doc.binned_item_ids().into_iter().collect();
    for it in &items {
        assert_eq!(
            done.contains(&it.id),
            it.is_closed() && !it.is_binned(),
            "{ctx}: done view mismatch for {}",
            it.id
        );
        assert_eq!(
            binned.contains(&it.id),
            it.is_binned(),
            "{ctx}: bin view mismatch for {}",
            it.id
        );
    }
}

fn assert_mirror_matches(doc: &Doc, mirror: &Mirror, ctx: &str) {
    let mut lists: HashSet<String> = mirror.open.keys().cloned().collect();
    lists.insert(LIST_INBOX.to_string());
    for l in doc.all_lists() {
        lists.insert(l.id);
    }
    for it in doc.all_items() {
        lists.insert(it.list_id);
    }
    for list_id in lists {
        let doc_live = doc.open_item_ids(&list_id);
        let mirror_live = mirror.open.get(&list_id).cloned().unwrap_or_default();
        assert_eq!(
            mirror_live, doc_live,
            "{ctx}: consumer mirror drifted from doc for list {list_id}"
        );
    }
}

fn drain_into_mirror(doc: &Doc, mirror: &mut Mirror) {
    for ev in doc.drain_events() {
        mirror.apply(doc, &ev);
    }
}

// ---------- random op driver ----------

fn random_op(doc: &Doc, rng: &mut Lcg, op_no: usize) {
    let lists: Vec<String> = std::iter::once(LIST_INBOX.to_string())
        .chain(doc.all_lists().into_iter().map(|l| l.id))
        .collect();
    let items = doc.all_items();
    let pick_list = |rng: &mut Lcg, lists: &[String]| lists[rng.below(lists.len())].clone();

    match rng.below(100) {
        // adds (most common, keeps the doc growing)
        0..=24 => {
            let list = pick_list(rng, &lists);
            let target = rng.below(8);
            let _ = doc.add_item_at(&list, &format!("item {op_no}"), target);
        }
        25..=29 => {
            let list = pick_list(rng, &lists);
            let texts: Vec<String> = (0..rng.below(4) + 1)
                .map(|i| format!("batch {op_no}.{i}"))
                .collect();
            let refs: Vec<&str> = texts.iter().map(String::as_str).collect();
            let _ = doc.add_items_at(&list, &refs, rng.below(8));
        }
        // in-list reorder
        30..=44 => {
            let list = pick_list(rng, &lists);
            let open = doc.open_item_ids(&list);
            if !open.is_empty() {
                let id = open[rng.below(open.len())].clone();
                let _ = doc.move_item(&id, &list, rng.below(open.len() + 1));
            }
        }
        // cross-list move (open or hidden)
        45..=59 => {
            if !items.is_empty() {
                let it = &items[rng.below(items.len())];
                let target = pick_list(rng, &lists);
                let _ = doc.move_item(&it.id, &target, rng.below(8));
            }
        }
        // lifecycle flips: done toggles plus random workflow transitions
        // over the v3 ladder (spec/data-model.md "Set lifecycle").
        60..=64 => {
            if !items.is_empty() {
                let it = &items[rng.below(items.len())];
                let _ = doc.set_item_done(&it.id, !it.is_done());
            }
        }
        65..=69 => {
            if !items.is_empty() {
                let it = &items[rng.below(items.len())];
                let target = match rng.below(6) {
                    0 => ItemLifecycle::Backlog,
                    1 => ItemLifecycle::Todo,
                    2 => ItemLifecycle::InProgress,
                    3 => ItemLifecycle::Review,
                    4 => ItemLifecycle::Cancelled,
                    _ => ItemLifecycle::Done,
                };
                let _ = doc.set_item_lifecycle(&it.id, target);
            }
        }
        70..=76 => {
            if !items.is_empty() {
                let it = &items[rng.below(items.len())];
                let _ = doc.set_item_binned(&it.id, !it.is_binned());
            }
        }
        // edits
        77..=79 => {
            if !items.is_empty() {
                let it = &items[rng.below(items.len())];
                let _ = doc.edit_item_text(&it.id, &format!("edited {op_no}"));
            }
        }
        // hard deletes
        80..=83 => {
            let binned = doc.binned_item_ids();
            if !binned.is_empty() {
                let id = &binned[rng.below(binned.len())];
                let _ = doc.delete_binned(id);
            }
        }
        84 => {
            let _ = doc.empty_bin();
        }
        // list churn
        85..=88 => {
            let _ = doc.add_list(&format!("List {op_no}"));
        }
        89 => {
            let user_lists = doc.all_lists();
            if !user_lists.is_empty() {
                let l = &user_lists[rng.below(user_lists.len())];
                let _ = doc.delete_list(&l.id);
            }
        }
        90 => {
            let user_lists = doc.all_lists();
            if !user_lists.is_empty() {
                let l = &user_lists[rng.below(user_lists.len())];
                let _ = doc.rename_list(&l.id, &format!("Renamed {op_no}"));
            }
        }
        // undo / redo
        91..=95 => {
            let _ = doc.undo();
        }
        96..=97 => {
            let _ = doc.redo();
        }
        // explicit reconciliation (must never change visible state)
        _ => {
            let before: Vec<String> = doc.open_item_ids(LIST_INBOX);
            let _ = doc.reconcile();
            assert_eq!(
                doc.open_item_ids(LIST_INBOX),
                before,
                "reconcile changed a visible projection"
            );
        }
    }
}

#[test]
fn randomized_multi_peer_convergence() {
    // Deeper local runs: MONOPLAN_FUZZ_SEEDS=50 cargo test -p monoplan-core \
    //   --test order_schema --release
    let seeds: u64 = std::env::var("MONOPLAN_FUZZ_SEEDS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(6);
    for seed in 1..=seeds {
        let mut rng = Lcg::new(seed);
        let dek = Dek::generate();
        const PEERS: usize = 3;
        const OPS: usize = 220;

        let mut docs: Vec<Doc> = Vec::new();
        let mut mirrors: Vec<Mirror> = Vec::new();
        for i in 0..PEERS {
            let doc = if i == 0 {
                Doc::new().unwrap()
            } else {
                Doc::empty()
            };
            let _ = doc.drain_events();
            mirrors.push(Mirror::materialize(&doc));
            docs.push(doc);
        }

        for op_no in 0..OPS {
            let p = rng.below(PEERS);
            random_op(&docs[p], &mut rng, op_no);
            drain_into_mirror(&docs[p], &mut mirrors[p]);
            let ctx = format!("seed {seed} op {op_no} peer {p}");
            assert_projection_invariants(&docs[p], &ctx);
            assert_mirror_matches(&docs[p], &mirrors[p], &ctx);

            // Random pairwise sync: ship a full encrypted snapshot from
            // one peer into another (Loro merges; same path a bootstrap
            // or catch-up frame takes through `apply_remote`).
            if rng.chance(20) {
                let from = rng.below(PEERS);
                let to = (from + 1 + rng.below(PEERS - 1)) % PEERS;
                let blob = docs[from].snapshot_blob(&dek).unwrap();
                docs[to].apply_remote(&dek, &blob).unwrap();
                drain_into_mirror(&docs[to], &mut mirrors[to]);
                let ctx = format!("seed {seed} op {op_no} sync {from}->{to}");
                assert_projection_invariants(&docs[to], &ctx);
                assert_mirror_matches(&docs[to], &mirrors[to], &ctx);
            }
        }

        // Full mesh sync until quiescent: two rounds of everyone-to-
        // everyone suffice for snapshot exchange (state is monotone).
        for _round in 0..2 {
            for from in 0..PEERS {
                let blob = docs[from].snapshot_blob(&dek).unwrap();
                for to in 0..PEERS {
                    if to != from {
                        docs[to].apply_remote(&dek, &blob).unwrap();
                        drain_into_mirror(&docs[to], &mut mirrors[to]);
                    }
                }
            }
        }

        let fp0 = docs[0].fingerprint();
        for (i, doc) in docs.iter().enumerate() {
            assert_eq!(
                doc.fingerprint(),
                fp0,
                "seed {seed}: peer {i} did not converge"
            );
            let ctx = format!("seed {seed} final peer {i}");
            assert_projection_invariants(doc, &ctx);
            assert_mirror_matches(doc, &mirrors[i], &ctx);
        }

        // Reconciliation after convergence keeps logical state intact
        // (stale/duplicate garbage removal is invisible) and re-syncs
        // cleanly.
        let visible_before: Vec<Vec<String>> = {
            let mut v = vec![docs[0].open_item_ids(LIST_INBOX)];
            for l in docs[0].all_lists() {
                v.push(docs[0].open_item_ids(&l.id));
            }
            v
        };
        docs[0].reconcile().unwrap();
        let visible_after: Vec<Vec<String>> = {
            let mut v = vec![docs[0].open_item_ids(LIST_INBOX)];
            for l in docs[0].all_lists() {
                v.push(docs[0].open_item_ids(&l.id));
            }
            v
        };
        assert_eq!(
            visible_before, visible_after,
            "seed {seed}: reconcile changed views"
        );
        let blob = docs[0].snapshot_blob(&dek).unwrap();
        docs[1].apply_remote(&dek, &blob).unwrap();
        assert_eq!(
            docs[0].fingerprint(),
            docs[1].fingerprint(),
            "seed {seed}: reconcile broke convergence"
        );
    }
}

/// Sequential multi-session history: each session is a fresh Loro peer
/// booted from the accumulated oplog rows (the BootGate replay loop),
/// doing bulk adds, cross-list multi-moves, undos, and captures. This
/// is the retired `move-undo-multipeer` repro shape that used to trap
/// the wasm engine under the v1 schema.
#[test]
fn large_synthetic_history_many_peers_lists_moves_undos() {
    let dek = Dek::generate();
    let mut rows: Vec<monoplan_protocol::EncryptedBlob> = Vec::new();

    let capture = |doc: &mut Doc, rows: &mut Vec<monoplan_protocol::EncryptedBlob>| {
        if let Some(blob) = doc.pending_export(&dek).unwrap() {
            rows.push(blob);
            doc.mark_persisted();
        }
    };
    let boot = |rows: &[monoplan_protocol::EncryptedBlob]| -> Doc {
        let mut doc = Doc::empty();
        for row in rows {
            let plaintext = dek.open(&row.ciphertext, &row.nonce).unwrap();
            doc.replay_oplog_update(&plaintext).unwrap();
        }
        doc.finish_oplog_replay();
        doc.mark_persisted();
        doc
    };

    // Session 1: seed the doc — lists + a big main backlog + churn.
    let mut list_a = String::new();
    let mut ids: Vec<String> = Vec::new();
    {
        let mut s1 = Doc::new().unwrap();
        list_a = s1.add_list("list-a").unwrap();
        s1.add_list("list-b").unwrap();
        let texts: Vec<String> = (0..800).map(|i| format!("s1-item-{i}")).collect();
        let refs: Vec<&str> = texts.iter().map(String::as_str).collect();
        ids = s1.add_items_at(LIST_INBOX, &refs, 0).unwrap();
        for i in 0..20 {
            s1.set_item_done(&ids[i * 11], true).unwrap();
        }
        for i in 0..10 {
            s1.move_item(&ids[i * 29], &list_a, 0).unwrap();
        }
        capture(&mut s1, &mut rows);
    }

    // Sessions 2..=5: new peers each time — more items, multi-select
    // cross-list moves of items authored by *earlier* peers, undos of
    // some of those moves, deletes, all captured.
    for session in 2..=5usize {
        let mut doc = boot(&rows);
        assert_projection_invariants(&doc, &format!("session {session} boot"));

        let more: Vec<String> = {
            let texts: Vec<String> = (0..300).map(|i| format!("s{session}-item-{i}")).collect();
            let refs: Vec<&str> = texts.iter().map(String::as_str).collect();
            doc.add_items_at(LIST_INBOX, &refs, 0).unwrap()
        };
        ids.extend(more);

        // Multi-select move: 6 old items into list-a, like the store's
        // planReorderMoves emits (one commit each).
        let live_main = doc.open_item_ids(LIST_INBOX);
        let selection: Vec<String> = live_main
            .iter()
            .skip(session * 7)
            .step_by(41)
            .take(6)
            .cloned()
            .collect();
        for (i, id) in selection.iter().enumerate() {
            doc.move_item(id, &list_a, i).unwrap();
        }
        // Undo half of them, redo one.
        for _ in 0..3 {
            assert!(doc.undo().unwrap());
        }
        assert!(doc.redo().unwrap());
        assert_projection_invariants(&doc, &format!("session {session} after undo/redo"));

        // Some lifecycle churn + a bin purge.
        let live_main = doc.open_item_ids(LIST_INBOX);
        for id in live_main.iter().step_by(37).take(8) {
            doc.set_item_binned(id, true).unwrap();
        }
        let binned = doc.binned_item_ids();
        if binned.len() > 4 {
            doc.delete_binned_items(&binned[..2].iter().map(String::as_str).collect::<Vec<_>>())
                .unwrap();
        }
        capture(&mut doc, &mut rows);
    }

    // Final session: boot the whole history, then snapshot save/load —
    // the exact post-undo persistence path that used to crash.
    let final_doc = boot(&rows);
    assert_projection_invariants(&final_doc, "final boot");
    let saved = final_doc.save().unwrap();
    let reloaded = Doc::load(&saved).unwrap();
    assert_eq!(final_doc.fingerprint(), reloaded.fingerprint());
    assert_projection_invariants(&reloaded, "reloaded");

    // And a fresh device bootstrapping from an encrypted snapshot blob
    // converges too.
    let snapshot = final_doc.snapshot_blob(&dek).unwrap();
    let mut device2 = Doc::empty();
    device2.apply_remote(&dek, &snapshot).unwrap();
    assert_eq!(device2.fingerprint(), final_doc.fingerprint());
    assert_projection_invariants(&device2, "device2 bootstrap");
}
