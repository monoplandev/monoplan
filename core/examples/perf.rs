//! Scale and write-path measurements for the core doc. Not a benchmark
//! harness; a quick "what does N items look like" probe. Run via
//! `bun run perf [scenario] [items] [paras-per-item]` from the workspace
//! root (release build). Scenarios: `all` (default), `scale`, `commit`,
//! `import`.
//!
//! - `scale`: 10k items (default) with two paragraphs of notes each, in three
//!   shapes: today's string register through the monoplan `Doc`, the v4
//!   mergeable `LoroText` shape on a raw Loro doc, and the same with a
//!   simulated typing history. Reports snapshot size, import, boot-shaped
//!   walk, notes read, and seal / open of the blob.
//! - `commit`: per-call cost of `add_item` vs `edit_item_notes` vs the
//!   bulk `add_items_at`, to separate commit cost from index cost.
//! - `import`: `import_json` of a synthetic v3 export of the same size.

use loro::{Container, ExportMode, LoroDoc, LoroMap, LoroValue, UpdateOptions, ValueOrContainer};
use monoplan_core::doc::Doc;
use monoplan_core::{
    Dek, ExportItem, ExportLifecycle, ExportList, ExportSettings, JsonExport, LIST_INBOX,
};
use std::time::Instant;

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
            eprintln!("unknown scenario {other:?}; use all | scale | commit | import");
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
            doc.edit_item_notes(id, &notes_for(cfg, i)).unwrap();
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
        doc.edit_item_notes(id, &format!("note {i} some longer text here"))
            .unwrap();
    }
    println!("  edit_item_notes x{n}: {:?}", t.elapsed());
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
                created_at: 1_700_000_000_000 + i as i64,
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
