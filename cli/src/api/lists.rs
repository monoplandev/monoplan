//! Lists: the rows of the workspace's lists, and their edits.
//!
//! A list leaves the active workspace by archive (kept intact,
//! restorable) or delete (the row goes and its items move to Inbox,
//! `spec/inbox-plan.md`).

use std::collections::HashMap;

use monoplan_core::{DeleteListMode, Doc, LIST_EVENTS, LIST_INBOX};
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

/// Rename the workspace the call runs against (`Doc::set_doc_name`):
/// doc content, so every member sees it and any member may. Home is
/// never named and a blank name is refused; the core trims. One undo
/// step, none when the name is unchanged.
pub fn rename_workspace(cx: &Ctx, name: &str) -> anyhow::Result<()> {
    let home = cx
        .workspaces
        .iter()
        .any(|(c, d)| Some(c.doc_id) == cx.home && std::ptr::eq(*d, cx.doc));
    anyhow::ensure!(!home, "the Personal workspace cannot be renamed");
    anyhow::ensure!(!name.trim().is_empty(), "a workspace needs a name");
    cx.doc.set_doc_name(Some(name))?;
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

/// What deleting a list does with its items. Both move every item to
/// Inbox.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, serde::Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
pub enum DeleteMode {
    /// Items keep their lifecycle: open ones land open in Inbox.
    ToInbox,
    /// Items not already binned are binned too.
    ToBin,
}

/// Delete a user-created list (`spec/inbox-plan.md`): its row goes and
/// every item it held (open, done and binned) moves to Inbox, at the end
/// of the queue; [`DeleteMode::ToBin`] also bins them. One commit, so
/// one undo restores the list and its items. The reserved lists are
/// refused.
pub fn delete_list(cx: &Ctx, list: &str, mode: DeleteMode) -> anyhow::Result<()> {
    let mode = match mode {
        DeleteMode::ToInbox => DeleteListMode::ToInbox,
        DeleteMode::ToBin => DeleteListMode::ToBin,
    };
    cx.doc.delete_list(list, mode)?;
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::focus::Lens;
    use crate::workspace::Candidate;
    use monoplan_core::DocId;

    #[test]
    fn rename_workspace_names_a_shared_doc_and_refuses_home_and_blank() {
        let home = Doc::new().unwrap();
        let work = Doc::new().unwrap();
        work.set_doc_name(Some("Work")).unwrap();
        let home_id = DocId(uuid::Uuid::from_bytes([1; 16]));
        let work_id = DocId(uuid::Uuid::from_bytes([2; 16]));
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
        let on = |doc| Ctx {
            doc,
            open_docs: &docs,
            lens: &lens,
            workspaces: &workspaces,
            home: Some(home_id),
        };

        rename_workspace(&on(&work), "  Team  ").unwrap();
        assert_eq!(work.get_settings().doc_name.as_deref(), Some("Team"));

        assert!(rename_workspace(&on(&work), "   ").is_err());
        assert_eq!(work.get_settings().doc_name.as_deref(), Some("Team"));

        assert!(rename_workspace(&on(&home), "Mine").is_err());
        assert_eq!(home.get_settings().doc_name, None);
    }
}
