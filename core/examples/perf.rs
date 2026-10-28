//! Scale and write-path measurements for the core doc. Not a benchmark
//! harness; a quick "what does N items look like" probe. Run via
//! `bun run perf [scenario] [items] [paras-per-item]` from the workspace
//! root (release build). Scenarios: `all` (default), `scale`, `commit`,
//! `import`, `chain`.
//!
//! - `scale`: 10k items (default) with two paragraphs of notes each, in three
//!   shapes: today's string register through the monoplan `Doc`, the v4
//!   mergeable `LoroText` shape on a raw Loro doc, and the same with a
//!   simulated typing history. Reports snapshot size, import, boot-shaped
//!   walk, notes read, and seal / open of the blob.
//! - `commit`: per-call cost of `add_item` vs `apply_notes_delta` vs the
//!   bulk `add_items_at`, to separate commit cost from index cost.
//! - `import`: `import_json` of a synthetic v3 export of the same size.
//! - `chain`: materialisation cost of one full snapshot vs a full head
//!   plus coalesced update deltas (one per `every` ops), vs today's
//!   per-op WAL rows. `bun run perf chain [ops] [every]` (defaults
//!   10_000 / 500). Not part of `all`: its arguments mean something else.

use loro::{
    Container, ExportMode, LoroDoc, LoroMap, LoroValue, UpdateOptions, ValueOrContainer,
    VersionVector,
};
use monoplan_core::doc::Doc;
use monoplan_core::{
    Dek, ExportItem, ExportLifecycle, ExportList, ExportSettings, JsonExport, LIST_INBOX,
    NotesDeltaOp,
};
use monoplan_protocol::EncryptedBlob;
use std::time::{Duration, Instant};

const WORDS: &[&str] = &[
    "the", "quick", "brown", "fox", "jumps", "over", "lazy", "dog", "milk", "coles", "call",
    "dentist", "renew", "passport", "before", "trip", "check", "flight", "times", "and", "book",
    "shuttle", "remember", "to", "ask", "about", "invoice", "from", "last", "month", "sort",
    "photos", "backup", "laptop", "fix", "bike", "tyre", "buy", "oat",
];

struct Cfg {
    items: usize,
    paras: usize,
}

/// ~40 words, ~230 chars, deterministic per seed.
fn para(seed: usize) -> String {
    let mut s = String::new();
    let mut x = seed.wrapping_mul(2654435761) ^ 0x9e3779b9;
    for i in 0..40 {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        if i > 0 {
            s.push(' ');
        }
        s.push_str(WORDS[x % WORDS.len()]);
    }
    s.push('.');
    s
}

fn notes_for(cfg: &Cfg, i: usize) -> String {
    (0..cfg.paras)
        .map(|p| para(i * 7 + p))
        .collect::<Vec<_>>()
        .join("\n\n")
}

fn mib(n: usize) -> String {
    format!("{:.1} MiB", n as f64 / 1048576.0)
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let scenario = args.first().map(String::as_str).unwrap_or("all");
    if scenario == "chain" {
        let ops = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(10_000);
        let every = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(500);
        chain(ops, every);
        return;
    }
    let cfg = Cfg {
        items: args.get(1).and_then(|s| s.parse().ok()).unwrap_or(10_000),
        paras: args.get(2).and_then(|s| s.parse().ok()).unwrap_or(2),
    };
    let total_chars: usize = (0..cfg.items).map(|i| notes_for(&cfg, i).len()).sum();
    println!(
        "{} items x {} paragraphs, {} of notes text",
        cfg.items,
        cfg.paras,
        mib(total_chars)
    );
    match scenario {
        "scale" => scale(&cfg),
        "commit" => commit(&cfg),
        "import" => import(&cfg),
        "all" => {
            scale(&cfg);
            commit(&cfg);
            import(&cfg);
        }
        other => {
            eprintln!("unknown scenario {other:?}; use all | scale | commit | import | chain");
            std::process::exit(2);
        }
    }
}

// ---------- scale ----------

