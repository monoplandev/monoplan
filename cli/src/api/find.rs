//! Find: the search index (`spec/search.md`) over every open doc.
//!
//! The engine is the core's (`Doc::search`: AND across tokens, last
//! token a prefix, its ranking). This is the surface's half, the same
//! policy as the web's Find (`js/web/src/findResults.tsx`): fan out
//! over every open doc, merge by score (stable, so ties keep doc order,
//! Home first), float lists above items, and tag each row with its
//! workspace, its list's name and whether that list is archived. An
//! empty query finds nothing. The built-in views (Starred, Agenda,
//! Done, Home's Inbox) are not doc entities; a client matches their
//! labels itself with the core's `matches_name`.

use std::collections::HashMap;

use monoplan_core::{ItemLifecycle, LIST_EVENTS, LIST_INBOX, ListView, SearchKind};
use serde::Serialize;

use super::{Ctx, doc_tag};

/// What a hit is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
pub enum FindKind {
    Item,
    List,
}

/// An item hit's lifecycle: the workflow states, and `Binned`, which
/// masks whichever of them the item holds underneath. Serialises as
/// the canonical name (`ItemLifecycle::name`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
pub enum HitState {
    Backlog,
    Todo,
    InProgress,
    Review,
    Done,
    Cancelled,
    Binned,
}

impl From<ItemLifecycle> for HitState {
    fn from(l: ItemLifecycle) -> Self {
        match l {
            ItemLifecycle::Backlog => HitState::Backlog,
            ItemLifecycle::Todo => HitState::Todo,
            ItemLifecycle::InProgress => HitState::InProgress,
            ItemLifecycle::Review => HitState::Review,
            ItemLifecycle::Done => HitState::Done,
            ItemLifecycle::Cancelled => HitState::Cancelled,
            ItemLifecycle::Binned => HitState::Binned,
        }
    }
}

/// One ranked hit; the `find --json` row.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct FindHit {
    pub kind: FindKind,
    /// The item's or the list's id; unique only within its doc.
    pub id: String,
    pub title: String,
    /// The row's workspace (`doc_name` absent for Home and an unnamed
    /// doc). Absent only where the caller has bare docs.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub doc_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub doc_name: Option<String>,
    /// Items: the list the item is in, and its name there (absent when
    /// the list is gone).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub list_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub list_name: Option<String>,
    /// Items: the lifecycle.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub state: Option<HitState>,
    /// Lists: the list's emoji, when it has one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    /// The list (or the item's list) is archived.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub archived: bool,
    /// Flat relevance score, for display and debugging.
    pub score: u64,
}

/// The merged, ranked hits for `query` over every open doc, at most
/// `limit`.
pub fn find(cx: &Ctx, query: &str, limit: usize) -> anyhow::Result<Vec<FindHit>> {
    let mut out: Vec<FindHit> = Vec::new();
    for (tag, doc) in cx.sources() {
        let found = doc.search(query, limit);
        if found.is_empty() {
            continue;
        }
        // Archived lists included: they stay indexed and keep naming
        // their items. The reserved lists have no `ListMeta` row, so
        // they are named here as the clients label them.
        let lists: HashMap<String, ListView> = doc
            .all_lists()
            .into_iter()
            .map(|l| (l.id.clone(), l))
            .collect();
        let (doc_id, doc_name) = doc_tag(tag);
        for r in found {
            let list = match r.kind {
                SearchKind::List => lists.get(&r.id),
                SearchKind::Item => r.list_id.as_ref().and_then(|id| lists.get(id)),
            };
            let list_name = match r.list_id.as_deref() {
                None => None,
                Some(LIST_INBOX) => Some("Inbox".to_string()),
                Some(LIST_EVENTS) => Some("Events".to_string()),
                Some(_) => list.map(|l| l.name.clone()),
            };
            out.push(FindHit {
                kind: match r.kind {
                    SearchKind::Item => FindKind::Item,
                    SearchKind::List => FindKind::List,
                },
                id: r.id,
                title: r.title,
                doc_id: doc_id.clone(),
                doc_name: doc_name.clone(),
                list_id: r.list_id,
                list_name,
                state: r.lifecycle.map(HitState::from),
                icon: match r.kind {
                    SearchKind::List => list.and_then(|l| l.icon.clone()),
                    SearchKind::Item => None,
                },
                archived: list.is_some_and(|l| l.archived_at.is_some()),
                score: r.score,
            });
        }
    }
    // Lists above items, each group by relevance. Stable, so ties
    // keep doc order.
    out.sort_by_key(|h| {
        let group = match h.kind {
            FindKind::List => 0,
            FindKind::Item => 1,
        };
        (group, std::cmp::Reverse(h.score))
    });
    out.truncate(limit);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hit_state_serialises_as_the_lifecycle_name() {
        for l in [
            ItemLifecycle::Backlog,
            ItemLifecycle::Todo,
            ItemLifecycle::InProgress,
            ItemLifecycle::Review,
            ItemLifecycle::Done,
            ItemLifecycle::Cancelled,
            ItemLifecycle::Binned,
        ] {
            assert_eq!(
                serde_json::to_value(HitState::from(l)).unwrap(),
                serde_json::Value::String(l.name().into())
            );
        }
    }
}
