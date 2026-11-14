//! The bin: what is in it, and the two hard deletes. Binning and
//! restoring an item are `items::set_binned`.

use serde::Serialize;

use super::{Ctx, doc_tag};

/// One binned item; the `bin show --json` row.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct BinEntry {
    pub id: String,
    pub text: String,
    pub list_id: String,
    /// Unix millis the item was binned.
    pub binned_at: i64,
    /// The row's workspace, on the read that spans them
    /// ([`binned_everywhere`]); absent on one workspace's own.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub doc_id: Option<String>,
}

fn entries(doc: &monoplan_core::Doc, doc_id: Option<String>) -> Vec<BinEntry> {
    doc.binned_items()
        .into_iter()
        .map(|i| BinEntry {
            binned_at: i.binned_at.unwrap_or_default(),
            id: i.id,
            text: i.text,
            list_id: i.list_id,
            doc_id: doc_id.clone(),
        })
        .collect()
}

/// The workspace's binned items, in no particular order.
pub fn binned(cx: &Ctx) -> anyhow::Result<Vec<BinEntry>> {
    Ok(entries(cx.doc, None))
}

/// Every open workspace's binned items, newest binned first, each
/// tagged with its workspace.
pub fn binned_everywhere(cx: &Ctx) -> anyhow::Result<Vec<BinEntry>> {
    let mut rows: Vec<BinEntry> = cx
        .sources()
        .into_iter()
        .flat_map(|(tag, doc)| entries(doc, doc_tag(tag).0))
        .collect();
    rows.sort_by(|a, b| (b.binned_at, &b.doc_id, &b.id).cmp(&(a.binned_at, &a.doc_id, &a.id)));
    Ok(rows)
}

/// Hard-delete every binned item of the workspace; how many went.
pub fn empty_bin(cx: &Ctx) -> anyhow::Result<u32> {
    Ok(cx.doc.empty_bin()? as u32)
}

/// Hard-delete one binned item. One that is not in the bin is refused.
pub fn delete_binned(cx: &Ctx, id: &str) -> anyhow::Result<()> {
    cx.doc.delete_binned(id)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::focus::Lens;
    use crate::workspace::Candidate;
    use monoplan_core::{Doc, DocId, LIST_INBOX};

    #[test]
    fn the_spanning_read_tags_rows_and_puts_the_newest_first() {
        let home = Doc::new().unwrap();
        let work = Doc::new().unwrap();
        let a = home.add_item(LIST_INBOX, "a").unwrap();
        home.add_item(LIST_INBOX, "kept").unwrap();
        home.set_item_binned(&a, true).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(2));
        let b = work.add_item(LIST_INBOX, "b").unwrap();
        work.set_item_binned(&b, true).unwrap();

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
        let lens = Lens::home(&home);
        let cx = Ctx {
            doc: &home,
            open_docs: &[&home, &work],
            lens: &lens,
            workspaces: &tags,
            home: Some(tags[0].0.doc_id),
        };
        let rows = binned_everywhere(&cx).unwrap();
        assert_eq!(
            rows.iter().map(|r| r.id.as_str()).collect::<Vec<_>>(),
            [b.as_str(), a.as_str()]
        );
        assert_eq!(rows[0].doc_id, Some(tags[1].0.doc_id.0.to_string()));
        assert_eq!(rows[1].doc_id, Some(tags[0].0.doc_id.0.to_string()));

        // One workspace's own read carries no tag, and its JSON none.
        let own = binned(&cx).unwrap();
        assert_eq!(own.len(), 1);
        let json = serde_json::to_value(&own[0]).unwrap();
        assert!(json.get("doc_id").is_none());
        assert!(json["binned_at"].is_i64());

        delete_binned(&cx, &a).unwrap();
        assert!(delete_binned(&cx, "nope").is_err());
        assert_eq!(empty_bin(&cx).unwrap(), 0);
    }
}
