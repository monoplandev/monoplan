//! The combined Inbox: every open workspace's Inbox as one queue
//! (`spec/inbox-plan.md` "One Inbox"). Each doc keeps its own `inbox`
//! bucket; this is a read over them, in the queue's order.

use serde::Serialize;

use super::{Ctx, doc_tag, items::Item};
use monoplan_core::LIST_INBOX;

/// One row of the combined Inbox: the item and its workspace.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct InboxEntry {
    #[serde(flatten)]
    pub item: Item,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub doc_id: Option<String>,
}

/// Every open workspace's Inbox rows as one arrival-ordered queue,
/// oldest first: what each doc's Inbox shows (Open, not binned, no
/// date), merged on the queue key `(inbox_at ?? created_at,
/// created_at)`, then the workspace and the id. Newest first is the
/// device's preference; the caller reverses.
pub fn inbox(cx: &Ctx) -> anyhow::Result<Vec<InboxEntry>> {
    let mut rows: Vec<((i64, i64), InboxEntry)> = cx
        .sources()
        .into_iter()
        .flat_map(|(tag, doc)| {
            let doc_id = doc_tag(tag).0;
            let mut items = doc.items_in_list(LIST_INBOX, false);
            items.retain(|i| i.is_listed());
            items
                .into_iter()
                .map(|i| {
                    let key = (i.inbox_at.unwrap_or(i.created_at), i.created_at);
                    (
                        key,
                        InboxEntry {
                            item: Item::from(&i),
                            doc_id: doc_id.clone(),
                        },
                    )
                })
                .collect::<Vec<_>>()
        })
        .collect();
    rows.sort_by(|(ka, a), (kb, b)| {
        ka.cmp(kb)
            .then_with(|| (&a.doc_id, &a.item.id).cmp(&(&b.doc_id, &b.item.id)))
    });
    Ok(rows.into_iter().map(|(_, r)| r).collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::focus::Lens;
    use monoplan_core::Doc;

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
        let ids: Vec<String> = inbox(&cx).unwrap().into_iter().map(|r| r.item.id).collect();
        assert_eq!(ids, [a, b, c, moved]);
    }
}
