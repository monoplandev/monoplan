//! Local plaintext search index over the decrypted doc (`spec/search.md`).
//!
//! An in-memory inverted index over items and lists: AND across query
//! tokens, the last token treated as a prefix, heuristic ranking tuned
//! for a command palette. It lives in core so every client (web, CLI,
//! TUI, mobile) shares one tokenizer and one ranking.
//!
//! Maintenance is a dirty set, not an event mirror. Every `AppEvent` the
//! doc enqueues is inspected ([`SearchIndex::note_event`]) and the ids it
//! touches are marked; the bulk paths that rebuild the projection index
//! mark everything. The next query reconciles the dirty ids against the
//! doc itself ([`crate::Doc::search`]), so the index never depends on an
//! event carrying the right payload — only on it naming the right id.
//! The index holds plaintext and is never persisted or sent anywhere.

use std::collections::{HashMap, HashSet};

use serde::{Serialize, Serializer};
use unicode_normalization::UnicodeNormalization;
use unicode_normalization::char::is_combining_mark;

use crate::doc::{ItemLifecycle, ItemView, ListView};
use crate::events::AppEvent;

/// Fold text into search tokens (`spec/search.md` "Normalization"):
/// NFKD, strip combining marks (so `articulo` meets `artículo`),
/// lowercase, split on anything that is not a letter or a number, drop
/// empties, de-duplicate preserving first occurrence.
pub fn tokenize(input: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    let mut cur = String::new();
    let flush = |cur: &mut String, out: &mut Vec<String>, seen: &mut HashSet<String>| {
        if !cur.is_empty() {
            if seen.insert(cur.clone()) {
                out.push(cur.clone());
            }
            cur.clear();
        }
    };
    for c in input.nfkd() {
        if is_combining_mark(c) {
            continue;
        }
        if c.is_alphanumeric() {
            for l in c.to_lowercase() {
                cur.push(l);
            }
        } else {
            flush(&mut cur, &mut out, &mut seen);
        }
    }
    flush(&mut cur, &mut out, &mut seen);
    out
}

/// Name-only filter predicate for pickers that narrow a short list of
/// names rather than querying the index. Every query token must prefix
/// some token of `name`, in any order; an empty query matches everything.
pub fn matches_name(name: &str, query: &str) -> bool {
    let wanted = tokenize(query);
    if wanted.is_empty() {
        return true;
    }
    let have = tokenize(name);
    wanted
        .iter()
        .all(|w| have.iter().any(|t| t.starts_with(w.as_str())))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum SearchKind {
    Item,
    List,
}

/// One ranked hit. `lifecycle` is set for items only.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchResult {
    pub id: String,
    pub kind: SearchKind,
    pub title: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub body: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub list_id: Option<String>,
    #[serde(
        skip_serializing_if = "Option::is_none",
        serialize_with = "ser_lifecycle"
    )]
    pub lifecycle: Option<ItemLifecycle>,
    /// Flat score for display / debugging. Ordering uses the bucket
    /// counts directly, so this never affects the result order.
    pub score: u64,
}

fn ser_lifecycle<S: Serializer>(l: &Option<ItemLifecycle>, s: S) -> Result<S::Ok, S::Error> {
    match l {
        Some(l) => s.serialize_str(l.name()),
        None => s.serialize_none(),
    }
}

/// Per-doc indexed view. Token sets are kept so a field edit can drop the
/// old postings before inserting the new ones.
#[derive(Debug, Clone)]
struct SearchDoc {
    id: String,
    kind: SearchKind,
    title: String,
    body: String,
    list_id: Option<String>,
    lifecycle: Option<ItemLifecycle>,
    updated_at: i64,
    title_tokens: HashSet<String>,
    body_tokens: HashSet<String>,
    context_tokens: HashSet<String>,
}

impl SearchDoc {
    fn all_tokens(&self) -> impl Iterator<Item = &String> {
        self.title_tokens
            .iter()
            .chain(self.body_tokens.iter())
            .chain(self.context_tokens.iter())
    }
}

/// Rank order for tie-breaking (`spec/search.md` "Ranking"): active work
/// first, then queued, then closed, then binned. Lists rank with binned.
fn lifecycle_rank(l: Option<ItemLifecycle>) -> u64 {
    match l {
        Some(ItemLifecycle::InProgress) => 5,
        Some(ItemLifecycle::Review) => 4,
        Some(ItemLifecycle::Todo) => 3,
        Some(ItemLifecycle::Backlog) => 2,
        Some(ItemLifecycle::Done) | Some(ItemLifecycle::Cancelled) => 1,
        Some(ItemLifecycle::Binned) | None => 0,
    }
}

