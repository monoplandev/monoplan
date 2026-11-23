//! Multi-row moves for drag and drop (`spec/mac-dnd-plan.md` "Engine"):
//! a reorder inside a list, Starred or the lists themselves, a drop of
//! rows onto a list, and binning or starring rows from any workspace.
//!
//! Each function is the whole of one gesture, so a client makes one
//! typed call per drop and the loop records it as one undo step
//! (`crate::undo`), however many commits and docs it touches. The rules
//! are the web's (`js/web/src/Workspace.tsx` `onReorder` / `relocateTo`
//! / `onDndDragEnd`), kept here so every native client shares them.
//!
//! A reorder names its drop slot by `before`, the row the block lands
//! directly above (`None`: the end), not by index: it is what a drop
//! between two rows names, and it stays right when the client's rows
//! are a filtered view of the order (Starred's hidden entries).

use monoplan_core::{Doc, DocId, LIST_INBOX};
use serde::Serialize;

use super::Ctx;
use super::items::ItemState;
use crate::commands::focus::Lens;
use crate::workspace::{self, Candidate};

/// A row of a view that mixes workspaces (Starred, Calendar, Bin): the
/// item and the workspace it lives in, `workspace` in the selector
/// grammar a typed call takes (doc id or name; `None` is Home).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct ItemRef {
    pub workspace: Option<String>,
    pub id: String,
}

/// One step of a planned reorder: put `id` at `index` of the order as it
/// stands after the steps before it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReorderMove {
    pub id: String,
    pub index: usize,
}

/// Plan a reorder of `order` that gathers `moved` into one block, in
/// `order`'s order, directly above `before` (`None`: at the end), with
/// at most one step per moved row; the port of the web's
/// `planReorderMoves` (`js/web/src/reorder.ts`). Moving every displaced
/// row instead would cost a one-row drag the distance it travelled in
/// commits.
///
/// Ids in `moved` that `order` lacks are ignored. A `before` that is
/// itself moved stands for the first unmoved row after it; one `order`
/// lacks is the end.
pub fn plan_reorder(order: &[String], moved: &[String], before: Option<&str>) -> Vec<ReorderMove> {
    let is_moved = |id: &str| moved.iter().any(|m| m == id);
    let moved_ids: Vec<&String> = order.iter().filter(|id| is_moved(id)).collect();
    if moved_ids.is_empty() {
        return Vec::new();
    }
    let before = before.and_then(|b| {
        let at = order.iter().position(|id| id == b)?;
        order[at..]
            .iter()
            .find(|id| !is_moved(id))
            .map(String::as_str)
    });
    let remaining: Vec<&String> = order.iter().filter(|id| !is_moved(id)).collect();
    let insert_at = before
        .and_then(|b| remaining.iter().position(|id| *id == b))
        .unwrap_or(remaining.len());
    let anchor = before
        .and_then(|b| order.iter().position(|id| id == b))
        .unwrap_or(order.len());

    // Rows crossing the anchor from above move bottom-up, rows crossing
    // it from below top-down: one step each, even for a discontiguous
    // selection straddling the drop slot.
    let indexed: Vec<(usize, &String, usize)> = moved_ids
        .iter()
        .enumerate()
        .map(|(offset, id)| {
            let from = order
                .iter()
                .position(|o| o == *id)
                .expect("taken from order");
            (offset, *id, from)
        })
        .collect();
    let above = indexed.iter().filter(|(_, _, from)| *from < anchor).rev();
    let below = indexed.iter().filter(|(_, _, from)| *from > anchor);

    let mut current: Vec<&String> = order.iter().collect();
    let mut moves = Vec::new();
    for (offset, id, _) in above.chain(below) {
        let from = current.iter().position(|c| c == id).expect("still present");
        let index = insert_at + offset;
        if from == index {
            continue;
        }
        moves.push(ReorderMove {
            id: (*id).clone(),
            index,
        });
        let it = current.remove(from);
        current.insert(index, it);
    }
    debug_assert!({
        let mut expected = remaining.clone();
        for (i, id) in moved_ids.iter().enumerate() {
            expected.insert(insert_at + i, id);
        }
        current == expected
    });
    moves
}

/// Reorder within the rows' own list: `ids` become one block, in list
/// order, directly above `before` (`None`: at the end of the open
/// items). The list is the first id's; ids of another list, closed or
/// binned are left where they are. Refused for Inbox, which keeps
/// arrival order.
pub fn reorder_items(cx: &Ctx, ids: &[String], before: Option<&str>) -> anyhow::Result<()> {
    let Some(first) = ids.first() else {
        return Ok(());
    };
    let Some(item) = cx.doc.get_item(first) else {
        anyhow::bail!("item not found: {first}");
    };
    let list = item.list_id;
    if list == LIST_INBOX {
        // Inbox is a queue in arrival order (`spec/inbox-plan.md`).
        anyhow::bail!("Inbox can't be reordered");
    }
    let order: Vec<String> = super::items::item_views(cx, &list, false)
        .into_iter()
        .map(|i| i.id)
        .collect();
    for step in plan_reorder(&order, ids, before) {
        cx.doc.move_item(&step.id, &list, step.index)?;
    }
    Ok(())
}

