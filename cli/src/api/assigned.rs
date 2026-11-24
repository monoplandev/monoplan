//! Assigned: the open items assigned to this account, in every open
//! workspace (`spec/sharing-plan.md` "Assignment"). The web's Assigned
//! lens (`DocsLayer.assignedItems`), with its order.

use std::cmp::Ordering;

use serde::Serialize;

use super::{Ctx, doc_tag, items::Item};

/// One row of the Assigned read: the item and its workspace.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct AssignedEntry {
    #[serde(flatten)]
    pub item: Item,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub doc_id: Option<String>,
}

/// The item's `when`, when it has one. `YYYY-MM-DD…`, so it orders as
/// text.
fn date(item: &Item) -> Option<&str> {
    item.when.as_deref()
}

/// Every open workspace's Open items (any open workflow state, not
/// binned) whose assignee is the doc's own account, tasks and events
/// alike, each tagged with its workspace. Dated items first by their
/// `when`, then the rest newest first. A doc with no account
/// (an accountless profile) has none.
pub fn assigned(cx: &Ctx) -> anyhow::Result<Vec<AssignedEntry>> {
    let mut rows: Vec<AssignedEntry> = cx
        .sources()
        .into_iter()
        .flat_map(|(tag, doc)| {
            let doc_id = doc_tag(tag).0;
            let Some(me) = doc.author().map(str::to_owned) else {
                return Vec::new();
            };
            doc.all_items()
                .iter()
                .filter(|i| i.is_open() && i.assignee.as_deref() == Some(me.as_str()))
                .map(|i| AssignedEntry {
                    item: Item::from(i),
                    doc_id: doc_id.clone(),
                })
                .collect::<Vec<_>>()
        })
        .collect();
    rows.sort_by(|a, b| {
        let dated = match (date(&a.item), date(&b.item)) {
            (Some(x), Some(y)) => x.cmp(y),
            (Some(_), None) => Ordering::Less,
            (None, Some(_)) => Ordering::Greater,
            (None, None) => Ordering::Equal,
        };
        dated
            .then(b.item.created_at.cmp(&a.item.created_at))
            .then_with(|| (&a.doc_id, &a.item.id).cmp(&(&b.doc_id, &b.item.id)))
    });
    Ok(rows)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::focus::Lens;
    use monoplan_core::{Doc, LIST_INBOX};

    #[test]
    fn spans_docs_keeps_ones_own_open_items_and_puts_dated_first() {
        let mut home = Doc::new().unwrap();
        let mut work = Doc::new().unwrap();
        home.set_author(Some("me"));
        work.set_author(Some("me"));
        let add = |doc: &Doc, text: &str, who: Option<&str>| {
            let id = doc.add_item(LIST_INBOX, text).unwrap();
            doc.set_item_assignee(&id, who).unwrap();
            std::thread::sleep(std::time::Duration::from_millis(2));
            id
        };
        let older = add(&work, "older", Some("me"));
        let newer = add(&home, "newer", Some("me"));
        let dated = add(&work, "dated", Some("me"));
        let sooner = add(&work, "sooner", Some("me"));
        add(&work, "theirs", Some("them"));
        add(&work, "nobody's", None);
        let closed = add(&work, "closed", Some("me"));
        let binned = add(&work, "binned", Some("me"));
        work.set_item_when(&dated, Some("2026-12-05")).unwrap();
        work.set_item_when(&sooner, Some("2026-12-01T09:00"))
            .unwrap();
        work.set_item_done(&closed, true).unwrap();
        work.set_item_binned(&binned, true).unwrap();

        let lens = Lens::home(&home);
        let docs = [&home, &work];
        let cx = Ctx::bare(&home, &docs, &lens);
        let ids: Vec<String> = assigned(&cx)
            .unwrap()
            .into_iter()
            .map(|r| r.item.id)
            .collect();
        assert_eq!(ids, [sooner, dated, newer, older]);

        // No account, no assignments.
        let anon = Doc::new().unwrap();
        add(&anon, "whose?", Some("me"));
        let lens = Lens::home(&anon);
        let docs = [&anon];
        let cx = Ctx::bare(&anon, &docs, &lens);
        assert!(assigned(&cx).unwrap().is_empty());
    }
}
