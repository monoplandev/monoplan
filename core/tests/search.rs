//! Search engine coverage from `spec/search.md` "Testing": add → query,
//! incremental updates, notes ranking below title, list-rename context
//! propagation, delete, multi-token AND, last-token prefix, and the
//! lifecycle tiebreak. Mutations go through the ordinary `Doc` API so the
//! dirty set is exercised exactly as production feeds it.

use monoplan_core::doc::{Doc, ItemLifecycle, LIST_INBOX, NotesDeltaOp};
use monoplan_core::search::SearchKind;

fn ids(doc: &Doc, q: &str) -> Vec<String> {
    doc.search(q, 50).into_iter().map(|r| r.id).collect()
}

fn titles(doc: &Doc, q: &str) -> Vec<String> {
    doc.search(q, 50).into_iter().map(|r| r.title).collect()
}

#[test]
fn fresh_item_is_reachable_by_a_token() {
    let doc = Doc::new().unwrap();
    doc.add_item(LIST_INBOX, "Buy groceries").unwrap();
    let r = doc.search("groceries", 50);
    assert_eq!(r.len(), 1);
    assert_eq!(r[0].kind, SearchKind::Item);
    assert_eq!(r[0].title, "Buy groceries");
    assert_eq!(r[0].list_id.as_deref(), Some(LIST_INBOX));
    assert_eq!(r[0].lifecycle, Some(ItemLifecycle::Backlog));
}

#[test]
fn index_is_current_without_draining_events() {
    // The web store drains the queue itself; the CLI never does. The
    // index must not depend on either.
    let doc = Doc::new().unwrap();
    doc.add_item(LIST_INBOX, "Buy groceries").unwrap();
    assert_eq!(ids(&doc, "groceries").len(), 1);
    assert!(!doc.drain_events().is_empty());
    assert_eq!(ids(&doc, "groceries").len(), 1);
}

#[test]
fn text_edit_swaps_tokens() {
    let doc = Doc::new().unwrap();
    let id = doc.add_item(LIST_INBOX, "Buy groceries").unwrap();
    assert_eq!(ids(&doc, "groceries"), std::slice::from_ref(&id));
    doc.edit_item_text(&id, "Read book").unwrap();
    assert!(ids(&doc, "groceries").is_empty());
    let r = doc.search("read", 50);
    assert_eq!(r.len(), 1);
    assert_eq!(r[0].id, id);
    assert_eq!(r[0].title, "Read book");
}

#[test]
fn notes_rank_below_title_for_the_same_query() {
    let doc = Doc::new().unwrap();
    let title_hit = doc.add_item(LIST_INBOX, "phoenix kickoff").unwrap();
    let notes_hit = doc.add_item(LIST_INBOX, "another item").unwrap();
    doc.apply_notes_delta(
        &notes_hit,
        &[NotesDeltaOp::Insert {
            insert: "see also: phoenix".into(),
        }],
    )
    .unwrap();
    assert_eq!(ids(&doc, "phoenix"), [title_hit, notes_hit.clone()]);
    let body = doc
        .search("phoenix", 50)
        .into_iter()
        .find(|r| r.id == notes_hit)
        .and_then(|r| r.body);
    assert_eq!(body.as_deref(), Some("see also: phoenix"));
}

#[test]
fn list_rename_updates_list_and_item_context() {
    let doc = Doc::new().unwrap();
    let list_id = doc.add_list("Work").unwrap();
    let item_id = doc.add_item(&list_id, "ship feature").unwrap();

    let r = ids(&doc, "work");
    assert!(r.contains(&list_id));
    assert!(r.contains(&item_id));

    doc.rename_list(&list_id, "Personal").unwrap();
    assert!(ids(&doc, "work").is_empty());
    let r = ids(&doc, "personal");
    assert!(r.contains(&list_id));
    assert!(r.contains(&item_id));

    let list = doc
        .search("personal", 50)
        .into_iter()
        .find(|r| r.id == list_id)
        .unwrap();
    assert_eq!(list.kind, SearchKind::List);
    assert_eq!(list.lifecycle, None);
}