fn scale(cfg: &Cfg) {
    // Today: monoplan Doc, notes as a string register. Built with the
    // bulk add so the number reflects the doc, not the per-add index
    // cost (see `commit`).
    {
        let t = Instant::now();
        let doc = Doc::new().unwrap();
        let texts: Vec<String> = (0..cfg.items).map(|i| format!("item {i}")).collect();
        let refs: Vec<&str> = texts.iter().map(String::as_str).collect();
        let ids = doc.add_items_at(LIST_INBOX, &refs, usize::MAX).unwrap();
        for (i, id) in ids.iter().enumerate() {
            doc.apply_notes_delta(
                id,
                &[NotesDeltaOp::Insert {
                    insert: notes_for(cfg, i),
                }],
            )
            .unwrap();
        }
        println!(
            "\n[A  v3 string register, monoplan Doc] build {:?}",
            t.elapsed()
        );
        let t = Instant::now();
        let bytes = doc.save().unwrap();
        println!("  save {:?}, envelope {}", t.elapsed(), mib(bytes.len()));
        let t = Instant::now();
        let d2 = Doc::load(&bytes).unwrap();
        println!("  Doc::load (import + rebuild_index) {:?}", t.elapsed());
        let t = Instant::now();
        let n = d2.all_items().len();
        println!("  all_items() {n} in {:?}", t.elapsed());
    }
    raw_v3(cfg);
    raw_v4(cfg, "B  v4 LoroText, one commit per item", 1);
    raw_v4(
        cfg,
        "C  v4 LoroText, 5 commits per paragraph (typing bursts)",
        5,
    );
}

fn insert_item_shell(items: &LoroMap, i: usize) -> LoroMap {
    let id = format!("it{i:06}");
    let m = items.insert_container(&id, LoroMap::new()).unwrap();
    m.insert("id", id.as_str()).unwrap();
    m.insert("text", format!("item {i}").as_str()).unwrap();
    m.insert("createdAt", 1_700_000_000_000i64 + i as i64)
        .unwrap();
    m.insert("location", format!("inbox:{i:08}").as_str())
        .unwrap();
    m
}

/// Boot-shaped read: touch one register per item, never the notes.
fn walk_meta(doc: &LoroDoc) -> usize {
    let mut n = 0usize;
    doc.get_map("items").for_each(|_k, v| {
        if let ValueOrContainer::Container(Container::Map(m)) = v
            && let Some(ValueOrContainer::Value(LoroValue::String(_))) = m.get("location")
        {
            n += 1;
        }
    });
    n
}

fn raw_v3(cfg: &Cfg) {
    let doc = LoroDoc::new();
    let items = doc.get_map("items");
    for i in 0..cfg.items {
        let m = insert_item_shell(&items, i);
        m.insert("notes", notes_for(cfg, i).as_str()).unwrap();
        doc.commit();
    }
    let snap = doc.export(ExportMode::Snapshot).unwrap();
    let d2 = LoroDoc::new();
    let t = Instant::now();
    d2.import(&snap).unwrap();
    println!(
        "\n[A' v3 string register, raw loro] import {:?}, snapshot {}",
        t.elapsed(),
        mib(snap.len())
    );
    let t = Instant::now();
    let n = walk_meta(&d2);
    println!(
        "  boot-shaped walk (location only): {:?} ({n} items)",
        t.elapsed()
    );
}

fn raw_v4(cfg: &Cfg, label: &str, chunks_per_para: usize) {
    let t = Instant::now();
    let doc = LoroDoc::new();
    let items = doc.get_map("items");
    for i in 0..cfg.items {
        let m = insert_item_shell(&items, i);
        let notes = m.ensure_mergeable_text("notes").unwrap();
        let full = notes_for(cfg, i);
        if chunks_per_para == 1 {
            notes.update(&full, UpdateOptions::default()).unwrap();
            doc.commit();
            continue;
        }
        let mut cur = String::new();
        for (pi, p) in full.split("\n\n").enumerate() {
            if pi > 0 {
                cur.push_str("\n\n");
            }
            let step = p.len().div_ceil(chunks_per_para);
            let mut pos = 0;
            while pos < p.len() {
                let mut end = (pos + step).min(p.len());
                while !p.is_char_boundary(end) {
                    end += 1;
                }
                cur.push_str(&p[pos..end]);
                notes.update(&cur, UpdateOptions::default()).unwrap();
                doc.commit();
                pos = end;
            }
        }
    }
    println!("\n[{label}] build {:?}", t.elapsed());
    let t = Instant::now();
    let snap = doc.export(ExportMode::Snapshot).unwrap();
    println!("  snapshot export {:?}, {}", t.elapsed(), mib(snap.len()));
    let t = Instant::now();
    let d2 = LoroDoc::new();
    d2.import(&snap).unwrap();
    println!("  import {:?}", t.elapsed());
    let t = Instant::now();
    let mut total = 0usize;
    d2.get_map("items").for_each(|_k, v| {
        if let ValueOrContainer::Container(Container::Map(m)) = v
            && let Some(ValueOrContainer::Container(Container::Text(t))) = m.get("notes")
        {
            total += t.to_string().len();
        }
    });
    println!(
        "  walk items + to_string every notes: {:?} ({})",
        t.elapsed(),
        mib(total)
    );
    let d3 = LoroDoc::new();
    d3.import(&snap).unwrap();
    let t = Instant::now();
    let n = walk_meta(&d3);
    println!(
        "  boot-shaped walk (location only, notes untouched): {:?} ({n} items)",
        t.elapsed()
    );
    let dek = Dek::generate();
    let t = Instant::now();
    let (ct, nonce) = dek.seal(&snap).unwrap();
    let seal_t = t.elapsed();
    let t = Instant::now();
    let pt = dek.open(&ct, &nonce).unwrap();
    println!(
        "  seal {seal_t:?}, open {:?} ({})",
        t.elapsed(),
        mib(pt.len())
    );
}

