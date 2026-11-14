//! Lists: the rows of the workspace's lists, and their edits.
//!
//! There is no delete (`spec/cli.md`): archive is the only way a list
//! leaves the active workspace.

use std::collections::HashMap;

use monoplan_core::{Doc, LIST_EVENTS, LIST_INBOX};
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

/// Add a list at the end of the workspace's lists and return its id.
/// The core trims the name and refuses an empty one.
pub fn add_list(cx: &Ctx, name: &str) -> anyhow::Result<String> {
    Ok(cx.doc.add_list(name)?)
}

/// Rename a user-created list; the reserved ones are refused.
pub fn rename_list(cx: &Ctx, list: &str, name: &str) -> anyhow::Result<()> {
    cx.doc.rename_list(list, name)?;
    Ok(())
}

/// Set a user-created list's emoji, or clear it (`None`).
pub fn set_list_icon(cx: &Ctx, list: &str, icon: Option<&str>) -> anyhow::Result<()> {
    cx.doc.set_list_icon(list, icon.unwrap_or(""))?;
    Ok(())
}

/// Archive a user-created list (it leaves the active workspace, its
/// items, ordering and metadata kept), or restore it.
pub fn set_list_archived(cx: &Ctx, list: &str, archived: bool) -> anyhow::Result<()> {
    cx.doc.set_list_archived(list, archived)?;
    Ok(())
}

/// A doc's list names by id, for the rows that name the list their
/// item is in.
pub(crate) struct ListNames(HashMap<String, String>);

impl ListNames {
    pub(crate) fn of(doc: &Doc) -> Self {
        ListNames(
            doc.all_lists()
                .into_iter()
                .map(|l| (l.id, l.name))
                .collect(),
        )
    }

    /// The name of `list_id`; `None` when the list is gone. The
    /// reserved lists have no `ListMeta` row (`all_lists` skips them),
    /// so they are named here as the clients label them.
    pub(crate) fn get(&self, list_id: &str) -> Option<String> {
        match list_id {
            LIST_INBOX => Some("Inbox".to_string()),
            LIST_EVENTS => Some("Events".to_string()),
            other => self.0.get(other).cloned(),
        }
    }
}