#[test]
fn moving_an_item_changes_its_context_tokens() {
    let doc = Doc::new().unwrap();
    let work = doc.add_list("Work").unwrap();
    let home = doc.add_list("Home").unwrap();
    let item = doc.add_item(&work, "ship feature").unwrap();
    assert!(ids(&doc, "work").contains(&item));
    doc.move_item(&item, &home, 0).unwrap();
    assert!(!ids(&doc, "work").contains(&item));
    assert!(ids(&doc, "home").contains(&item));
}

#[test]
fn hard_delete_removes_the_item() {
    let doc = Doc::new().unwrap();
    let id = doc.add_item(LIST_INBOX, "Buy groceries").unwrap();
    assert_eq!(ids(&doc, "groceries").len(), 1);
    doc.set_item_binned(&id, true).unwrap();
    assert_eq!(ids(&doc, "groceries").len(), 1);
    doc.delete_binned(&id).unwrap();
    assert!(ids(&doc, "groceries").is_empty());
}

#[test]
fn multi_token_queries_are_and() {
    let doc = Doc::new().unwrap();
    doc.add_item(LIST_INBOX, "Buy groceries").unwrap();
    doc.add_item(LIST_INBOX, "Read book").unwrap();
    assert_eq!(ids(&doc, "buy groceries").len(), 1);
    assert!(ids(&doc, "buy book").is_empty());
}

#[test]
fn last_token_is_a_prefix() {
    let doc = Doc::new().unwrap();
    doc.add_item(LIST_INBOX, "Buy groceries").unwrap();
    doc.add_item(LIST_INBOX, "Read Phoenix spec").unwrap();
    doc.add_item(LIST_INBOX, "Plan team offsite").unwrap();
    assert_eq!(titles(&doc, "buy gro"), ["Buy groceries"]);
    assert_eq!(titles(&doc, "pho"), ["Read Phoenix spec"]);
    assert_eq!(titles(&doc, "off"), ["Plan team offsite"]);
    // Only the final token is a prefix; earlier ones must match whole.
    assert!(titles(&doc, "bu groceries").is_empty());
}

#[test]
fn accents_fold_both_ways() {
    let doc = Doc::new().unwrap();
    let id = doc.add_item(LIST_INBOX, "Leer el artículo").unwrap();
    assert_eq!(ids(&doc, "articulo"), std::slice::from_ref(&id));
    assert_eq!(ids(&doc, "ARTÍCULO"), [id]);
}

#[test]
fn lifecycle_tiebreak_orders_active_then_closed_then_binned() {
    let doc = Doc::new().unwrap();
    let live = doc.add_item(LIST_INBOX, "Apple").unwrap();
    let done = doc.add_item(LIST_INBOX, "Apple").unwrap();
    let binned = doc.add_item(LIST_INBOX, "Apple").unwrap();
    doc.set_item_done(&done, true).unwrap();
    doc.set_item_binned(&binned, true).unwrap();
    assert_eq!(ids(&doc, "apple"), [live, done, binned]);
}

#[test]
fn empty_and_punctuation_queries_return_nothing() {
    let doc = Doc::new().unwrap();
    doc.add_item(LIST_INBOX, "Buy groceries").unwrap();
    assert!(doc.search("", 50).is_empty());
    assert!(doc.search("  #! ", 50).is_empty());
}

#[test]
fn limit_caps_results() {
    let doc = Doc::new().unwrap();
    for i in 0..10 {
        doc.add_item(LIST_INBOX, &format!("Apple {i}")).unwrap();
    }
    assert_eq!(doc.search("apple", 3).len(), 3);
}

#[test]
fn survives_a_save_load_round_trip() {
    let doc = Doc::new().unwrap();
    let list = doc.add_list("Work").unwrap();
    let id = doc.add_item(&list, "ship feature").unwrap();
    let bytes = doc.save().unwrap();
    let again = Doc::load(&bytes).unwrap();
    assert_eq!(ids(&again, "ship"), std::slice::from_ref(&id));
    assert!(ids(&again, "work").contains(&id));
}
