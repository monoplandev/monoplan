//! Done: what got finished in the workspace, newest first
//! (`commands::done` for the selection rule and the terminal's days).

use serde::Serialize;

use super::{Ctx, doc_tag, items::Item};
use crate::commands::done::select_done;

/// Which workspaces and lists the spanning Done read draws from: the
/// web Done view's filter (`js/web/src/doneFilter.ts`). Two facets,
/// ANDed; an empty facet filters nothing. Applied as given: narrowing
/// a stored choice to the docs that are open and the lists that exist
/// is the caller's, which also shows it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct DoneFilter {
    /// Workspaces, by doc id.
    pub docs: Vec<String>,
    /// Lists, each as `<doc id>/<list id>`: `inbox` exists in every
    /// doc, so a list is named by doc and id together.
    pub lists: Vec<String>,
}

/// One row of the spanning Done read: the item and its workspace.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct DoneEntry {
    #[serde(flatten)]
    pub item: Item,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub doc_id: Option<String>,
}

/// The workspace's Done, not-binned items across every list, newest
/// first by the moment each entered Done; `since` (unix millis) and
/// `limit` bound them, `None` being unbounded. A full `limit` back
/// means there may be more.
pub fn done(cx: &Ctx, since: Option<i64>, limit: Option<usize>) -> anyhow::Result<Vec<Item>> {
    let items = cx.doc.all_items();
    Ok(select_done(&items, since, limit)
        .into_iter()
        .map(Item::from)
        .collect())
}

/// How many items [`done`] would return with no `limit`.
pub fn done_count(cx: &Ctx, since: Option<i64>) -> anyhow::Result<u32> {
    let items = cx.doc.all_items();
    Ok(select_done(&items, since, None).len() as u32)
}

/// Every open workspace's Done, not-binned items that `filter` admits,
/// newest first, each tagged with its workspace.
fn admitted(cx: &Ctx, filter: &DoneFilter) -> Vec<DoneEntry> {
    let mut rows: Vec<DoneEntry> = cx
        .sources()
        .into_iter()
        .flat_map(|(tag, doc)| {
            let doc_id = doc_tag(tag).0;
            let key = doc_id.clone().unwrap_or_default();
            let items = if filter.docs.is_empty() || filter.docs.contains(&key) {
                doc.all_items()
            } else {
                Vec::new()
            };
            select_done(&items, None, None)
                .into_iter()
                .filter(|i| {
                    filter.lists.is_empty()
                        || filter.lists.contains(&format!("{key}/{}", i.list_id))
                })
                .map(|i| DoneEntry {
                    item: Item::from(i),
                    doc_id: doc_id.clone(),
                })
                .collect::<Vec<_>>()
        })
        .collect();
    rows.sort_by(|a, b| {
        (b.item.lifecycle_at, &a.doc_id, &a.item.id).cmp(&(
            a.item.lifecycle_at,
            &b.doc_id,
            &b.item.id,
        ))
    });
    rows
}

/// [`done`] across every open workspace, narrowed by `filter`: the
/// newest `limit` rows, `None` being unbounded.
pub fn done_everywhere(
    cx: &Ctx,
    filter: &DoneFilter,
    limit: Option<usize>,
) -> anyhow::Result<Vec<DoneEntry>> {
    let mut rows = admitted(cx, filter);
    if let Some(n) = limit {
        rows.truncate(n);
    }
    Ok(rows)
}

/// How many rows [`done_everywhere`] would return with no `limit`.
pub fn done_count_everywhere(cx: &Ctx, filter: &DoneFilter) -> anyhow::Result<u32> {
    Ok(admitted(cx, filter).len() as u32)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::focus::Lens;
    use crate::workspace::Candidate;
    use monoplan_core::{Doc, DocId, LIST_INBOX};

    #[test]
    fn the_spanning_read_filters_by_workspace_and_list() {
        let home = Doc::new().unwrap();
        let work = Doc::new().unwrap();
        let chores = home.add_list("Chores").unwrap();
        let tick = |doc: &Doc, list: &str, text: &str| {
            let id = doc.add_item(list, text).unwrap();
            doc.set_item_done(&id, true).unwrap();
            std::thread::sleep(std::time::Duration::from_millis(2));
            id
        };
        let a = tick(&home, LIST_INBOX, "a");
        let b = tick(&work, LIST_INBOX, "b");
        let c = tick(&home, &chores, "c");
        home.add_item(LIST_INBOX, "open").unwrap();

        let tags = [
            (
                Candidate {
                    doc_id: DocId(uuid::Uuid::from_u128(1)),
                    name: None,
                },
                &home,
            ),
            (
                Candidate {
                    doc_id: DocId(uuid::Uuid::from_u128(2)),
                    name: Some("Work".into()),
                },
                &work,
            ),
        ];
        let (home_id, work_id) = (
            tags[0].0.doc_id.0.to_string(),
            tags[1].0.doc_id.0.to_string(),
        );
        let lens = Lens::home(&home);
        let cx = Ctx {
            doc: &home,
            open_docs: &[&home, &work],
            lens: &lens,
            workspaces: &tags,
            home: Some(tags[0].0.doc_id),
        };
        let ids = |filter: &DoneFilter, limit| {
            done_everywhere(&cx, filter, limit)
                .unwrap()
                .into_iter()
                .map(|r| r.item.id)
                .collect::<Vec<_>>()
        };

        // Unfiltered: every workspace, newest first, tagged.
        let all = done_everywhere(&cx, &DoneFilter::default(), None).unwrap();
        assert_eq!(
            all.iter().map(|r| r.item.id.clone()).collect::<Vec<_>>(),
            [c.clone(), b.clone(), a.clone()]
        );
        assert_eq!(all[1].doc_id, Some(work_id.clone()));
        assert_eq!(ids(&DoneFilter::default(), Some(2)), [c.clone(), b.clone()]);

        let docs = DoneFilter {
            docs: vec![home_id.clone()],
            lists: vec![],
        };
        assert_eq!(ids(&docs, None), [c.clone(), a.clone()]);
        assert_eq!(done_count_everywhere(&cx, &docs).unwrap(), 2);

        // A list is its doc's: Work's inbox is not Home's.
        let lists = DoneFilter {
            docs: vec![],
            lists: vec![
                format!("{work_id}/{LIST_INBOX}"),
                format!("{home_id}/{chores}"),
            ],
        };
        assert_eq!(ids(&lists, None), [c.clone(), b.clone()]);

        // The facets are ANDed.
        let both = DoneFilter {
            docs: vec![home_id.clone()],
            lists: lists.lists.clone(),
        };
        assert_eq!(ids(&both, None), [c]);
        assert_eq!(done_count_everywhere(&cx, &both).unwrap(), 1);
    }
}