// ---------- commit ----------

fn commit(cfg: &Cfg) {
    let n = cfg.items;
    let doc = Doc::new().unwrap();
    let t = Instant::now();
    let mut ids = Vec::with_capacity(n);
    for i in 0..n {
        ids.push(doc.add_item(LIST_INBOX, &format!("item {i}")).unwrap());
    }
    println!("\n[commit] add_item x{n}: {:?}", t.elapsed());
    let t = Instant::now();
    for (i, id) in ids.iter().enumerate() {
        doc.apply_notes_delta(
            id,
            &[NotesDeltaOp::Insert {
                insert: format!("note {i} some longer text here"),
            }],
        )
        .unwrap();
    }
    println!("  apply_notes_delta x{n}: {:?}", t.elapsed());
    let doc2 = Doc::new().unwrap();
    let texts: Vec<String> = (0..n).map(|i| format!("item {i}")).collect();
    let refs: Vec<&str> = texts.iter().map(String::as_str).collect();
    let t = Instant::now();
    doc2.add_items_at(LIST_INBOX, &refs, usize::MAX).unwrap();
    println!("  add_items_at (bulk, one commit) x{n}: {:?}", t.elapsed());
    let t = Instant::now();
    let _ = doc.drain_events();
    println!("  drain_events {:?}", t.elapsed());
}

// ---------- import ----------

fn import(cfg: &Cfg) {
    let export = JsonExport {
        version: 1,
        settings: ExportSettings {
            show_list_counts: false,
            inbox_view: None,
        },
        lists: vec![ExportList {
            id: LIST_INBOX.to_string(),
            name: "Inbox".to_string(),
            icon: None,
            view: None,
            archived_at: None,
            created_at: None,
            builtin: true,
        }],
        items: (0..cfg.items)
            .map(|i| ExportItem {
                id: format!("src{i}"),
                text: format!("item {i}"),
                notes: notes_for(cfg, i),
                list_id: LIST_INBOX.to_string(),
                lifecycle: ExportLifecycle {
                    state: "backlog".to_string(),
                    at: 1_700_000_000_000 + i as i64,
                },
                deadline: None,
                when: None,
                duration: None,
                place: None,
                created_at: 1_700_000_000_000 + i as i64,
                created_by: None,
                started_at: None,
                done_at: None,
                binned_at: None,
            })
            .collect(),
        focus: Vec::new(),
    };
    let json = serde_json::to_string(&export).unwrap();
    println!("\n[import] export JSON {}", mib(json.len()));
    let doc = Doc::new().unwrap();
    let t = Instant::now();
    let summary = doc.import_json_str(&json).unwrap();
    println!(
        "  import_json_str {:?} ({} items added)",
        t.elapsed(),
        summary.items_added
    );
    let t = Instant::now();
    let evs = doc.drain_events().len();
    println!("  drain_events {evs} in {:?}", t.elapsed());
    let t = Instant::now();
    let bytes = doc.save().unwrap();
    println!("  save {:?}, envelope {}", t.elapsed(), mib(bytes.len()));
}

// ---------- chain ----------