/// Land `ids`, in the order given (the caller's visible order), as one
/// block from the 0-based `index` of `list`'s open items: a drop onto a
/// list in the sidebar passes 0, as the web's does. Binned rows are
/// restored first, since a move out of the Bin has no other meaning;
/// Done rows stay Done (reopening is its own gesture). Ids the
/// workspace no longer has are skipped. Onto Inbox the rows land at the
/// end of the queue and rows already there stay; among themselves they
/// sort as the queue does (arrival time, then creation), which is not
/// necessarily the order given.
pub fn move_items(cx: &Ctx, ids: &[String], list: &str, index: usize) -> anyhow::Result<()> {
    land(cx.doc, ids, list, index)
}

/// `move_items` over the doc the rows are in.
fn land(doc: &Doc, ids: &[String], list: &str, index: usize) -> anyhow::Result<()> {
    let mut at = index;
    for id in ids {
        let Some(item) = doc.get_item(id) else {
            continue;
        };
        if item.is_binned() {
            doc.set_item_binned(id, false)?;
        }
        // Inbox is a queue: a row already in it has no slot to move to
        // (and every row moved in lands at the end, whatever `index`).
        if list == LIST_INBOX && item.list_id == LIST_INBOX {
            continue;
        }
        doc.move_item(id, list, at)?;
        at += 1;
    }
    Ok(())
}

/// `move_items` for rows of any workspace: a drop from a view that
/// mixes them (Starred), or onto another workspace's list in the
/// sidebar. `items`, in the order given, land from the 0-based `index`
/// of `list` in the workspace `to` names (`None`: Home). Rows of that
/// workspace move as `move_items` moves them; the rest cross over as
/// `move_item_across` does (id and star kept, binned rows restored,
/// closed ones still closed) and sit directly under them, as the web's
/// `relocateTo` leaves a mixed block. One call, so one undo step over
/// every doc involved. Rows their workspace no longer has are skipped.
///
/// Asking whether the audience changes is the client's, before it
/// calls.
pub fn move_items_across(
    cx: &Ctx,
    items: &[ItemRef],
    to: Option<&str>,
    list: &str,
    index: usize,
) -> anyhow::Result<()> {
    let (dst, dst_id) = resolve(cx, to)?;
    let mut own: Vec<String> = Vec::new();
    // Each source doc's rows, docs in the order their first row came.
    let mut arriving: Vec<(Option<DocId>, &Doc, Vec<&str>)> = Vec::new();
    for r in items {
        let (src, src_id) = resolve(cx, r.workspace.as_deref())?;
        if src_id == dst_id {
            own.push(r.id.clone());
        } else if src.get_item(&r.id).is_some() {
            match arriving.iter_mut().find(|(id, _, _)| *id == src_id) {
                Some((_, _, ids)) => ids.push(&r.id),
                None => arriving.push((src_id, src, vec![&r.id])),
            }
        }
    }
    // A foreign ref names the doc; Home's own are bare.
    let target = dst_id
        .filter(|id| Some(*id) != cx.home)
        .map(|id| id.0.to_string());
    let mut at = index;
    for (_, src, ids) in &arriving {
        monoplan_core::move_items_across_docs(
            src,
            dst,
            ids,
            list,
            at,
            Some(monoplan_core::FocusRetarget {
                home: home_doc(cx)?,
                target_doc_id: target.as_deref(),
            }),
        )?;
        at += ids.len();
    }
    // Last, so they end above the arrivals.
    land(dst, &own, list, index)
}

/// Reorder Starred: `ids` become one block directly above `before`, a
/// starred id (`None`: the end). The plan runs over Home's every live
/// entry, the ones a client cannot show included (`spec/focus.md`), so
/// hidden entries keep their place around the block.
pub fn reorder_starred(cx: &Ctx, ids: &[String], before: Option<&str>) -> anyhow::Result<()> {
    let home = home_doc(cx)?;
    let order: Vec<String> = home
        .focus_entries()
        .into_iter()
        .map(|e| e.item_id)
        .collect();
    for step in plan_reorder(&order, ids, before) {
        home.move_in_focus(&step.id, step.index)?;
    }
    Ok(())
}

/// Reorder the workspace's active lists: `ids` become one block
/// directly above `before` (`None`: the end). Inbox and Events are not
/// in the order and the core refuses to move them.
pub fn move_lists(cx: &Ctx, ids: &[String], before: Option<&str>) -> anyhow::Result<()> {
    let order: Vec<String> = cx.doc.active_lists().into_iter().map(|l| l.id).collect();
    for step in plan_reorder(&order, ids, before) {
        cx.doc.move_list(&step.id, step.index)?;
    }
    Ok(())
}