/// What the next query must reconcile against the doc before answering.
#[derive(Debug, Default)]
pub(crate) struct Dirty {
    pub(crate) all: bool,
    pub(crate) items: HashSet<String>,
    pub(crate) lists: HashSet<String>,
}

impl Dirty {
    fn is_clean(&self) -> bool {
        !self.all && self.items.is_empty() && self.lists.is_empty()
    }
}

#[derive(Debug, Default)]
pub struct SearchIndex {
    docs_by_id: HashMap<String, SearchDoc>,
    /// token -> ids of docs containing it in any field.
    postings: HashMap<String, HashSet<String>>,
    /// list id -> item ids whose `list_id` is that list. Lets a list
    /// rename reindex just its items.
    items_by_list: HashMap<String, HashSet<String>>,
    /// list id -> current name, read when an item is indexed so its
    /// context tokens can be built without the list doc in hand.
    list_names: HashMap<String, String>,
    dirty: Dirty,
}

impl SearchIndex {
    /// An index that knows nothing yet and will build itself whole on
    /// the first reconcile.
    pub fn new() -> Self {
        Self {
            dirty: Dirty {
                all: true,
                ..Dirty::default()
            },
            ..Self::default()
        }
    }

    // ---------- dirty tracking ----------

    /// Everything must be rebuilt (boot replay, bulk import, translation
    /// fallback).
    pub fn mark_all_dirty(&mut self) {
        self.dirty.all = true;
        self.dirty.items.clear();
        self.dirty.lists.clear();
    }

    /// Record which ids an enqueued event touches. Pure ordering events
    /// and register changes that are not indexed are ignored.
    pub fn note_event(&mut self, ev: &AppEvent) {
        if self.dirty.all {
            return;
        }
        match ev {
            AppEvent::FullResync => self.mark_all_dirty(),
            AppEvent::ItemAdded { id, .. }
            | AppEvent::ItemRemoved { id }
            | AppEvent::ItemTextChanged { id, .. }
            | AppEvent::ItemNotesChanged { id, .. }
            | AppEvent::ItemNotesDelta { id, .. }
            | AppEvent::ItemLifecycleChanged { id, .. }
            | AppEvent::ItemListChanged { id, .. } => {
                self.dirty.items.insert(id.clone());
            }
            AppEvent::ListAdded { id, .. }
            | AppEvent::ListRemoved { id }
            | AppEvent::ListRenamed { id, .. } => {
                self.dirty.lists.insert(id.clone());
            }
            _ => {}
        }
    }

    /// Take the pending dirty set, leaving the index clean. The caller
    /// reconciles the returned ids against the doc.
    pub(crate) fn take_dirty(&mut self) -> Option<Dirty> {
        if self.dirty.is_clean() {
            return None;
        }
        Some(std::mem::take(&mut self.dirty))
    }

    /// Item ids currently indexed under `list_id` (a snapshot; safe to
    /// iterate while reindexing).
    pub(crate) fn item_ids_in_list(&self, list_id: &str) -> Vec<String> {
        self.items_by_list
            .get(list_id)
            .map(|s| s.iter().cloned().collect())
            .unwrap_or_default()
    }

    // ---------- maintenance ----------

    pub fn clear(&mut self) {
        self.docs_by_id.clear();
        self.postings.clear();
        self.items_by_list.clear();
        self.list_names.clear();
    }

