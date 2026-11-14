//! Lists: the rows of the workspace's lists.

use std::collections::HashMap;

use serde::Serialize;

use super::Ctx;

/// Which lists [`lists`] returns.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
pub enum ListFilter {
    Active,
    Archived,
    All,
}

/// One list; the `list ls --json` row.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct ListInfo {
    pub id: String,
    pub name: String,
    /// The list's emoji, when the user picked one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    pub archived_at: Option<i64>,
    pub created_at: i64,
    /// How many Open items the list holds (not closed, not binned).
    pub open: u32,
}

/// The workspace's lists in their order, each with its open count
/// (one walk of the doc's items for all of them).
pub fn lists(cx: &Ctx, filter: ListFilter) -> anyhow::Result<Vec<ListInfo>> {
    let mut open = HashMap::<String, u32>::new();
    for item in cx.doc.all_items() {
        if item.is_open() {
            *open.entry(item.list_id).or_insert(0) += 1;
        }
    }
    Ok(cx
        .doc
        .all_lists()
        .into_iter()
        .filter(|l| match filter {
            ListFilter::Active => l.archived_at.is_none(),
            ListFilter::Archived => l.archived_at.is_some(),
            ListFilter::All => true,
        })
        .map(|l| ListInfo {
            open: open.get(&l.id).copied().unwrap_or(0),
            id: l.id,
            name: l.name,
            icon: l.icon,
            archived_at: l.archived_at,
            created_at: l.created_at,
        })
        .collect())
}