/// Bin or restore rows from any workspace. Rows already so, and ones
/// their workspace no longer has, are skipped.
pub fn set_binned_many(cx: &Ctx, items: &[ItemRef], binned: bool) -> anyhow::Result<()> {
    for r in items {
        let (doc, _) = resolve(cx, r.workspace.as_deref())?;
        let Some(item) = doc.get_item(&r.id) else {
            continue;
        };
        if item.is_binned() != binned {
            doc.set_item_binned(&r.id, binned)?;
        }
    }
    Ok(())
}

/// Star rows from any workspace, on top of Starred in the order given.
/// Rows already starred, not Open, or gone are skipped, as the web's
/// `addToFocusMany`; each keeps its list.
pub fn star_items(cx: &Ctx, items: &[ItemRef]) -> anyhow::Result<()> {
    // Prepended last first, so the block reads top-down as given.
    for r in items.iter().rev() {
        let (doc, doc_id) = resolve(cx, r.workspace.as_deref())?;
        if !doc.get_item(&r.id).is_some_and(|i| i.is_open()) {
            continue;
        }
        match doc_id {
            Some(doc_id) if Some(doc_id) != cx.home => {
                Lens::foreign(home_doc(cx)?, doc_id).star(doc, &r.id, 0)?
            }
            Some(_) => Lens::home(doc).star(doc, &r.id, 0)?,
            // A bare context: the call's own doc and lens.
            None => cx.lens.star(doc, &r.id, 0)?,
        }
    }
    Ok(())
}

/// Untick a row Starred still shows (the done linger, a beat after its
/// tick): reopen it and star it again directly above `before`, the
/// starred item that now follows it on screen (`None`: the end), in one
/// call and so one undo step. Closing a Home item drops its entry, so
/// the reopen alone would leave it unstarred; a foreign entry outlives
/// the close and keeps its place, the star being a no-op. The web's
/// `undoneIntoFocus`.
pub fn reopen_into_starred(cx: &Ctx, item: &ItemRef, before: Option<&str>) -> anyhow::Result<()> {
    let (doc, doc_id) = resolve(cx, item.workspace.as_deref())?;
    doc.set_item_lifecycle(&item.id, ItemState::Backlog.into())?;
    let home = home_doc(cx)?;
    let index = before
        .and_then(|b| home.focus_entries().iter().position(|e| e.item_id == b))
        .unwrap_or(usize::MAX);
    match doc_id {
        Some(doc_id) if Some(doc_id) != cx.home => {
            Lens::foreign(home, doc_id).star(doc, &item.id, index)
        }
        Some(_) => Lens::home(doc).star(doc, &item.id, index),
        None => cx.lens.star(doc, &item.id, index),
    }
}

/// Move one item to `list` of the workspace `to` names (`None`: Home),
/// at the end of that list's open items: what an item page's list
/// picker does when the pick is another workspace's list
/// (`spec/sharing-plan.md` "Cross-doc move"). The id is kept, a Starred
/// entry follows the item, a binned item lands restored and a closed
/// one stays closed. The target adopts, Home re-points Starred, then
/// the source releases, so the loop records one undo step over the
/// docs involved. A `to` that is the item's own workspace is the plain
/// move to the end of `list`.
///
/// Asking whether the audience changes (into or out of a workspace
/// with other members) is the client's, before it calls.
pub fn move_item_across(
    cx: &Ctx,
    item: &ItemRef,
    to: Option<&str>,
    list: &str,
) -> anyhow::Result<()> {
    let (src, src_id) = resolve(cx, item.workspace.as_deref())?;
    let (dst, dst_id) = resolve(cx, to)?;
    let index = dst.items_in_list(list, true).len();
    if src_id == dst_id {
        anyhow::ensure!(
            src.get_item(&item.id).is_some(),
            "item not found: {}",
            item.id
        );
        src.move_item(&item.id, list, index)?;
        return Ok(());
    }
    // A foreign ref names the doc; Home's own are bare.
    let target = dst_id
        .filter(|id| Some(*id) != cx.home)
        .map(|id| id.0.to_string());
    monoplan_core::move_items_across_docs(
        src,
        dst,
        &[item.id.as_str()],
        list,
        index,
        Some(monoplan_core::FocusRetarget {
            home: home_doc(cx)?,
            target_doc_id: target.as_deref(),
        }),
    )?;
    Ok(())
}

/// The doc `selector` names among the call's workspaces, with its id.
/// A bare context (no workspaces, tests) has only its own doc, named by
/// `None`.
fn resolve<'a>(cx: &Ctx<'a>, selector: Option<&str>) -> anyhow::Result<(&'a Doc, Option<DocId>)> {
    if cx.workspaces.is_empty() {
        anyhow::ensure!(
            selector.is_none(),
            "no workspace {selector:?} in this context"
        );
        return Ok((cx.doc, None));
    }
    let Some(home) = cx.home else {
        anyhow::bail!("the Home workspace is not open");
    };
    let candidates: Vec<Candidate> = cx.workspaces.iter().map(|(c, _)| c.clone()).collect();
    let id = workspace::resolve(home, &candidates, selector)?;
    let doc = cx
        .workspaces
        .iter()
        .find(|(c, _)| c.doc_id == id)
        .map(|(_, d)| *d)
        .expect("resolved among the candidates");
    Ok((doc, Some(id)))
}

