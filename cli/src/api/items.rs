//! Items: the rows of a list, one item's page, adding, and every
//! per-item write. Each setter is its own core setter and its own
//! commit.

use monoplan_core::{ItemLifecycle, ItemView, WorkflowState, place_suggestions};
use serde::Serialize;

use super::Ctx;
use crate::commands::items::known_place;

/// The workflow register's state (`spec/data-model.md` "Lifecycle").
/// Serialises as the canonical name (`WorkflowState::name`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
pub enum ItemState {
    Backlog,
    Todo,
    InProgress,
    Review,
    Done,
    Cancelled,
}

impl From<WorkflowState> for ItemState {
    fn from(s: WorkflowState) -> Self {
        match s {
            WorkflowState::Backlog => ItemState::Backlog,
            WorkflowState::Todo => ItemState::Todo,
            WorkflowState::InProgress => ItemState::InProgress,
            WorkflowState::Review => ItemState::Review,
            WorkflowState::Done => ItemState::Done,
            WorkflowState::Cancelled => ItemState::Cancelled,
        }
    }
}

impl From<ItemState> for WorkflowState {
    fn from(s: ItemState) -> Self {
        match s {
            ItemState::Backlog => WorkflowState::Backlog,
            ItemState::Todo => WorkflowState::Todo,
            ItemState::InProgress => WorkflowState::InProgress,
            ItemState::Review => WorkflowState::Review,
            ItemState::Done => WorkflowState::Done,
            ItemState::Cancelled => WorkflowState::Cancelled,
        }
    }
}

impl From<ItemState> for ItemLifecycle {
    fn from(s: ItemState) -> Self {
        match s {
            ItemState::Backlog => ItemLifecycle::Backlog,
            ItemState::Todo => ItemLifecycle::Todo,
            ItemState::InProgress => ItemLifecycle::InProgress,
            ItemState::Review => ItemLifecycle::Review,
            ItemState::Done => ItemLifecycle::Done,
            ItemState::Cancelled => ItemLifecycle::Cancelled,
        }
    }
}

/// An item's place (`monoplan_core::Place`, `spec/place-plan.md`),
/// copied at this boundary so the core stays free of uniffi. Serialises
/// exactly as the core type does.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct Place {
    pub label: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lat: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lon: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub address: Option<String>,
    /// Opaque provider hint (`osm:node/123`).
    #[serde(skip_serializing_if = "Option::is_none", rename = "ref")]
    pub reference: Option<String>,
}

impl From<&monoplan_core::Place> for Place {
    fn from(p: &monoplan_core::Place) -> Self {
        Place {
            label: p.label.clone(),
            lat: p.lat,
            lon: p.lon,
            address: p.address.clone(),
            reference: p.reference.clone(),
        }
    }
}

impl From<Place> for monoplan_core::Place {
    fn from(p: Place) -> Self {
        monoplan_core::Place {
            label: p.label,
            lat: p.lat,
            lon: p.lon,
            address: p.address,
            reference: p.reference,
        }
    }
}

/// One item as a list shows it; the `item ls --json` row.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct Item {
    pub id: String,
    pub text: String,
    pub list_id: String,
    /// The workflow register's state, whatever the bin mask says:
    /// `binned_at` tells a binned item.
    pub state: ItemState,
    /// Unix millis the register's state was entered.
    pub lifecycle_at: i64,
    pub created_at: i64,
    /// Creator's account id, when the creating client stamped one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub created_by: Option<String>,
    pub started_at: Option<i64>,
    pub done_at: Option<i64>,
    pub binned_at: Option<i64>,
    /// Date-only deadline (`YYYY-MM-DD`), when set.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub deadline: Option<String>,
    /// Planned date (`YYYY-MM-DD` or `YYYY-MM-DDTHH:MM`), when set.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub when: Option<String>,
    /// Pending date suggestion (`when`-shaped), when one is unanswered.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub when_suggested: Option<String>,
    /// Duration in whole minutes, when set.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration: Option<u32>,
    /// Place (`{label, lat?, lon?, address?, ref?}`), when set.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub place: Option<Place>,
}

impl From<&ItemView> for Item {
    fn from(item: &ItemView) -> Self {
        Item {
            id: item.id.clone(),
            text: item.text.clone(),
            list_id: item.list_id.clone(),
            state: item.state.into(),
            lifecycle_at: item.lifecycle_at,
            created_at: item.created_at,
            created_by: item.created_by.clone(),
            started_at: item.started_at,
            done_at: item.done_at,
            binned_at: item.binned_at,
            deadline: item.deadline.clone(),
            when: item.when.clone(),
            when_suggested: item.when_suggested.clone(),
            duration: item.duration,
            place: item.place.as_ref().map(Place::from),
        }
    }
}

/// The items of `list` in order, binned ones left out; closed ones
/// (`Done`, `Cancelled`) only with `include_done`. The core views, for
/// a caller that renders more than the row carries; [`items`] is the
/// rows.
pub fn item_views(cx: &Ctx, list: &str, include_done: bool) -> Vec<ItemView> {
    let mut items = cx.doc.items_in_list(list, false);
    if !include_done {
        items.retain(|i| !i.is_closed());
    }
    items
}

