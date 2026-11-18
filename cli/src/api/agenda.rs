//! Agenda: dated items by day, Today first (`commands::agenda` for
//! the placement, tone and ordering rules, which are pure functions
//! tested there).

use std::collections::HashMap;

use chrono::{Days, Local, NaiveDate};
use monoplan_core::{DocId, ItemView};
use serde::Serialize;

use super::{
    Ctx, doc_tag,
    items::{ItemState, Place},
};
use crate::commands::agenda::{self as rules, Source};
use crate::workspace::Candidate;

/// Row tone, least to most urgent. Warning comes from the deadline
/// alone; overdue is a past deadline, or a task's `when` that is over.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
pub enum AgendaTone {
    Neutral,
    Warning,
    Overdue,
}

/// What put the row on its day: one of its two dates, or, for a task
/// done today, that it was done (`Done`, always on Today).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
pub enum AgendaPlacedBy {
    When,
    Deadline,
    Done,
}

/// One row of a group; the `agenda --json` row.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct AgendaEntry {
    pub id: String,
    pub text: String,
    pub list_id: String,
    /// The name of the list the item is in; absent for an item with no
    /// list, which lives on the calendar alone.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub list_name: Option<String>,
    /// That list's emoji, when it has one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub list_icon: Option<String>,
    pub state: ItemState,
    /// An event (`spec/events-plan.md`): its date goes by instead of
    /// going overdue.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub event: bool,
    /// `YYYY-MM-DD` (all-day) or `YYYY-MM-DDTHH:MM` (timed, floating).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub when: Option<String>,
    /// Whole minutes.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub deadline: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub place: Option<Place>,
    /// Which field placed the row.
    pub placed_by: AgendaPlacedBy,
    pub tone: AgendaTone,
    /// The merged agenda only: the row's workspace. `doc_name` is
    /// absent for Home (never named) and for an unnamed doc.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub doc_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub doc_name: Option<String>,
}

/// One day's rows; the `agenda --json` element.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct AgendaGroup {
    /// `YYYY-MM-DD`.
    pub day: String,
    pub today: bool,
    pub rows: Vec<AgendaEntry>,
}

/// What an agenda is built from and for.
pub struct AgendaQuery<'a> {
    /// How many days past today to show (Today is always shown).
    pub days: u32,
    /// How many days before today to show: the days gone by, ahead of
    /// Today in the result. 0 for none.
    pub past_days: u32,
    /// The local date to use as today (`YYYY-MM-DD`); the clock's
    /// when `None`.
    pub today: Option<&'a str>,
    /// Every open workspace merged, rows tagged with theirs, minus the
    /// ones hidden from the calendar (Home's `calendar_hidden_docs`).
    /// Otherwise the context's own workspace, untagged.
    pub everywhere: bool,
    /// The tasks done today, at the end of Today (`placed_by: done`).
    /// Without it a done task is not on the agenda at all: the tray
    /// and the widgets, which show what is left.
    pub completed: bool,
}

/// An agenda as placed, before it is shaped into rows: what the
/// terminal prints from.
pub struct Placed {
    /// `YYYY-MM-DD`.
    pub today: String,
    /// In date order: each non-empty day gone by (none unless the
    /// query asks for them), Today, even when empty, then each
    /// non-empty day ahead.
    pub days: Vec<rules::AgendaDay>,
    /// Name and emoji of every list of the docs placed, by the row's
    /// workspace (`None` on a single-doc agenda) and list id.
    lists: HashMap<(Option<DocId>, String), (String, Option<String>)>,
}