    /// Rebuild from scratch. Lists first so items can read list names.
    pub fn rebuild<'a>(
        &mut self,
        lists: &[ListView],
        items: impl IntoIterator<Item = &'a ItemView>,
    ) {
        self.clear();
        for l in lists {
            self.index_list(l);
        }
        for it in items {
            self.index_item(it);
        }
    }

    /// Insert or replace the doc for `list`, and reindex its items so
    /// their context tokens follow the (possibly new) name.
    pub fn index_list(&mut self, list: &ListView) {
        self.list_names.insert(list.id.clone(), list.name.clone());
        self.remove(&list.id);
        let title_tokens: HashSet<String> = tokenize(&list.name).into_iter().collect();
        self.insert_doc(SearchDoc {
            id: list.id.clone(),
            kind: SearchKind::List,
            title: list.name.clone(),
            body: String::new(),
            list_id: None,
            lifecycle: None,
            updated_at: list.created_at,
            title_tokens,
            body_tokens: HashSet::new(),
            context_tokens: HashSet::new(),
        });
    }

    /// Forget a list: drop its doc and name. Items still pointing at it
    /// keep their `list_id` but lose their context tokens on reindex.
    pub fn remove_list(&mut self, list_id: &str) {
        self.remove(list_id);
        self.list_names.remove(list_id);
    }

    /// Insert or replace the doc for `item`.
    pub fn index_item(&mut self, item: &ItemView) {
        self.remove(&item.id);
        let ctx_name = self
            .list_names
            .get(&item.list_id)
            .map(String::as_str)
            .unwrap_or("");
        // Recency signal: the bin mask, else the register's transition
        // time (which already falls back to `created_at`).
        let updated_at = item.binned_at.unwrap_or(item.lifecycle_at);
        self.insert_doc(SearchDoc {
            id: item.id.clone(),
            kind: SearchKind::Item,
            title: item.text.clone(),
            body: item.notes.clone(),
            list_id: (!item.list_id.is_empty()).then(|| item.list_id.clone()),
            lifecycle: Some(item.lifecycle()),
            updated_at,
            title_tokens: tokenize(&item.text).into_iter().collect(),
            body_tokens: tokenize(&item.notes).into_iter().collect(),
            context_tokens: tokenize(ctx_name).into_iter().collect(),
        });
    }

    /// Drop a doc (item or list) and its postings. No-op if absent.
    pub fn remove(&mut self, id: &str) {
        let Some(doc) = self.docs_by_id.remove(id) else {
            return;
        };
        for t in doc.all_tokens() {
            if let Some(set) = self.postings.get_mut(t) {
                set.remove(id);
                if set.is_empty() {
                    self.postings.remove(t);
                }
            }
        }
        if let Some(list_id) = &doc.list_id
            && let Some(set) = self.items_by_list.get_mut(list_id)
        {
            set.remove(id);
            if set.is_empty() {
                self.items_by_list.remove(list_id);
            }
        }
    }

    fn insert_doc(&mut self, doc: SearchDoc) {
        for t in doc.all_tokens() {
            self.postings
                .entry(t.clone())
                .or_default()
                .insert(doc.id.clone());
        }
        if doc.kind == SearchKind::Item
            && let Some(list_id) = &doc.list_id
        {
            self.items_by_list
                .entry(list_id.clone())
                .or_default()
                .insert(doc.id.clone());
        }
        self.docs_by_id.insert(doc.id.clone(), doc);
    }

    // ---------- query ----------

    /// Ranked hits for `input`, at most `limit`. Empty / punctuation-only
    /// input returns nothing.
    pub fn query(&self, input: &str, limit: usize) -> Vec<SearchResult> {
        let tokens = tokenize(input);
        let Some((final_token, exact_tokens)) = tokens.split_last() else {
            return Vec::new();
        };

        let mut candidates: Option<HashSet<&str>> = None;
        for t in exact_tokens {
            let Some(set) = self.postings.get(t) else {
                return Vec::new();
            };
            let set: HashSet<&str> = set.iter().map(String::as_str).collect();
            candidates = Some(intersect(candidates, set));
            if candidates.as_ref().is_some_and(HashSet::is_empty) {
                return Vec::new();
            }
        }
        let final_set = self.prefix_candidates(final_token);
        if final_set.is_empty() {
            return Vec::new();
        }
        let candidates = intersect(candidates, final_set);

        let mut scored: Vec<Scored<'_>> = candidates
            .into_iter()
            .filter_map(|id| self.docs_by_id.get(id))
            .filter_map(|doc| score_doc(doc, exact_tokens, final_token))
            .collect();
        scored.sort_by(|a, b| {
            b.title_exact
                .cmp(&a.title_exact)
                .then(b.title_prefix.cmp(&a.title_prefix))
                .then(b.body_hits.cmp(&a.body_hits))
                .then(b.context_hits.cmp(&a.context_hits))
                .then(lifecycle_rank(b.doc.lifecycle).cmp(&lifecycle_rank(a.doc.lifecycle)))
                .then(b.doc.updated_at.cmp(&a.doc.updated_at))
                .then(a.doc.id.cmp(&b.doc.id))
        });
        scored
            .into_iter()
            .take(limit)
            .map(|s| SearchResult {
                id: s.doc.id.clone(),
                kind: s.doc.kind,
                title: s.doc.title.clone(),
                body: (!s.doc.body.is_empty()).then(|| s.doc.body.clone()),
                list_id: s.doc.list_id.clone(),
                lifecycle: s.doc.lifecycle,
                score: s.score,
            })
            .collect()
    }

    /// Ids of docs holding any token that starts with `prefix`. A linear
    /// scan over unique tokens; the corpus is small enough today.
    fn prefix_candidates(&self, prefix: &str) -> HashSet<&str> {
        let mut out = HashSet::new();
        for (token, ids) in &self.postings {
            if token.starts_with(prefix) {
                out.extend(ids.iter().map(String::as_str));
            }
        }
        out
    }
}

