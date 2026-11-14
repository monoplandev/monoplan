//! Agenda: dated items by day, Today first (`commands::agenda` for
//! the placement, tone and ordering rules, which are pure functions
//! tested there).

use chrono::{Days, Local, NaiveDate};
use monoplan_core::ItemView;
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

/// Which field put the row on its day.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
pub enum AgendaPlacedBy {
    When,
    Deadline,
}

/// One row of a group; the `agenda --json` row.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct AgendaEntry {
    pub id: String,
    pub text: String,
    pub list_id: String,
    pub state: ItemState,
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
    /// Absent on an Unscheduled row: no field placed it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub placed_by: Option<AgendaPlacedBy>,
    pub tone: AgendaTone,
    /// The merged agenda only: the row's workspace. `doc_name` is
    /// absent for Home (never named) and for an unnamed doc.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub doc_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub doc_name: Option<String>,
}

/// One day's rows, or the Unscheduled group; the `agenda --json`
/// element.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct AgendaGroup {
    /// `YYYY-MM-DD`, or `None` (JSON `null`) for the Unscheduled group.
    pub day: Option<String>,
    pub today: bool,
    pub rows: Vec<AgendaEntry>,
}

/// What an agenda is built from and for.
pub struct AgendaQuery<'a> {
    /// How many days past today to show (Today is always shown).
    pub days: u32,
    /// The local date to use as today (`YYYY-MM-DD`); the clock's
    /// when `None`.
    pub today: Option<&'a str>,
    /// Every open workspace merged, rows tagged with theirs, minus the
    /// ones hidden from the calendar (Home's `calendar_hidden_docs`).
    /// Otherwise the context's own workspace, untagged.
    pub everywhere: bool,
}

/// An agenda as placed, before it is shaped into rows: what the
/// terminal prints from.
pub struct Placed {
    /// `YYYY-MM-DD`.
    pub today: String,
    /// Today first, even when empty; then each non-empty day.
    pub days: Vec<rules::AgendaDay>,
    /// Open events with no date at all, oldest first.
    pub unscheduled: Vec<(Option<Candidate>, ItemView)>,
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
    let today = today.format("%Y-%m-%d").to_string();
    let horizon = horizon.format("%Y-%m-%d").to_string();

    let shown: Vec<(Option<&Candidate>, Vec<ItemView>)> = if query.everywhere {
        let hidden = cx.home_doc()?.get_settings().calendar_hidden_docs;
        cx.workspaces
            .iter()
            .filter(|(c, _)| !hidden.iter().any(|h| *h == c.doc_id.0.to_string()))
            .map(|(c, d)| (Some(c), d.all_items()))
            .collect()
    } else {
        vec![(None, cx.doc.all_items())]
    };
    let sources: Vec<Source<'_>> = shown
        .iter()
        .map(|(c, items)| (*c, items.as_slice()))
        .collect();
    Ok(Placed {
        days: rules::build_agenda_from(&sources, &today, &horizon),
        unscheduled: rules::unscheduled_from(&sources)
            .into_iter()
            .map(|(doc, item)| (doc, item.clone()))
            .collect(),
        today,
    })
}

/// The agenda as rows: Today first (always there), the Unscheduled
/// group after it when there is one, then each non-empty day up to the
/// horizon.
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
    let mut out: Vec<AgendaGroup> = placed
        .days
        .iter()
        .map(|d| AgendaGroup {
            today: d.day == placed.today,
            day: Some(d.day.clone()),
            rows: d
                .rows
                .iter()
                .map(|r| {
                    let (doc_id, doc_name) = doc_tag(r.doc.as_ref());
                    AgendaEntry {
                        id: r.item.id.clone(),
                        text: r.item.text.clone(),
                        list_id: r.item.list_id.clone(),
                        state: r.item.state.into(),
                        when: r.item.when.clone(),
                        duration: r.item.duration,
                        deadline: r.item.deadline.clone(),
                        place: r.item.place.as_ref().map(Place::from),
                        placed_by: Some(r.placed_by),
                        tone: r.tone,
                        doc_id,
                        doc_name,
                    }
                })
                .collect(),
        })
        .collect();
    if !placed.unscheduled.is_empty() {
        // After Today, which the placement always puts first.
        out.insert(
            1,
            AgendaGroup {
                day: None,
                today: false,
                rows: placed
                    .unscheduled
                    .iter()
                    .map(|(doc, item)| {
                        let (doc_id, doc_name) = doc_tag(doc.as_ref());
                        AgendaEntry {
                            id: item.id.clone(),
                            text: item.text.clone(),
                            list_id: item.list_id.clone(),
                            state: item.state.into(),
                            when: None,
                            duration: None,
                            deadline: None,
                            place: item.place.as_ref().map(Place::from),
                            placed_by: None,
                            tone: AgendaTone::Neutral,
                            doc_id,
                            doc_name,
                        }
                    })
                    .collect(),
            },
        );
    }
    out
}
