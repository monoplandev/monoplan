//! Items: the rows of a list, adding, ticking.

use monoplan_core::{ItemLifecycle, ItemView, WorkflowState};
use serde::Serialize;

use super::Ctx;

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
        ItemLifecycle::Done
    } else {
        ItemLifecycle::Backlog
    };
    cx.doc.set_item_lifecycle(id, state)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::focus::Lens;
    use monoplan_core::{Doc, LIST_INBOX};

    /// The `--json` row is pinned here, field for field: what every
    /// script and (until `Engine.request` goes) both apps decode.
    #[test]
    fn item_serialises_as_the_ls_json_row() {
        let doc = Doc::new().unwrap();
        let lens = Lens::home(&doc);
        let cx = Ctx {
            doc: &doc,
            open_docs: &[&doc],
            lens: &lens,
        };
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