/// How long does a fresh `Doc` take to materialise the same final state
/// from each storage shape? Mirrors `boot_doc`: decrypt + one
/// `apply_remote_batch` over the blobs in order, events discarded.
///
/// Shapes, all landing on the same oplog VV:
/// - `full`: one `ExportMode::Snapshot` of the whole doc (state + oplog).
/// - `head + k deltas`: a full snapshot at the checkpoint `k` intervals
///   back, then one coalesced `updates_in_range` blob per interval. The
///   server chain shape, and the proposed local "coalesced WAL fold".
/// - `head + rows`: a full snapshot at the checkpoint, then one blob per
///   op. Today's local WAL shape when the fold threshold hasn't fired.
///
/// Also prints the producer-side cost (full export vs one delta export)
/// so the write/read trade-off is on one screen.
fn chain(ops: usize, every: usize) {
    let every = every.max(1);
    let dek = Dek::generate();
    let mut doc = Doc::new().unwrap();

    // Checkpoint VVs: `vvs[0]` is the seeded doc (lists, settings),
    // `vvs[j]` is after `j * every` ops. Per-op WAL rows captured as
    // the host would: `pending_export` after every commit.
    let mut vvs: Vec<VersionVector> = vec![doc.oplog_vv()];
    let mut rows: Vec<EncryptedBlob> = Vec::with_capacity(ops);
    let mut ids: Vec<String> = Vec::new();
    let mut x: usize = 0x9e3779b9;
    let mut next = || {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        x
    };
    let t = Instant::now();
    for i in 0..ops {
        // Mix: half adds, a third edits, the rest done-toggles — one
        // commit each, like interactive use.
        let r = next() % 10;
        if ids.is_empty() || r < 5 {
            ids.push(doc.add_item(LIST_INBOX, &format!("item {i}")).unwrap());
        } else if r < 8 {
            let id = &ids[next() % ids.len()];
            doc.edit_item_text(id, &format!("item {i} edited")).unwrap();
        } else {
            let id = &ids[next() % ids.len()];
            doc.set_item_done(id, next() % 2 == 0).unwrap();
        }
        if let Some(b) = doc.pending_export(&dek).unwrap() {
            rows.push(b);
        }
        doc.mark_persisted();
        if (i + 1) % every == 0 {
            vvs.push(doc.oplog_vv());
        }
    }
    if vvs.last() != Some(&doc.oplog_vv()) {
        vvs.push(doc.oplog_vv());
    }
    let _ = doc.drain_events();
    let final_vv = doc.oplog_vv();
    let n_items = doc.all_items().len();
    let row_bytes: usize = rows.iter().map(blob_len).sum();
    println!(
        "[chain] {ops} ops ({n_items} items), delta every {every} ops, {} checkpoints, build {:?}",
        vvs.len() - 1,
        t.elapsed()
    );
    println!(
        "  per-op WAL rows: {} blobs, {} ({} B/blob avg)",
        rows.len(),
        mib(row_bytes),
        row_bytes / rows.len().max(1)
    );

    // Producer side.
    let t = Instant::now();
    let full = doc.snapshot_blob(&dek).unwrap();
    let full_export = t.elapsed();
    let last = vvs.len() - 1;
    let t = Instant::now();
    let one_delta = doc
        .delta_blob_between(&dek, &vvs[last - 1], &vvs[last])
        .unwrap();
    let delta_export = t.elapsed();
    println!(
        "\n  produce: full snapshot {} in {full_export:?}; one {every}-op delta {} in {delta_export:?}",
        mib(blob_len(&full)),
        mib(blob_len(&one_delta))
    );

    // Consumer side.
    let materialise = |label: &str, blobs: &[EncryptedBlob]| {
        let bytes: usize = blobs.iter().map(blob_len).sum();
        let mut best = Duration::MAX;
        for _ in 0..3 {
            let mut d = Doc::empty();
            let t = Instant::now();
            d.apply_remote_batch(&dek, blobs.iter()).unwrap();
            let _ = d.drain_events();
            let elapsed = t.elapsed();
            assert_eq!(d.oplog_vv(), final_vv, "{label}: chain is not lossless");
            assert_eq!(d.all_items().len(), n_items, "{label}: item count differs");
            best = best.min(elapsed);
        }
        println!(
            "  {label:<30} {:>5} blobs {:>10}  materialise {best:?}",
            blobs.len(),
            mib(bytes)
        );
    };

    println!("\n  materialise (min of 3, decrypt + apply_remote_batch into Doc::empty):");
    materialise("full", std::slice::from_ref(&full));

    let mut ks: Vec<usize> = [1usize, 2, 5, 10, 20, 50, 100]
        .into_iter()
        .filter(|&k| k < last)
        .collect();
    ks.push(last);
    for k in ks {
        let head_at = last - k;
        let mut blobs = vec![doc.snapshot_blob_at(&dek, &vvs[head_at]).unwrap()];
        for j in head_at..last {
            blobs.push(doc.delta_blob_between(&dek, &vvs[j], &vvs[j + 1]).unwrap());
        }
        let label = if k == last {
            format!("seed head + {k} deltas (all)")
        } else {
            format!("head + {k} deltas")
        };
        materialise(&label, &blobs);
    }

    // Today's shape: head + one row per op since. One interval's worth,
    // and everything.
    let since = rows.len().saturating_sub(every);
    let head_rows = {
        let mut b = vec![doc.snapshot_blob_at(&dek, &vvs[last - 1]).unwrap()];
        b.extend(rows[since..].iter().cloned());
        b
    };
    materialise(&format!("head + {} rows", rows.len() - since), &head_rows);
    let all_rows = {
        let mut b = vec![doc.snapshot_blob_at(&dek, &vvs[0]).unwrap()];
        b.extend(rows.iter().cloned());
        b
    };
    materialise(&format!("seed head + {} rows (all)", rows.len()), &all_rows);
}

fn blob_len(b: &EncryptedBlob) -> usize {
    b.ciphertext.len() + b.nonce.len()
}
