//! Places: the account's known places, and the items that carry one.
//! Both span every open doc (`spec/place-plan.md` "Reuse").

use monoplan_core::{ItemView, place_matches, place_suggestions};
use serde::Serialize;

use super::{
    Ctx, doc_tag,
    items::{Item, Place},
    lists::ListNames,
};
use crate::workspace::Candidate;

/// A place already on some item of the account, with how often and
/// how recently it is used; the `places --json` row.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct PlaceSuggestion {
    #[serde(flatten)]
    pub place: Place,
    /// Items (not binned) carrying this place.
    pub count: u32,
    /// Unix millis of the newest use.
    pub last_used: i64,
}

/// The known places matching `query` (label or address; all when
/// empty), deduped, most recently used first; at most `limit`, 0 being
/// no limit. What a place field offers before any geocoder is asked.
pub fn places(cx: &Ctx, query: &str, limit: usize) -> anyhow::Result<Vec<PlaceSuggestion>> {
    Ok(
        place_suggestions(cx.open_docs.iter().copied(), query, limit)
            .iter()
            .map(|s| PlaceSuggestion {
                place: Place::from(&s.place),
                count: s.count as u32,
                last_used: s.last_used,
            })
            .collect(),
    )
}

/// An item with where it is; the `places --items --json` row: the
/// `item ls --json` row plus the fields below.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct LocatedItem {
    #[serde(flatten)]
    pub item: Item,
    /// The item's list's name there; absent when the list is gone.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub list_name: Option<String>,
    /// The row's workspace, when the caller knows its workspaces.
    /// `doc_name` is absent for Home and an unnamed doc.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub doc_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub doc_name: Option<String>,
}

/// One row of [`placed_item_views`]: the workspace, the item, and its
/// list's name.
pub type PlacedView<'a> = (Option<&'a Candidate>, ItemView, Option<String>);

/// The Open items (not closed, not binned) that carry a place, over
/// every open doc, each doc's in its own item order; `query` keeps
/// those whose place matches, as for the places themselves, and
/// `limit` caps the rows (0 is no limit). A place without coordinates
/// counts: what can be pinned is the reader's question. The core
/// views, for a caller that renders more than the row carries;
/// [`placed_items`] is the rows.
pub fn placed_item_views<'a>(cx: &Ctx<'a>, query: &str, limit: usize) -> Vec<PlacedView<'a>> {
    let mut rows = Vec::new();
    for (tag, doc) in cx.sources() {
        let names = ListNames::of(doc);
        for item in doc.all_items() {
            let placed = item.place.as_ref().is_some_and(|p| place_matches(p, query));
            if !item.is_open() || !placed {
                continue;
            }
            let list_name = names.get(&item.list_id);
            rows.push((tag, item, list_name));
        }
    }
    if limit > 0 {
        rows.truncate(limit);
    }
    rows
}

/// [`placed_item_views`] as rows. What the iOS map reads.
pub fn placed_items(cx: &Ctx, query: &str, limit: usize) -> anyhow::Result<Vec<LocatedItem>> {
    Ok(placed_item_views(cx, query, limit)
        .iter()
        .map(|(tag, item, list_name)| {
            let (doc_id, doc_name) = doc_tag(*tag);
            LocatedItem {
                item: Item::from(item),
                list_name: list_name.clone(),
                doc_id,
                doc_name,
            }
        })
        .collect())
}
