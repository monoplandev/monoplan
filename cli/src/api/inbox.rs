//! The combined Inbox: every open workspace's Inbox as one queue
//! (`spec/inbox-plan.md` "One Inbox"). Each doc keeps its own `inbox`
//! bucket; this is a read over them, in the queue's order.

use serde::Serialize;

use super::{Ctx, doc_tag, items::Item};
use monoplan_core::{Doc, ItemView, inbox};

/// One row of the combined Inbox: the item and its workspace.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct InboxEntry {
    #[serde(flatten)]
    pub item: Item,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub doc_id: Option<String>,
}

/// The combined Inbox's rows as one arrival-ordered queue, oldest
/// first: what each doc's Inbox shows (Open, not binned, no date) in
/// the workspaces `filter` names (doc ids), or every open workspace
/// when it is empty or names none that is open
/// (`monoplan_core::inbox::scope`), merged on the queue key
/// (`monoplan_core::inbox::merge`). Newest first is the device's
/// preference; the caller reverses.
pub fn inbox(cx: &Ctx, filter: &[String]) -> anyhow::Result<Vec<InboxEntry>> {
    let sources: Vec<(Option<String>, &Doc)> = cx
        .sources()
        .into_iter()
        .map(|(tag, doc)| (doc_tag(tag).0, doc))
        .collect();
    let open: Vec<Option<String>> = sources.iter().map(|(k, _)| k.clone()).collect();
    let filter: Vec<Option<String>> = filter.iter().cloned().map(Some).collect();
    let shown = inbox::scope(&open, &filter);
    let mut rows: Vec<(Option<String>, ItemView)> = sources
        .iter()
        .filter(|(k, _)| shown.contains(&k))
        .flat_map(|(k, doc)| {
            inbox::doc_rows(doc, |_| false)
                .into_iter()
                .map(move |i| (k.clone(), i))
        })
        .collect();
    inbox::merge(&mut rows);
    Ok(rows
        .into_iter()
        .map(|(doc_id, i)| InboxEntry {
            item: Item::from(&i),
            doc_id,
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::focus::Lens;
    use monoplan_core::LIST_INBOX;

    #[test]
    fn merges_every_docs_inbox_in_arrival_order() {
        let home = Doc::new().unwrap();
        let work = Doc::new().unwrap();
        let add = |doc: &Doc, list: &str, text: &str| {
            let id = doc.add_item(list, text).unwrap();
            std::thread::sleep(std::time::Duration::from_millis(2));
            id
        };
        let a = add(&work, LIST_INBOX, "a");
        let b = add(&home, LIST_INBOX, "b");
        let c = add(&work, LIST_INBOX, "c");
        let dated = add(&home, LIST_INBOX, "dated");
        home.set_item_when(&dated, Some("2026-12-01")).unwrap();
        let done = add(&work, LIST_INBOX, "done");
        work.set_item_done(&done, true).unwrap();
        let binned = add(&home, LIST_INBOX, "binned");
        home.set_item_binned(&binned, true).unwrap();
        let list = work.add_list("Work").unwrap();
        add(&work, &list, "listed");
        // Moved in last: it queues behind everything.
        let moved = add(&home, &home.add_list("Elsewhere").unwrap(), "moved");
        home.move_item(&moved, LIST_INBOX, 0).unwrap();

        let lens = Lens::home(&home);
        let docs = [&home, &work];
        let cx = Ctx::bare(&home, &docs, &lens);
        let ids: Vec<String> = inbox(&cx, &[])
            .unwrap()
            .into_iter()
            .map(|r| r.item.id)
            .collect();
        assert_eq!(ids, [a, b, c, moved]);
    }

    #[test]
    fn filter_narrows_to_the_named_workspaces() {
        use crate::workspace::Candidate;
        use monoplan_core::DocId;
        let docs = [
            Doc::new().unwrap(),
            Doc::new().unwrap(),
            Doc::new().unwrap(),
        ];
        let ids: Vec<DocId> = (0..3).map(|_| DocId(uuid::Uuid::now_v7())).collect();
        let items: Vec<String> = docs
            .iter()
            .map(|d| d.add_item(LIST_INBOX, "x").unwrap())
            .collect();
        let workspaces: Vec<(Candidate, &Doc)> = ids
            .iter()
            .zip(&docs)
            .map(|(id, d)| {
                (
                    Candidate {
                        doc_id: *id,
                        name: None,
                    },
                    d,
                )
            })
            .collect();
        let lens = Lens::home(&docs[0]);
        let open: Vec<&Doc> = docs.iter().collect();
        let mut cx = Ctx::bare(&docs[0], &open, &lens);
        cx.workspaces = &workspaces;
        let shown = |filter: &[DocId]| -> Vec<String> {
            let filter: Vec<String> = filter.iter().map(|d| d.0.to_string()).collect();
            let mut got: Vec<String> = inbox(&cx, &filter)
                .unwrap()
                .into_iter()
                .map(|r| r.item.id)
                .collect();
            got.sort();
            got
        };
        let sorted = |mut v: Vec<String>| {
            v.sort();
            v
        };
        assert_eq!(shown(&[]), sorted(items.clone()));
        assert_eq!(
            shown(&[ids[2], ids[0]]),
            sorted(vec![items[0].clone(), items[2].clone()])
        );
        // A filter naming no open workspace reads as every workspace.
        assert_eq!(shown(&[DocId(uuid::Uuid::now_v7())]), sorted(items));
    }
}
