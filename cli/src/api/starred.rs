//! Starred: the curated Focus lens (`spec/focus.md`; "Starred" is the
//! UI's name for it). Starring and unstarring an item are
//! `items::star` / `items::unstar`.

use monoplan_core::ItemView;
use serde::Serialize;

use super::{
    Ctx, doc_tag,
    items::{ItemState, Place},
};
use crate::commands::focus::resolve_all;
use crate::workspace::Candidate;

/// One starred item; the `starred --json` row.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct StarredItem {
    /// 1-based position among the rows returned.
    pub pos: u32,
    pub id: String,
    pub text: String,
    pub list_id: String,
    pub state: ItemState,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub place: Option<Place>,
    /// The full lens only: the row's workspace (`doc_name` absent for
    /// Home and an unnamed doc) and its list's name there.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub doc_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub doc_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub list_name: Option<String>,
}

fn row(
    pos: usize,
    item: &ItemView,
    tag: Option<&Candidate>,
    list_name: Option<String>,
) -> StarredItem {
    let (doc_id, doc_name) = doc_tag(tag);
    StarredItem {
        pos: pos as u32,
        id: item.id.clone(),
        text: item.text.clone(),
        list_id: item.list_id.clone(),
        state: item.state.into(),
        place: item.place.as_ref().map(Place::from),
        doc_id,
        doc_name,
        list_name,
    }
}

/// The workspace's own lens: its Open referenced items in curated
/// order. Entries that point into another workspace are not resolved
/// here; [`starred_everywhere`] is the full lens.
pub fn starred(cx: &Ctx) -> anyhow::Result<Vec<StarredItem>> {
    Ok(cx
        .doc
        .focus_view()
        .iter()
        .enumerate()
        .map(|(i, item)| row(i + 1, item, None, None))
        .collect())
}

/// The full lens: Home's entries in curated order with each
/// cross-workspace entry resolved against the doc it names
/// (`commands::focus::resolve_all`), every row tagged with its
/// workspace and list. Entries whose doc is not open or whose item is
/// missing or not Open are left out. Reads only; pruning dead refs
/// stays with the clients that observe the close.
pub fn starred_everywhere(cx: &Ctx) -> anyhow::Result<Vec<StarredItem>> {
    let home = cx
        .home
        .ok_or_else(|| anyhow::anyhow!("the Home workspace is not open"))?;
    Ok(resolve_all(cx.workspaces, home)?
        .iter()
        .enumerate()
        .map(|(i, r)| row(i + 1, &r.item, Some(r.doc), r.list_name.clone()))
        .collect())
}

/// Reorder a starred item: to directly under `after`, another starred
/// item, or else to the 1-based `pos` (the top when neither is given).
///
/// A position counts every live entry, the ones a client cannot show
/// included (a ref into a workspace that is not open, or to an item
/// that has closed there), so a client that shows the resolved lens
/// names a neighbour with `after` instead: the item lands directly
/// under it, whatever hidden entries sit around them.
pub fn move_starred(
    cx: &Ctx,
    id: &str,
    pos: Option<usize>,
    after: Option<&str>,
) -> anyhow::Result<()> {
    let index = match after {
        Some(anchor) => {
            let entries = cx.doc.focus_entries();
            let at = |id: &str| entries.iter().position(|e| e.item_id == id);
            let Some(anchor_at) = at(anchor) else {
                anyhow::bail!("not starred: {anchor}");
            };
            match at(id) {
                // The slot the item ends up in: taking it out from
                // above the anchor moves the anchor up one.
                Some(from) if from < anchor_at => anchor_at,
                _ => anchor_at + 1,
            }
        }
        None => pos.unwrap_or(1).saturating_sub(1),
    };
    cx.doc.move_in_focus(id, index)?;
    Ok(())
}