/// [`item_views`] as rows.
pub fn items(cx: &Ctx, list: &str, include_done: bool) -> anyhow::Result<Vec<Item>> {
    Ok(item_views(cx, list, include_done)
        .iter()
        .map(Item::from)
        .collect())
}

/// Add one item per text to `list` and return their ids, in the order
/// given. `top` puts them above the list's open items (one commit);
/// otherwise each is appended.
pub fn add_items(cx: &Ctx, list: &str, texts: &[String], top: bool) -> anyhow::Result<Vec<String>> {
    if texts.is_empty() {
        anyhow::bail!("no item text provided");
    }
    if top {
        let texts: Vec<&str> = texts.iter().map(String::as_str).collect();
        return Ok(cx.doc.add_items_at(list, &texts, 0)?);
    }
    let mut ids = Vec::with_capacity(texts.len());
    for text in texts {
        ids.push(cx.doc.add_item(list, text)?);
    }
    Ok(ids)
}

/// Tick or untick: `Done`, or back to `Backlog`.
pub fn set_done(cx: &Ctx, id: &str, done: bool) -> anyhow::Result<()> {
    let state = if done {
        ItemState::Done
    } else {
        ItemState::Backlog
    };
    set_state(cx, id, state)
}

/// One item with what only its own page needs; the `item show --json`
/// row: the `item ls --json` row plus the fields below.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct ItemInfo {
    #[serde(flatten)]
    pub item: Item,
    /// Plain text; empty when the item has none.
    pub notes: String,
    /// The item's list's name; absent for the reserved lists, which
    /// have no `ListMeta` row (clients label those).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub list_name: Option<String>,
    /// That list's emoji, when it has one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub list_icon: Option<String>,
    /// The item shows in Starred: Open, and referenced by Home's lens.
    pub starred: bool,
}

/// The core view of one item, whatever its state (open, closed or
/// binned).
pub fn item_view(cx: &Ctx, id: &str) -> anyhow::Result<ItemView> {
    cx.doc
        .get_item(id)
        .ok_or_else(|| anyhow::anyhow!("item not found: {id}"))
}

/// [`item_view`] as the page's row.
pub fn item(cx: &Ctx, id: &str) -> anyhow::Result<ItemInfo> {
    Ok(item_info(cx, &item_view(cx, id)?))
}

/// The page's row for a view already in hand.
pub fn item_info(cx: &Ctx, item: &ItemView) -> ItemInfo {
    let list = cx.doc.get_list_meta(&item.list_id);
    ItemInfo {
        item: Item::from(item),
        notes: item.notes.clone(),
        list_icon: list.as_ref().and_then(|l| l.icon.clone()),
        list_name: list.map(|l| l.name),
        starred: cx.lens.has(item),
    }
}

/// Move the item to any of the six workflow states.
pub fn set_state(cx: &Ctx, id: &str, state: ItemState) -> anyhow::Result<()> {
    cx.doc.set_item_lifecycle(id, state.into())?;
    Ok(())
}

/// Retitle the item.
pub fn set_text(cx: &Ctx, id: &str, text: &str) -> anyhow::Result<()> {
    cx.doc.edit_item_text(id, text)?;
    Ok(())
}

/// Move the item to `list` (its own when `None`: a reorder), at the
/// 1-based `pos` among that list's open items, the slot it ends up in;
/// no `pos` appends, and one past the end clamps to the end. Moving
/// into `events` makes the item an event; back out, a task again.
pub fn move_item(cx: &Ctx, id: &str, list: Option<&str>, pos: Option<usize>) -> anyhow::Result<()> {
    let list = match list {
        Some(list) => list.to_string(),
        None => item_view(cx, id)?.list_id,
    };
    // `move_item` counts the index over the projection without the
    // moving item, and clamps to the existing range.
    let index = match pos {
        Some(pos) => pos.saturating_sub(1),
        None => cx.doc.items_in_list(&list, true).len(),
    };
    cx.doc.move_item(id, &list, index)?;
    Ok(())
}

/// Set the planned date (`YYYY-MM-DD`, or `YYYY-MM-DDTHH:MM` floating),
/// or clear it and the duration (`None`). The core validates.
pub fn set_when(cx: &Ctx, id: &str, when: Option<&str>) -> anyhow::Result<()> {
    cx.doc.set_item_when(id, when)?;
    Ok(())
}

/// Raise a date suggestion (`when`-shaped) on the item, or dismiss the
/// pending one (`None`). Setting `when` to the same value accepts it.
pub fn set_when_suggested(cx: &Ctx, id: &str, when: Option<&str>) -> anyhow::Result<()> {
    cx.doc.set_item_when_suggested(id, when)?;
    Ok(())
}

/// Set the duration in whole minutes, or clear it (`None`).
pub fn set_duration(cx: &Ctx, id: &str, minutes: Option<u32>) -> anyhow::Result<()> {
    cx.doc.set_item_duration(id, minutes)?;
    Ok(())
}