fn intersect<'a>(a: Option<HashSet<&'a str>>, b: HashSet<&'a str>) -> HashSet<&'a str> {
    match a {
        None => b,
        Some(a) => {
            let (smaller, larger) = if a.len() <= b.len() {
                (&a, &b)
            } else {
                (&b, &a)
            };
            smaller
                .iter()
                .filter(|id| larger.contains(*id))
                .copied()
                .collect()
        }
    }
}

struct Scored<'a> {
    doc: &'a SearchDoc,
    title_exact: u32,
    title_prefix: u32,
    body_hits: u32,
    context_hits: u32,
    score: u64,
}

fn any_starts_with(set: &HashSet<String>, prefix: &str) -> bool {
    set.iter().any(|t| t.starts_with(prefix))
}

fn score_doc<'a>(
    doc: &'a SearchDoc,
    exact_tokens: &[String],
    final_token: &str,
) -> Option<Scored<'a>> {
    let mut title_exact = 0;
    let mut title_prefix = 0;
    let mut body_hits = 0;
    let mut context_hits = 0;

    for t in exact_tokens {
        if doc.title_tokens.contains(t) {
            title_exact += 1;
        } else if doc.body_tokens.contains(t) {
            body_hits += 1;
        } else if doc.context_tokens.contains(t) {
            context_hits += 1;
        } else {
            return None;
        }
    }

    // Final token: exact beats prefix in the title; otherwise fall back
    // through body then context. Rechecked so the bucket is right.
    if doc.title_tokens.contains(final_token) {
        title_exact += 1;
    } else if any_starts_with(&doc.title_tokens, final_token) {
        title_prefix += 1;
    } else if doc.body_tokens.contains(final_token)
        || any_starts_with(&doc.body_tokens, final_token)
    {
        body_hits += 1;
    } else if doc.context_tokens.contains(final_token)
        || any_starts_with(&doc.context_tokens, final_token)
    {
        context_hits += 1;
    } else {
        return None;
    }

    let score = u64::from(title_exact) * 10_000
        + u64::from(title_prefix) * 1_000
        + u64::from(body_hits) * 100
        + u64::from(context_hits) * 10
        + lifecycle_rank(doc.lifecycle);

    Some(Scored {
        doc,
        title_exact,
        title_prefix,
        body_hits,
        context_hits,
        score,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokenize_spec_examples() {
        assert_eq!(tokenize("Buy groceries"), ["buy", "groceries"]);
        assert_eq!(tokenize("PR #142"), ["pr", "142"]);
        assert_eq!(tokenize("Q3 roadmap"), ["q3", "roadmap"]);
    }

    #[test]
    fn tokenize_lowercases_and_dedupes() {
        assert_eq!(tokenize("Foo FOO foo Bar"), ["foo", "bar"]);
    }

    #[test]
    fn tokenize_degenerate_inputs() {
        assert!(tokenize("").is_empty());
        assert!(tokenize("   ").is_empty());
        assert!(tokenize("#!?,.").is_empty());
    }

    #[test]
    fn tokenize_nfkd_folds_fullwidth() {
        assert_eq!(tokenize("ＰＲ １４２"), ["pr", "142"]);
    }

    #[test]
    fn tokenize_folds_accents_both_ways() {
        assert_eq!(tokenize("artículo"), ["articulo"]);
        assert_eq!(tokenize("Crème brûlée"), ["creme", "brulee"]);
        // Precomposed and decomposed forms meet at the same token.
        assert_eq!(tokenize("e\u{0301}"), tokenize("é"));
    }

    #[test]
    fn matches_name_prefixes_every_token_in_any_order() {
        assert!(matches_name("Work projects", ""));
        assert!(matches_name("Work projects", "pro wo"));
        assert!(matches_name("Récits", "rec"));
        assert!(!matches_name("Work projects", "home"));
    }
}
