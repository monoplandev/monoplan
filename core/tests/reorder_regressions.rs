//! Reorders inside a user list (Inbox is a queue and refuses them,
//! `spec/inbox-plan.md`).

use monoplan_core::doc::Doc;

#[test]
fn move_item_to_same_visible_slot_is_noop() {
    let doc = Doc::new().unwrap();
    let main = doc.add_list("Main").unwrap();
    let a = doc.add_item(&main, "a").unwrap();
    let b = doc.add_item(&main, "b").unwrap();
    let c = doc.add_item(&main, "c").unwrap();

    doc.move_item(&b, &main, 1).unwrap();

    assert_eq!(doc.open_item_ids(&main), vec![a, b, c]);
}

#[test]
fn move_item_down_uses_resulting_visible_index() {
    let doc = Doc::new().unwrap();
    let main = doc.add_list("Main").unwrap();
    let a = doc.add_item(&main, "a").unwrap();
    let b = doc.add_item(&main, "b").unwrap();
    let c = doc.add_item(&main, "c").unwrap();
    let d = doc.add_item(&main, "d").unwrap();

    doc.move_item(&b, &main, 2).unwrap();

    assert_eq!(doc.open_item_ids(&main), vec![a, c, b, d]);
}

#[test]
fn undo_redo_roundtrips_reorder_with_hidden_items() {
    let doc = Doc::new().unwrap();
    let main = doc.add_list("Main").unwrap();
    let other = doc.add_list("Other").unwrap();
    let a = doc.add_item(&main, "a").unwrap();
    let b = doc.add_item(&main, "b").unwrap();
    let c = doc.add_item(&main, "c").unwrap();
    let hidden = doc.add_item(&other, "hidden").unwrap();
    doc.set_item_done(&hidden, true).unwrap();

    doc.move_item(&a, &main, 2).unwrap();
    doc.move_item(&b, &other, 0).unwrap();
    let expected_main = doc.open_item_ids(&main);
    let expected_other = doc.open_item_ids(&other);

    while doc.can_undo() {
        assert!(doc.undo().unwrap());
    }
    assert!(doc.open_item_ids(&main).is_empty());
    assert!(doc.open_item_ids(&other).is_empty());

    while doc.can_redo() {
        assert!(doc.redo().unwrap());
    }
    assert_eq!(doc.open_item_ids(&main), expected_main);
    assert_eq!(doc.open_item_ids(&other), expected_other);
    assert_eq!(doc.open_item_ids(&main), vec![c, a]);
    assert_eq!(doc.open_item_ids(&other), vec![b]);
}