/// Set the deadline (`YYYY-MM-DD`), or clear it (`None`).
pub fn set_deadline(cx: &Ctx, id: &str, deadline: Option<&str>) -> anyhow::Result<()> {
    cx.doc.set_item_deadline(id, deadline)?;
    Ok(())
}

/// Set the item's place, or clear it (`None`). A place with
/// coordinates or an address is written as given. A bare label that
/// matches a place already on some item of any open workspace
/// (case-insensitively, `spec/place-plan.md` "Reuse") takes that place
/// whole, coordinates and address included, unless `exact`, which
/// writes just the label. Validation lives in the core.
pub fn set_place(cx: &Ctx, id: &str, place: Option<Place>, exact: bool) -> anyhow::Result<()> {
    let value = place.map(|place| {
        let bare = place.lat.is_none() && place.lon.is_none() && place.address.is_none();
        let known = if bare && !exact {
            known_place(
                &place_suggestions(cx.open_docs.iter().copied(), &place.label, 0),
                &place.label,
            )
        } else {
            None
        };
        known.unwrap_or_else(|| place.into())
    });
    cx.doc.set_item_place(id, value.as_ref())?;
    Ok(())
}

/// Star the item at Starred's 0-based entry `index` (`usize::MAX`
/// appends). Starred is Home's whatever workspace the item is in
/// (`Lens`). No-op when already starred or not Open.
pub fn star(cx: &Ctx, id: &str, index: usize) -> anyhow::Result<()> {
    cx.lens.star(cx.doc, id, index)
}

/// Unstar the item; the item itself is untouched.
pub fn unstar(cx: &Ctx, id: &str) -> anyhow::Result<()> {
    cx.lens.unstar(id)
}

/// Send the item to the bin, or restore it from there. Only the mask
/// changes: the workflow state under it is kept, so a done-then-binned
/// item comes back into Done and an open one into its list where it was.
pub fn set_binned(cx: &Ctx, id: &str, binned: bool) -> anyhow::Result<()> {
    cx.doc.set_item_binned(id, binned)?;
    Ok(())
}

/// Copy the item beside itself in its own list and return the copy's
/// id. With `star`, a starred original's copy is starred in the slot
/// under it; an unstarred original's copy is left unstarred.
pub fn duplicate(cx: &Ctx, id: &str, star: bool) -> anyhow::Result<String> {
    let copy = cx.doc.duplicate_item(id)?;
    if star && let Some(at) = cx.lens.position(id) {
        cx.lens.star(cx.doc, &copy, at + 1)?;
    }
    Ok(copy)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::focus::Lens;
    use monoplan_core::{Doc, LIST_INBOX};

    /// The `--json` row is pinned here, field for field: what every
    /// script decodes.
    #[test]
    fn item_serialises_as_the_ls_json_row() {
        let doc = Doc::new().unwrap();
        let lens = Lens::home(&doc);
        let docs = [&doc];
        let cx = Ctx::bare(&doc, &docs, &lens);
        let id = add_items(&cx, LIST_INBOX, &["Gym".into()], false)
            .unwrap()
            .remove(0);
        doc.set_item_when(&id, Some("2026-11-14T09:00")).unwrap();
        doc.set_item_place(
            &id,
            Some(&monoplan_core::Place {
                label: "Gym".into(),
                lat: Some(1.5),
                lon: Some(2.5),
                address: None,
                reference: Some("osm:node/1".into()),
            }),
        )
        .unwrap();
        set_done(&cx, &id, true).unwrap();

        let rows = items(&cx, LIST_INBOX, true).unwrap();
        let row = rows.iter().find(|r| r.id == id).unwrap();
        assert_eq!(row.state, ItemState::Done);
        let json = serde_json::to_value(row).unwrap();
        let keys: Vec<&str> = json
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        // Unset optionals that the old row skipped are absent; the
        // three stamps are always there, null when unset.
        for k in ["created_by", "deadline", "when_suggested"] {
            assert!(!keys.contains(&k), "{k} should be absent: {json}");
        }
        assert_eq!(json["state"], "done");
        assert_eq!(json["when"], "2026-11-14T09:00");
        assert!(json["started_at"].is_null());
        assert!(json["binned_at"].is_null());
        assert_eq!(
            json["place"],
            serde_json::json!({"label": "Gym", "lat": 1.5, "lon": 2.5, "ref": "osm:node/1"})
        );
        // Open items only without `include_done`.
        assert!(
            items(&cx, LIST_INBOX, false)
                .unwrap()
                .iter()
                .all(|r| r.id != id)
        );
    }

    #[test]
    fn in_progress_serialises_as_the_canonical_name() {
        for s in [
            WorkflowState::Backlog,
            WorkflowState::Todo,
            WorkflowState::InProgress,
            WorkflowState::Review,
            WorkflowState::Done,
            WorkflowState::Cancelled,
        ] {
            assert_eq!(
                serde_json::to_value(ItemState::from(s)).unwrap(),
                serde_json::Value::String(s.name().into())
            );
        }
    }
}