/// Home, whose Starred it is: the call's own doc in a bare context.
fn home_doc<'a>(cx: &Ctx<'a>) -> anyhow::Result<&'a Doc> {
    if cx.workspaces.is_empty() {
        Ok(cx.doc)
    } else {
        cx.home_doc()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::undo::History;
    use monoplan_core::LIST_INBOX;

    fn s(ids: &[&str]) -> Vec<String> {
        ids.iter().map(|i| i.to_string()).collect()
    }

    fn apply(order: &[String], moves: &[ReorderMove]) -> Vec<String> {
        let mut out = order.to_vec();
        for m in moves {
            let from = out
                .iter()
                .position(|o| *o == m.id)
                .expect("planned id present");
            let id = out.remove(from);
            out.insert(m.index, id);
        }
        out
    }

    fn mv(id: &str, index: usize) -> ReorderMove {
        ReorderMove {
            id: id.into(),
            index,
        }
    }

    const IDS: [&str; 6] = ["a", "b", "c", "d", "e", "f"];

    #[test]
    fn one_row_down_is_one_move_whatever_the_distance() {
        let ids = s(&IDS);
        let moves = plan_reorder(&ids, &s(&["a"]), None);
        assert_eq!(moves, vec![mv("a", 5)]);
        assert_eq!(apply(&ids, &moves), s(&["b", "c", "d", "e", "f", "a"]));
    }

    #[test]
    fn one_row_up_is_one_move() {
        let ids = s(&IDS);
        let moves = plan_reorder(&ids, &s(&["f"]), Some("b"));
        assert_eq!(moves, vec![mv("f", 1)]);
        assert_eq!(apply(&ids, &moves), s(&["a", "f", "b", "c", "d", "e"]));
    }

    #[test]
    fn a_contiguous_block_keeps_its_order_both_ways() {
        let ids = s(&IDS);
        let down = plan_reorder(&ids, &s(&["b", "c"]), None);
        assert_eq!(down.len(), 2);
        assert_eq!(apply(&ids, &down), s(&["a", "d", "e", "f", "b", "c"]));
        let up = plan_reorder(&ids, &s(&["e", "f"]), Some("b"));
        assert_eq!(up.len(), 2);
        assert_eq!(apply(&ids, &up), s(&["a", "e", "f", "b", "c", "d"]));
    }

    #[test]
    fn a_discontiguous_selection_becomes_one_block_in_order() {
        let ids = s(&IDS);
        // Given out of order: the block still takes the list's order.
        let moves = plan_reorder(&ids, &s(&["d", "b"]), None);
        assert_eq!(moves.len(), 2);
        assert_eq!(apply(&ids, &moves), s(&["a", "c", "e", "f", "b", "d"]));
    }

    #[test]
    fn dropping_where_the_block_already_is_plans_nothing() {
        assert!(plan_reorder(&s(&IDS), &s(&["b", "c"]), Some("d")).is_empty());
        assert!(plan_reorder(&s(&IDS), &s(&["e", "f"]), None).is_empty());
    }

    #[test]
    fn a_moved_before_stands_for_the_next_unmoved_row() {
        let ids = s(&IDS);
        // `c` is dragged too: the slot is above `e`, the first unmoved
        // row after it.
        let moves = plan_reorder(&ids, &s(&["a", "c", "d"]), Some("c"));
        assert_eq!(apply(&ids, &moves), s(&["b", "a", "c", "d", "e", "f"]));
        // Nothing unmoved after it: the end.
        let moves = plan_reorder(&ids, &s(&["a", "f"]), Some("f"));
        assert_eq!(apply(&ids, &moves), s(&["b", "c", "d", "e", "a", "f"]));
    }

    #[test]
    fn unknown_ids_are_ignored() {
        let ids = s(&IDS);
        assert!(plan_reorder(&ids, &s(&["x"]), None).is_empty());
        let moves = plan_reorder(&ids, &s(&["x", "a"]), Some("nope"));
        assert_eq!(apply(&ids, &moves), s(&["b", "c", "d", "e", "f", "a"]));
    }

    #[test]
    fn every_selection_and_slot_converges_in_one_move_per_row() {
        let ids = s(&IDS);
        for mask in 1u32..(1 << ids.len()) {
            let moved: Vec<String> = ids
                .iter()
                .enumerate()
                .filter(|(i, _)| mask & (1 << i) != 0)
                .map(|(_, id)| id.clone())
                .collect();
            let remaining: Vec<String> = ids
                .iter()
                .filter(|id| !moved.contains(id))
                .cloned()
                .collect();
            let slots = remaining.iter().map(|r| Some(r.as_str())).chain([None]);
            for before in slots {
                let at = before
                    .map(|b| remaining.iter().position(|r| r == b).unwrap())
                    .unwrap_or(remaining.len());
                let mut expected = remaining.clone();
                for (i, id) in moved.iter().enumerate() {
                    expected.insert(at + i, id.clone());
                }
                let moves = plan_reorder(&ids, &moved, before);
                assert!(moves.len() <= moved.len(), "{moved:?} before {before:?}");
                assert_eq!(apply(&ids, &moves), expected, "{moved:?} before {before:?}");
            }
        }
    }

    // ----- the calls, over bare docs -----

    fn add(doc: &Doc, list: &str, texts: &[&str]) -> Vec<String> {
        texts
            .iter()
            .map(|t| doc.add_item(list, t).unwrap())
            .collect()
    }

    fn open_texts(doc: &Doc, list: &str) -> Vec<String> {
        doc.items_in_list(list, false)
            .into_iter()
            .filter(|i| i.is_listed())
            .map(|i| i.text)
            .collect()
    }

    fn starred_texts(doc: &Doc) -> Vec<String> {
        doc.focus_entries()
            .into_iter()
            .map(|e| doc.get_item(&e.item_id).unwrap().text)
            .collect()
    }

    /// Run `f` as the loop runs a typed call: one `History` entry.
    fn as_call(doc: &Doc, history: &mut History, f: impl FnOnce(&Ctx)) {
        let lens = Lens::home(doc);
        let docs = [doc];
        let cx = Ctx::bare(doc, &docs, &lens);
        let tagged = [(DocId(uuid::Uuid::from_bytes([1; 16])), doc)];
        let before = history.before(&tagged);
        f(&cx);
        history.record(before, &tagged);
    }

    fn undo(doc: &Doc, history: &mut History) {
        let tagged = [(DocId(uuid::Uuid::from_bytes([1; 16])), doc)];
        assert!(history.undo(&tagged).unwrap());
    }

    #[test]
    fn reorder_items_moves_a_discontiguous_block_as_one_undo_step() {
        let doc = Doc::new().unwrap();
        let list = doc.add_list("Work").unwrap();
        let ids = add(&doc, &list, &["a", "b", "c", "d", "e"]);
        let mut h = History::default();
        as_call(&doc, &mut h, |cx| {
            reorder_items(cx, &[ids[3].clone(), ids[0].clone()], Some(&ids[2])).unwrap()
        });
        assert_eq!(open_texts(&doc, &list), s(&["b", "a", "d", "c", "e"]));
        undo(&doc, &mut h);
        assert_eq!(open_texts(&doc, &list), s(&["a", "b", "c", "d", "e"]));
    }

    #[test]
    fn reorder_items_to_the_end_and_leaves_done_rows_alone() {
        let doc = Doc::new().unwrap();
        let list = doc.add_list("Work").unwrap();
        let ids = add(&doc, &list, &["a", "b", "c"]);
        let done = add(&doc, &list, &["x"]).remove(0);
        doc.set_item_done(&done, true).unwrap();
        let mut h = History::default();
        as_call(&doc, &mut h, |cx| {
            reorder_items(cx, &[ids[0].clone(), done.clone()], None).unwrap()
        });
        assert_eq!(open_texts(&doc, &list), s(&["b", "c", "a"]));
        assert!(doc.get_item(&done).unwrap().is_closed());
    }

    #[test]
    fn reorder_items_refuses_inbox() {
        let doc = Doc::new().unwrap();
        let ids = add(&doc, LIST_INBOX, &["a", "b"]);
        let lens = Lens::home(&doc);
        let docs = [&doc];
        let cx = Ctx::bare(&doc, &docs, &lens);
        assert!(reorder_items(&cx, &[ids[1].clone()], Some(&ids[0])).is_err());
        assert_eq!(open_texts(&doc, LIST_INBOX), s(&["a", "b"]));
    }

    #[test]
    fn move_items_onto_inbox_queues_at_the_end_and_skips_rows_already_there() {
        let doc = Doc::new().unwrap();
        let list = doc.add_list("Work").unwrap();
        let inbox = add(&doc, LIST_INBOX, &["a", "b"]);
        let work = add(&doc, &list, &["w1", "w2"]);
        let lens = Lens::home(&doc);
        let docs = [&doc];
        let cx = Ctx::bare(&doc, &docs, &lens);
        let given = [work[1].clone(), inbox[0].clone(), work[0].clone()];
        move_items(&cx, &given, LIST_INBOX, 0).unwrap();
        // The arrivals are behind what was there. Their own order hangs
        // on whether the two moves share a millisecond (the tiebreak is
        // creation time), so it is not asserted.
        let open = open_texts(&doc, LIST_INBOX);
        assert_eq!(open[..2], s(&["a", "b"]));
        let mut arrived = open[2..].to_vec();
        arrived.sort();
        assert_eq!(arrived, s(&["w1", "w2"]));
    }

    #[test]
    fn move_items_lands_a_block_on_top_in_the_given_order_restoring_binned_rows() {
        let doc = Doc::new().unwrap();
        let list = doc.add_list("Work").unwrap();
        add(&doc, &list, &["w1", "w2"]);
        let ids = add(&doc, LIST_INBOX, &["a", "b", "c"]);
        doc.set_item_binned(&ids[2], true).unwrap();
        let mut h = History::default();
        as_call(&doc, &mut h, |cx| {
            let given = [ids[2].clone(), "gone".to_string(), ids[0].clone()];
            move_items(cx, &given, &list, 0).unwrap()
        });
        assert_eq!(open_texts(&doc, &list), s(&["c", "a", "w1", "w2"]));
        assert_eq!(open_texts(&doc, LIST_INBOX), s(&["b"]));
        assert!(!doc.get_item(&ids[2]).unwrap().is_binned());
        undo(&doc, &mut h);
        assert_eq!(open_texts(&doc, LIST_INBOX), s(&["a", "b"]));
        assert!(doc.get_item(&ids[2]).unwrap().is_binned());
        assert_eq!(open_texts(&doc, &list), s(&["w1", "w2"]));
    }

    #[test]
    fn move_items_keeps_done_rows_done() {
        let doc = Doc::new().unwrap();
        let list = doc.add_list("Work").unwrap();
        let id = add(&doc, LIST_INBOX, &["a"]).remove(0);
        doc.set_item_done(&id, true).unwrap();
        let lens = Lens::home(&doc);
        let docs = [&doc];
        move_items(
            &Ctx::bare(&doc, &docs, &lens),
            std::slice::from_ref(&id),
            &list,
            0,
        )
        .unwrap();
        let item = doc.get_item(&id).unwrap();
        assert_eq!(item.list_id, list);
        assert!(item.is_closed());
    }

    #[test]
    fn reorder_starred_plans_over_every_entry() {
        let doc = Doc::new().unwrap();
        let ids = add(&doc, LIST_INBOX, &["a", "b", "c", "d"]);
        for id in &ids {
            doc.add_to_focus(id, usize::MAX).unwrap();
        }
        let mut h = History::default();
        as_call(&doc, &mut h, |cx| {
            reorder_starred(cx, &[ids[2].clone(), ids[3].clone()], Some(&ids[0])).unwrap()
        });
        assert_eq!(starred_texts(&doc), s(&["c", "d", "a", "b"]));
        undo(&doc, &mut h);
        assert_eq!(starred_texts(&doc), s(&["a", "b", "c", "d"]));
    }

    #[test]
    fn move_lists_reorders_active_lists() {
        let doc = Doc::new().unwrap();
        let a = doc.add_list("A").unwrap();
        let b = doc.add_list("B").unwrap();
        let c = doc.add_list("C").unwrap();
        let mut h = History::default();
        as_call(&doc, &mut h, |cx| {
            move_lists(cx, &[c.clone(), b.clone()], Some(&a)).unwrap()
        });
        let names: Vec<String> = doc.active_lists().into_iter().map(|l| l.name).collect();
        assert_eq!(names, s(&["B", "C", "A"]));
        undo(&doc, &mut h);
        let names: Vec<String> = doc.active_lists().into_iter().map(|l| l.name).collect();
        assert_eq!(names, s(&["A", "B", "C"]));
    }

    fn refs(ids: &[String]) -> Vec<ItemRef> {
        ids.iter()
            .map(|id| ItemRef {
                workspace: None,
                id: id.clone(),
            })
            .collect()
    }

    #[test]
    fn set_binned_many_skips_rows_already_so() {
        let doc = Doc::new().unwrap();
        let ids = add(&doc, LIST_INBOX, &["a", "b", "c"]);
        doc.set_item_binned(&ids[1], true).unwrap();
        let mut h = History::default();
        as_call(&doc, &mut h, |cx| {
            let mut given = refs(&ids);
            given.push(ItemRef {
                workspace: None,
                id: "gone".into(),
            });
            set_binned_many(cx, &given, true).unwrap()
        });
        assert!(ids.iter().all(|id| doc.get_item(id).unwrap().is_binned()));
        undo(&doc, &mut h);
        assert_eq!(open_texts(&doc, LIST_INBOX), s(&["a", "c"]));
    }

    #[test]
    fn star_items_prepends_the_block_in_order_skipping_starred_and_closed() {
        let doc = Doc::new().unwrap();
        let ids = add(&doc, LIST_INBOX, &["a", "b", "c", "d"]);
        doc.add_to_focus(&ids[3], 0).unwrap();
        doc.add_to_focus(&ids[1], 0).unwrap();
        doc.set_item_done(&ids[2], true).unwrap();
        let mut h = History::default();
        as_call(&doc, &mut h, |cx| star_items(cx, &refs(&ids)).unwrap());
        // `b` stays where it was; `c` is closed; `a` lands on top.
        assert_eq!(starred_texts(&doc), s(&["a", "b", "d"]));
        undo(&doc, &mut h);
        assert_eq!(starred_texts(&doc), s(&["b", "d"]));
    }

    #[test]
    fn reopen_into_starred_puts_the_row_back_above_before_in_one_step() {
        let doc = Doc::new().unwrap();
        let ids = add(&doc, LIST_INBOX, &["a", "b", "c"]);
        for id in ids.iter().rev() {
            doc.add_to_focus(id, 0).unwrap();
        }
        doc.set_item_done(&ids[1], true).unwrap();
        assert_eq!(starred_texts(&doc), s(&["a", "c"]));
        let mut h = History::default();
        let given = ItemRef {
            workspace: None,
            id: ids[1].clone(),
        };
        as_call(&doc, &mut h, |cx| {
            reopen_into_starred(cx, &given, Some(&ids[2])).unwrap()
        });
        assert_eq!(starred_texts(&doc), s(&["a", "b", "c"]));
        assert!(doc.get_item(&ids[1]).unwrap().is_open());
        // One step back: closed and unstarred again.
        undo(&doc, &mut h);
        assert_eq!(starred_texts(&doc), s(&["a", "c"]));
        assert!(!doc.get_item(&ids[1]).unwrap().is_open());
        // No row after it: the end.
        as_call(&doc, &mut h, |cx| {
            reopen_into_starred(cx, &given, None).unwrap()
        });
        assert_eq!(starred_texts(&doc), s(&["a", "c", "b"]));
    }

    #[test]
    fn rows_of_two_workspaces_bin_and_star_as_one_call() {
        let home = Doc::new().unwrap();
        let work = Doc::new().unwrap();
        let home_id = DocId(uuid::Uuid::from_bytes([1; 16]));
        let work_id = DocId(uuid::Uuid::from_bytes([2; 16]));
        let h_ids = add(&home, LIST_INBOX, &["h1", "h2"]);
        let w_ids = add(&work, LIST_INBOX, &["w1", "w2"]);
        let workspaces = [
            (
                Candidate {
                    doc_id: home_id,
                    name: None,
                },
                &home,
            ),
            (
                Candidate {
                    doc_id: work_id,
                    name: Some("Work".into()),
                },
                &work,
            ),
        ];
        let docs = [&home, &work];
        let lens = Lens::home(&home);
        let cx = Ctx {
            doc: &home,
            open_docs: &docs,
            lens: &lens,
            workspaces: &workspaces,
            home: Some(home_id),
        };
        let tagged = [(home_id, &home), (work_id, &work)];
        let mut h = History::default();

        // Work's row by name, then by id; Home's by `None`.
        let given = [
            ItemRef {
                workspace: Some("Work".into()),
                id: w_ids[0].clone(),
            },
            ItemRef {
                workspace: None,
                id: h_ids[0].clone(),
            },
            ItemRef {
                workspace: Some(work_id.0.to_string()),
                id: w_ids[1].clone(),
            },
        ];
        let before = h.before(&tagged);
        star_items(&cx, &given).unwrap();
        h.record(before, &tagged);
        let entries = home.focus_entries();
        let got: Vec<(Option<String>, &str)> = entries
            .iter()
            .map(|e| (e.doc_id.clone(), e.item_id.as_str()))
            .collect();
        let work_tag = Some(work_id.0.to_string());
        assert_eq!(
            got,
            vec![
                (work_tag.clone(), w_ids[0].as_str()),
                (None, h_ids[0].as_str()),
                (work_tag, w_ids[1].as_str()),
            ]
        );

        let before = h.before(&tagged);
        set_binned_many(&cx, &given, true).unwrap();
        h.record(before, &tagged);
        assert_eq!(open_texts(&home, LIST_INBOX), s(&["h2"]));
        assert!(open_texts(&work, LIST_INBOX).is_empty());
        // One step back restores both workspaces.
        assert!(h.undo(&tagged).unwrap());
        assert_eq!(open_texts(&home, LIST_INBOX), s(&["h1", "h2"]));
        assert_eq!(open_texts(&work, LIST_INBOX), s(&["w1", "w2"]));
    }

    #[test]
    fn an_item_moves_to_another_workspace_with_its_star_and_comes_back_on_undo() {
        let home = Doc::new().unwrap();
        let work = Doc::new().unwrap();
        let home_id = DocId(uuid::Uuid::from_bytes([1; 16]));
        let work_id = DocId(uuid::Uuid::from_bytes([2; 16]));
        let h_ids = add(&home, LIST_INBOX, &["h1", "h2"]);
        let list = work.add_list("Plans").unwrap();
        add(&work, &list, &["w1"]);
        let workspaces = [
            (
                Candidate {
                    doc_id: home_id,
                    name: None,
                },
                &home,
            ),
            (
                Candidate {
                    doc_id: work_id,
                    name: Some("Work".into()),
                },
                &work,
            ),
        ];
        let docs = [&home, &work];
        let lens = Lens::home(&home);
        let cx = Ctx {
            doc: &home,
            open_docs: &docs,
            lens: &lens,
            workspaces: &workspaces,
            home: Some(home_id),
        };
        let tagged = [(home_id, &home), (work_id, &work)];
        let mut h = History::default();
        lens.star(&home, &h_ids[0], 0).unwrap();

        let moved = ItemRef {
            workspace: None,
            id: h_ids[0].clone(),
        };
        let before = h.before(&tagged);
        move_item_across(&cx, &moved, Some("Work"), &list).unwrap();
        h.record(before, &tagged);
        // Appended, under its own id; gone from Home; the star follows.
        assert_eq!(open_texts(&home, LIST_INBOX), s(&["h2"]));
        assert_eq!(open_texts(&work, &list), s(&["w1", "h1"]));
        assert_eq!(work.get_item(&h_ids[0]).unwrap().text, "h1");
        let entries = home.focus_entries();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].doc_id, Some(work_id.0.to_string()));

        // One step back undoes both halves and the star's re-pointing.
        assert!(h.undo(&tagged).unwrap());
        assert_eq!(open_texts(&home, LIST_INBOX), s(&["h1", "h2"]));
        assert_eq!(open_texts(&work, &list), s(&["w1"]));
        assert_eq!(home.focus_entries()[0].doc_id, None);

        // Back again by the other workspace's selector, into Home.
        assert!(h.redo(&tagged).unwrap());
        let moved = ItemRef {
            workspace: Some(work_id.0.to_string()),
            id: h_ids[0].clone(),
        };
        move_item_across(&cx, &moved, None, LIST_INBOX).unwrap();
        assert!(work.get_item(&h_ids[0]).is_none());
        assert!(home.get_item(&h_ids[0]).is_some());
        assert_eq!(home.focus_entries()[0].doc_id, None);

        // Its own workspace named as the target is the plain move.
        let stay = ItemRef {
            workspace: Some("Work".into()),
            id: work.items_in_list(&list, true)[0].id.clone(),
        };
        move_item_across(&cx, &stay, Some("Work"), LIST_INBOX).unwrap();
        assert_eq!(open_texts(&work, LIST_INBOX), s(&["w1"]));
    }

    #[test]
    fn rows_of_several_workspaces_land_on_one_list_in_one_step() {
        let home = Doc::new().unwrap();
        let work = Doc::new().unwrap();
        let home_id = DocId(uuid::Uuid::from_bytes([1; 16]));
        let work_id = DocId(uuid::Uuid::from_bytes([2; 16]));
        let h_ids = add(&home, LIST_INBOX, &["h1", "h2", "h3"]);
        let list = work.add_list("Plans").unwrap();
        let w_ids = add(&work, &list, &["w1", "w2", "w3"]);
        let workspaces = [
            (
                Candidate {
                    doc_id: home_id,
                    name: None,
                },
                &home,
            ),
            (
                Candidate {
                    doc_id: work_id,
                    name: Some("Work".into()),
                },
                &work,
            ),
        ];
        let docs = [&home, &work];
        let lens = Lens::home(&home);
        let cx = Ctx {
            doc: &home,
            open_docs: &docs,
            lens: &lens,
            workspaces: &workspaces,
            home: Some(home_id),
        };
        let tagged = [(home_id, &home), (work_id, &work)];
        let mut h = History::default();
        lens.star(&home, &h_ids[2], 0).unwrap();
        home.set_item_binned(&h_ids[0], true).unwrap();

        // A mixed block, as Starred would give it: Home's rows by
        // nothing or by doc id, Work's by name.
        let given = [
            ItemRef {
                workspace: None,
                id: h_ids[2].clone(),
            },
            ItemRef {
                workspace: Some("Work".into()),
                id: w_ids[2].clone(),
            },
            ItemRef {
                workspace: Some(home_id.0.to_string()),
                id: h_ids[0].clone(),
            },
            ItemRef {
                workspace: None,
                id: "gone".into(),
            },
        ];
        let before = h.before(&tagged);
        move_items_across(&cx, &given, Some("Work"), &list, 0).unwrap();
        h.record(before, &tagged);
        // The list's own row on top, then the arrivals in the order
        // given, the binned one restored.
        assert_eq!(open_texts(&work, &list), s(&["w3", "h3", "h1", "w1", "w2"]));
        assert_eq!(open_texts(&home, LIST_INBOX), s(&["h2"]));
        assert!(home.get_item(&h_ids[0]).is_none());
        assert_eq!(home.focus_entries()[0].doc_id, Some(work_id.0.to_string()));

        assert!(h.undo(&tagged).unwrap());
        assert_eq!(open_texts(&work, &list), s(&["w1", "w2", "w3"]));
        assert_eq!(open_texts(&home, LIST_INBOX), s(&["h2", "h3"]));
        assert!(home.get_item(&h_ids[0]).unwrap().is_binned());
        assert_eq!(home.focus_entries()[0].doc_id, None);

        // With every row already the target's, it is `move_items`.
        let own = [ItemRef {
            workspace: Some("Work".into()),
            id: w_ids[1].clone(),
        }];
        move_items_across(&cx, &own, Some("Work"), &list, 0).unwrap();
        assert_eq!(open_texts(&work, &list), s(&["w2", "w1", "w3"]));
    }

    #[test]
    fn a_workspace_a_bare_context_lacks_is_an_error() {
        let doc = Doc::new().unwrap();
        let lens = Lens::home(&doc);
        let docs = [&doc];
        let given = [ItemRef {
            workspace: Some("Work".into()),
            id: "x".into(),
        }];
        assert!(set_binned_many(&Ctx::bare(&doc, &docs, &lens), &given, true).is_err());
    }
}