/// Place the items the query covers.
pub fn place(cx: &Ctx, query: &AgendaQuery) -> anyhow::Result<Placed> {
    let today = match query.today {
        Some(t) => NaiveDate::parse_from_str(t, "%Y-%m-%d")
            .map_err(|_| anyhow::anyhow!("--today must be YYYY-MM-DD: {t:?}"))?,
        None => Local::now().date_naive(),
    };
    let horizon = today
        .checked_add_days(Days::new(u64::from(query.days)))
        .ok_or_else(|| anyhow::anyhow!("--days out of range"))?;
    let floor = today
        .checked_sub_days(Days::new(u64::from(query.past_days)))
        .ok_or_else(|| anyhow::anyhow!("--past out of range"))?;
    // The local midnight today began at: a task done since is today's.
    let closed_since = query
        .completed
        .then(|| {
            today
                .and_hms_opt(0, 0, 0)
                .and_then(|t| t.and_local_timezone(Local).earliest())
                .map(|t| t.timestamp_millis())
        })
        .flatten();
    let floor = floor.format("%Y-%m-%d").to_string();
    let today = today.format("%Y-%m-%d").to_string();
    let horizon = horizon.format("%Y-%m-%d").to_string();

    let docs: Vec<(Option<&Candidate>, &monoplan_core::Doc)> = if query.everywhere {
        let hidden = cx.home_doc()?.get_settings().calendar_hidden_docs;
        cx.workspaces
            .iter()
            .filter(|(c, _)| !hidden.iter().any(|h| *h == c.doc_id.0.to_string()))
            .map(|(c, d)| (Some(c), *d))
            .collect()
    } else {
        vec![(None, cx.doc)]
    };
    let shown: Vec<(Option<&Candidate>, Vec<ItemView>)> =
        docs.iter().map(|(c, d)| (*c, d.all_items())).collect();
    let mut lists = HashMap::new();
    for (c, d) in &docs {
        for l in d.all_lists() {
            lists.insert((c.map(|c| c.doc_id), l.id), (l.name, l.icon));
        }
    }
    let sources: Vec<Source<'_>> = shown
        .iter()
        .map(|(c, items)| (*c, items.as_slice()))
        .collect();
    Ok(Placed {
        days: rules::build_agenda_from(&sources, &floor, &today, &horizon, closed_since),
        today,
        lists,
    })
}

/// The agenda as rows: Today first (always there), then each
/// non-empty day up to the horizon. With `past_days`, the non-empty
/// days gone by come ahead of Today, oldest first.
pub fn agenda(cx: &Ctx, query: &AgendaQuery) -> anyhow::Result<Vec<AgendaGroup>> {
    Ok(groups(&place(cx, query)?))
}

/// [`agenda`] as the JSON `agenda --json` prints, compact: a file
/// format for a reader that is not this process (the iOS widget's
/// snapshot).
pub fn agenda_json(cx: &Ctx, query: &AgendaQuery) -> anyhow::Result<String> {
    Ok(serde_json::to_string(&agenda(cx, query)?)?)
}

/// [`Placed`] shaped into rows.
pub fn groups(placed: &Placed) -> Vec<AgendaGroup> {
    placed
        .days
        .iter()
        .map(|d| AgendaGroup {
            today: d.day == placed.today,
            day: d.day.clone(),
            rows: d
                .rows
                .iter()
                .map(|r| {
                    let (doc_id, doc_name) = doc_tag(r.doc.as_ref());
                    let list = placed
                        .lists
                        .get(&(r.doc.as_ref().map(|c| c.doc_id), r.item.list_id.clone()));
                    AgendaEntry {
                        id: r.item.id.clone(),
                        text: r.item.text.clone(),
                        list_id: r.item.list_id.clone(),
                        list_name: list.map(|(name, _)| name.clone()),
                        list_icon: list.and_then(|(_, icon)| icon.clone()),
                        state: r.item.state.into(),
                        event: r.item.event,
                        when: r.item.when.clone(),
                        duration: r.item.duration,
                        deadline: r.item.deadline.clone(),
                        place: r.item.place.as_ref().map(Place::from),
                        placed_by: r.placed_by,
                        tone: r.tone,
                        doc_id,
                        doc_name,
                    }
                })
                .collect(),
        })
        .collect()
}
