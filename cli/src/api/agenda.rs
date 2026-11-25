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

/// Row tone: overdue is a task's `when` that is over.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
pub enum AgendaTone {
    Neutral,
    Overdue,
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
    pub place: Option<Place>,
    /// The account id the item is assigned to (`spec/sharing-plan.md`
    /// "Assignment"); absent when unassigned.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub assignee: Option<String>,
    pub tone: AgendaTone,
    /// The merged agenda only: the row's workspace. `doc_name` is
    /// absent for Home (never named) and for an unnamed doc.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub doc_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub doc_name: Option<String>,
    /// On a spread agenda (`AgendaQuery::spread`), a row of a `when`
    /// covering more than one day: which day of the span this row is.
    /// Absent otherwise, and off the JSON then.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "uniffi", uniffi(default = None))]
    pub span: Option<AgendaSpan>,
    // What the row means on its day, worked out here so every client
    // draws the same thing (the web's `Upcoming.tsx` reads a `DayRow`
    // the same way). Not on the JSON: a reader of that has the fields
    // above.
    /// The day the row is placed on (`YYYY-MM-DD`): a span's later
    /// rows are on later days; an overdue row is on Today.
    #[serde(skip)]
    pub day: String,
    /// The row is an event's on a day gone by: a record of what was on,
    /// drawn muted.
    #[serde(skip)]
    pub past: bool,
    /// The row leads with a checkbox. An event's does not while it is
    /// open (nothing asks for the tick) or once its day has gone by:
    /// the event pill holds the box's place. One closed from today on
    /// shows its box, ticked or crossed.
    #[serde(skip)]
    pub shows_check: bool,
    /// The N of "Day N" under the title: the day of a multi-day span
    /// this row is, the first included. Absent off a span, in Overdue
    /// (one row) and on a timed span under 24 hours (an evening that
    /// runs past midnight is one occasion).
    #[serde(skip)]
    pub span_label: Option<u32>,
    /// What the time lane at the row's trailing edge holds.
    #[serde(skip)]
    pub lane: AgendaLane,
}

/// What a Calendar row's time lane holds (the web's `timeLabel`). The
/// words are the client's.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
pub enum AgendaLane {
    /// Nothing: a task with no clock time (for the day, it does not
    /// fill it), where an event would say "All day".
    Empty,
    /// "All day": an event with no clock time, or a timed span's day it
    /// runs clean through.
    AllDay,
    /// The start's clock, `HH:MM`: a timed row, a timed span's first
    /// day.
    Start { clock: String },
    /// "until HH:MM": a timed span's last day.
    Until { clock: String },
    /// The day an overdue task was planned for (`YYYY-MM-DD`): Overdue
    /// has no day header to carry it.
    Planned { day: String },
}

/// A row's place in a span of days (`spec/calendar-plan.md` "Spans"):
/// the web's `DayRow.span`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct AgendaSpan {
    /// 1-based: the span's first day is 1.
    pub day: u32,
    /// How many days the span covers.
    pub total: u32,
}

/// One day's rows; the `agenda --json` element.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct AgendaGroup {
    /// `YYYY-MM-DD`. Today's date on the Overdue group.
    pub day: String,
    pub today: bool,
    pub rows: Vec<AgendaEntry>,
    /// The Calendar's Overdue section ([`calendar`]): the open tasks
    /// whose day has passed, right above Today. Elsewhere they lead
    /// Today's own group.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    #[cfg_attr(feature = "uniffi", uniffi(default = false))]
    pub overdue: bool,
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
    /// A row on every day a `when` covers, each with its place in the
    /// span (`commands::agenda::build_agenda_spread`), as the web's
    /// Upcoming. Otherwise a span is on its start day alone.
    pub spread: bool,
}

/// The Calendar filter's checked workspaces, read off Home's
/// `calendar_hidden_docs` (a personal preference synced across this
/// account's devices) as the web's Upcoming filter reads it: nothing
/// open hidden is no filter, so nothing checked; otherwise every open
/// workspace not hidden, in `Ctx::workspaces` order. Hidden ids of docs
/// no longer open are harmless and ignored.
pub fn calendar_checked_docs(cx: &Ctx) -> anyhow::Result<Vec<String>> {
    let hidden = cx.home_doc()?.get_settings().calendar_hidden_docs;
    Ok(checked_from_hidden(&open_doc_ids(cx), &hidden))
}

/// Make `checked` the Calendar filter's checked workspaces: none
/// checked (or every one) hides nothing, otherwise every open
/// workspace not checked is hidden. Only the workspaces that change are
/// written, a commit each on Home.
pub fn set_calendar_checked_docs(cx: &Ctx, checked: &[String]) -> anyhow::Result<()> {
    let home = cx.home_doc()?;
    let hidden = home.get_settings().calendar_hidden_docs;
    for (doc_id, hide) in hidden_changes(&open_doc_ids(cx), &hidden, checked) {
        home.set_calendar_doc_hidden(&doc_id, hide)?;
    }
    Ok(())
}

fn open_doc_ids(cx: &Ctx) -> Vec<String> {
    cx.workspaces
        .iter()
        .map(|(c, _)| c.doc_id.0.to_string())
        .collect()
}

/// [`calendar_checked_docs`] over plain ids.
fn checked_from_hidden(open: &[String], hidden: &[String]) -> Vec<String> {
    if !open.iter().any(|d| hidden.contains(d)) {
        return Vec::new();
    }
    open.iter()
        .filter(|d| !hidden.contains(d))
        .cloned()
        .collect()
}

/// The writes that make `checked` the checked set: each open workspace
/// whose hidden state changes, and to what.
fn hidden_changes(open: &[String], hidden: &[String], checked: &[String]) -> Vec<(String, bool)> {
    let all = open.iter().all(|d| checked.contains(d));
    open.iter()
        .filter_map(|d| {
            let hide = !checked.is_empty() && !all && !checked.contains(d);
            (hide != hidden.contains(d)).then(|| (d.clone(), hide))
        })
        .collect()
}

/// The Calendar nav entry's count over every workspace minus those
/// hidden from the calendar: Open items owed or on today
/// (`rules::attention_count`, the web's `attentionBadge`). `today` is
/// the clock's local date when `None`.
pub fn attention_count(cx: &Ctx, today: Option<&str>) -> anyhow::Result<u32> {
    let today = match today {
        Some(t) => t.to_string(),
        None => Local::now().date_naive().format("%Y-%m-%d").to_string(),
    };
    let hidden = cx.home_doc()?.get_settings().calendar_hidden_docs;
    let mut count = 0;
    for (c, d) in cx.workspaces {
        if hidden.iter().any(|h| *h == c.doc_id.0.to_string()) {
            continue;
        }
        count += rules::attention_count(&d.all_items(), &today);
    }
    Ok(count)
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
        days: if query.spread {
            rules::build_agenda_spread(&sources, &floor, &today, &horizon)
        } else {
            rules::build_agenda_from(&sources, &floor, &today, &horizon)
        },
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
    let today = placed.today.as_str();
    placed
        .days
        .iter()
        .map(|d| AgendaGroup {
            today: d.day == placed.today,
            day: d.day.clone(),
            overdue: false,
            rows: d
                .rows
                .iter()
                .map(|r| {
                    let (doc_id, doc_name) = doc_tag(r.doc.as_ref());
                    let list = placed
                        .lists
                        .get(&(r.doc.as_ref().map(|c| c.doc_id), r.item.list_id.clone()));
                    let event = r.item.is_event();
                    let past = event && d.day.as_str() < today;
                    let span = r.span.map(|(day, total)| AgendaSpan { day, total });
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
                        place: r.item.place.as_ref().map(Place::from),
                        assignee: r.item.assignee.clone(),
                        tone: r.tone,
                        doc_id,
                        doc_name,
                        span,
                        day: d.day.clone(),
                        past,
                        shows_check: !event || (r.item.state.is_closed() && !past),
                        span_label: span_label(
                            r.item.when.as_deref(),
                            r.item.duration,
                            span,
                            r.tone,
                        ),
                        lane: lane(r.item.when.as_deref(), r.item.duration, event, span, r.tone),
                    }
                })
                .collect(),
        })
        .collect()
}

/// [`AgendaEntry::span_label`].
fn span_label(
    when: Option<&str>,
    duration: Option<u32>,
    span: Option<AgendaSpan>,
    tone: AgendaTone,
) -> Option<u32> {
    let span = span?;
    if tone == AgendaTone::Overdue {
        return None;
    }
    let timed = when.is_some_and(|w| w.len() >= 16);
    if timed && duration.unwrap_or(0) < 1440 {
        return None;
    }
    Some(span.day)
}

/// [`AgendaEntry::lane`]: an overdue row its planned day; otherwise a
/// timed row its start clock, or on a timed span its first day's, "All
/// day" on the days it runs through and its end clock on the last; an
/// event with no clock "All day", a task nothing.
fn lane(
    when: Option<&str>,
    duration: Option<u32>,
    event: bool,
    span: Option<AgendaSpan>,
    tone: AgendaTone,
) -> AgendaLane {
    let Some(when) = when else {
        return AgendaLane::Empty;
    };
    if tone == AgendaTone::Overdue {
        return AgendaLane::Planned {
            day: when[..when.len().min(10)].to_string(),
        };
    }
    let all_day = if event {
        AgendaLane::AllDay
    } else {
        AgendaLane::Empty
    };
    let Some(clock) = when.get(11..16) else {
        return all_day;
    };
    let Some(span) = span.filter(|s| s.day > 1) else {
        return AgendaLane::Start {
            clock: clock.to_string(),
        };
    };
    let start = clock
        .split_once(':')
        .and_then(|(h, m)| Some(h.parse::<u32>().ok()? * 60 + m.parse::<u32>().ok()?));
    match (start, duration) {
        (Some(start), Some(minutes)) if minutes > 0 && span.day == span.total => {
            let end = (start + minutes) % 1440;
            AgendaLane::Until {
                clock: format!("{:02}:{:02}", end / 60, end % 60),
            }
        }
        _ => all_day,
    }
}

/// How far ahead [`calendar`] reaches: far enough to be the whole
/// future. There is no recurrence, so dated items are a finite set and
/// the placement walks every item whatever the horizon.
pub const CALENDAR_DAYS: u32 = 36_500;
/// How far back [`calendar`] reaches: the days gone by shown above
/// Today. Bounded, as the past only grows.
pub const CALENDAR_PAST_DAYS: u32 = 365;
/// Days past Today that have a group when nothing is on them, if
/// nothing is placed later: two weeks of days in all, the web's
/// `AGENDA_ALL_DAYS_AHEAD`.
pub const CALENDAR_FILL_AHEAD: u64 = 13;

/// The Calendar's device preferences, the web's filter-menu view
/// options (not filters: not counted, not cleared).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct CalendarOptions {
    /// Only Today and the days something is on ("Hide empty days");
    /// otherwise every day from Today on has a group.
    pub hide_empty_days: bool,
    /// No day before Today ("Hide past days"). Overdue stays.
    pub hide_past_days: bool,
}

/// The Calendar page as the Apple apps draw it, over every workspace
/// minus those hidden from the calendar (`spec/calendar-plan.md`
/// "Agenda"): the days gone by that something was on (unless
/// `hide_past_days`), Overdue as its own group while something is,
/// Today (always), then the days ahead: every one for two weeks and on
/// to the last day with rows, or only those with rows under
/// `hide_empty_days`. Spans have a row on each of their days. The web
/// builds the same page itself (`groupByDay`, `fillEmptyDays`).
pub fn calendar(
    cx: &Ctx,
    today: Option<&str>,
    options: CalendarOptions,
) -> anyhow::Result<Vec<AgendaGroup>> {
    let placed = place(
        cx,
        &AgendaQuery {
            days: CALENDAR_DAYS,
            past_days: if options.hide_past_days {
                0
            } else {
                CALENDAR_PAST_DAYS
            },
            today,
            everywhere: true,
            spread: true,
        },
    )?;
    let today = placed.today.clone();
    Ok(shape_calendar(groups(&placed), &today, options))
}

/// [`calendar`]'s days from the agenda's: Overdue lifted out of Today
/// into its own group, the days gone by dropped under
/// `hide_past_days`, the empty days ahead filled in unless
/// `hide_empty_days`.
fn shape_calendar(
    groups: Vec<AgendaGroup>,
    today: &str,
    options: CalendarOptions,
) -> Vec<AgendaGroup> {
    let mut out = Vec::new();
    let mut ahead: Vec<AgendaGroup> = Vec::new();
    for mut group in groups {
        if group.day.as_str() < today {
            if !options.hide_past_days {
                out.push(group);
            }
            continue;
        }
        if group.today {
            let (overdue, own): (Vec<_>, Vec<_>) = group
                .rows
                .into_iter()
                .partition(|r| r.tone == AgendaTone::Overdue);
            group.rows = own;
            if !overdue.is_empty() {
                out.push(AgendaGroup {
                    day: group.day.clone(),
                    today: false,
                    rows: overdue,
                    overdue: true,
                });
            }
        }
        ahead.push(group);
    }
    if options.hide_empty_days {
        out.extend(ahead);
        return out;
    }
    let Ok(start) = NaiveDate::parse_from_str(today, "%Y-%m-%d") else {
        out.extend(ahead);
        return out;
    };
    let mut last = start + Days::new(CALENDAR_FILL_AHEAD);
    if let Some(latest) = ahead
        .iter()
        .filter_map(|g| NaiveDate::parse_from_str(&g.day, "%Y-%m-%d").ok())
        .max()
        && latest > last
    {
        last = latest;
    }
    let mut ahead = ahead.into_iter().peekable();
    let mut day = start;
    while day <= last {
        let stamp = day.format("%Y-%m-%d").to_string();
        match ahead.peek() {
            Some(g) if g.day == stamp => out.push(ahead.next().expect("peeked")),
            _ => out.push(AgendaGroup {
                today: stamp == today,
                day: stamp,
                rows: Vec::new(),
                overdue: false,
            }),
        }
        day = day + Days::new(1);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn group(day: &str, today: bool, rows: Vec<AgendaEntry>) -> AgendaGroup {
        AgendaGroup {
            day: day.into(),
            today,
            rows,
            overdue: false,
        }
    }

    fn row(id: &str, tone: AgendaTone) -> AgendaEntry {
        AgendaEntry {
            id: id.into(),
            text: id.into(),
            list_id: "inbox".into(),
            list_name: None,
            list_icon: None,
            state: ItemState::Todo,
            event: false,
            when: None,
            duration: None,
            place: None,
            assignee: None,
            tone,
            doc_id: None,
            doc_name: None,
            span: None,
            day: String::new(),
            past: false,
            shows_check: true,
            span_label: None,
            lane: AgendaLane::Empty,
        }
    }

    fn days(groups: &[AgendaGroup]) -> Vec<&str> {
        groups.iter().map(|g| g.day.as_str()).collect()
    }

    #[test]
    fn the_calendar_lifts_overdue_and_fills_two_weeks() {
        let groups = vec![
            group("2026-11-20", false, vec![row("gone", AgendaTone::Neutral)]),
            group(
                "2026-11-23",
                true,
                vec![
                    row("late", AgendaTone::Overdue),
                    row("now", AgendaTone::Neutral),
                ],
            ),
            group("2026-11-25", false, vec![row("b", AgendaTone::Neutral)]),
        ];
        let out = shape_calendar(groups.clone(), "2026-11-23", CalendarOptions::default());
        assert_eq!(out.len(), 2 + 14);
        assert_eq!(
            &days(&out)[..4],
            ["2026-11-20", "2026-11-23", "2026-11-23", "2026-11-24"]
        );
        assert!(out[1].overdue && !out[1].today);
        assert_eq!(out[1].rows[0].id, "late");
        assert!(out[2].today);
        assert_eq!(out[2].rows.len(), 1);
        assert_eq!(out.last().map(|g| g.day.as_str()), Some("2026-12-06"));

        let out = shape_calendar(
            groups,
            "2026-11-23",
            CalendarOptions {
                hide_empty_days: true,
                hide_past_days: true,
            },
        );
        assert_eq!(days(&out), ["2026-11-23", "2026-11-23", "2026-11-25"]);
    }

    #[test]
    fn the_calendar_fills_to_the_last_day_with_rows() {
        let groups = vec![
            group("2026-11-23", true, vec![]),
            group("2027-01-02", false, vec![row("c", AgendaTone::Neutral)]),
        ];
        let out = shape_calendar(groups, "2026-11-23", CalendarOptions::default());
        assert_eq!(out.last().map(|g| g.day.as_str()), Some("2027-01-02"));
        assert_eq!(out.len(), 41);
        assert_eq!(out.iter().filter(|g| g.today).count(), 1);
    }

    #[test]
    fn the_lane_reads_what_the_day_holds() {
        let span = |day, total| Some(AgendaSpan { day, total });
        let n = AgendaTone::Neutral;
        assert_eq!(
            lane(Some("2026-11-30"), None, false, None, n),
            AgendaLane::Empty
        );
        assert_eq!(
            lane(Some("2026-11-30"), None, true, None, n),
            AgendaLane::AllDay
        );
        let timed = Some("2026-11-30T22:00");
        let long = Some(2 * 1440 + 180);
        let start = AgendaLane::Start {
            clock: "22:00".into(),
        };
        assert_eq!(lane(timed, long, true, span(1, 3), n), start);
        assert_eq!(lane(timed, long, true, span(2, 3), n), AgendaLane::AllDay);
        assert_eq!(
            lane(timed, long, true, span(3, 3), n),
            AgendaLane::Until {
                clock: "01:00".into()
            }
        );
        assert_eq!(lane(timed, long, false, span(2, 3), n), AgendaLane::Empty);
        assert_eq!(
            lane(
                Some("2026-11-02T08:00"),
                None,
                false,
                None,
                AgendaTone::Overdue
            ),
            AgendaLane::Planned {
                day: "2026-11-02".into()
            }
        );
        // "Day N": not on an evening past midnight, not in Overdue.
        assert_eq!(span_label(timed, long, span(2, 3), n), Some(2));
        assert_eq!(span_label(timed, Some(120), span(2, 2), n), None);
        assert_eq!(
            span_label(Some("2026-11-30"), Some(4320), span(1, 3), n),
            Some(1)
        );
    }

    #[test]
    fn an_event_has_no_checkbox_while_open_or_once_its_day_has_gone_by() {
        use monoplan_core::{LIST_INBOX, WorkflowState};
        let item = |id: &str, when: &str, event: bool, done: bool| ItemView {
            id: id.into(),
            text: id.into(),
            notes: String::new(),
            list_id: LIST_INBOX.into(),
            state: if done {
                WorkflowState::Done
            } else {
                WorkflowState::Backlog
            },
            lifecycle_at: 1,
            when: Some(when.into()),
            when_zoned: None,
            when_suggested: None,
            event,
            duration: None,
            place: None,
            created_at: 1,
            created_by: None,
            assignee: None,
            subtasks: Vec::new(),
            files: Vec::new(),
            started_at: None,
            done_at: None,
            binned_at: None,
            inbox_at: None,
        };
        let today = "2026-10-26";
        let items = [
            item("open", "2026-10-26T20:00", true, false),
            item("done", "2026-10-26T21:00", true, true),
            item("gone", "2026-10-20T20:00", true, true),
            item("task", "2026-10-26", false, false),
        ];
        let placed = Placed {
            days: rules::build_agenda_spread(&[(None, &items)], "2026-10-01", today, "2026-11-30"),
            today: today.into(),
            lists: HashMap::new(),
        };
        let rows: Vec<(String, bool, bool)> = groups(&placed)
            .into_iter()
            .flat_map(|g| g.rows)
            .map(|r| (r.id, r.shows_check, r.past))
            .collect();
        let by = |id: &str| rows.iter().find(|r| r.0 == id).map(|r| (r.1, r.2));
        assert_eq!(by("open"), Some((false, false)));
        assert_eq!(by("done"), Some((true, false)));
        assert_eq!(by("gone"), Some((false, true)));
        assert_eq!(by("task"), Some((true, false)));
    }

    #[test]
    fn checked_workspaces_round_trip_through_the_hidden_set() {
        let open: Vec<String> = ["h", "a", "b"].map(String::from).to_vec();
        let ids = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert!(checked_from_hidden(&open, &ids(&["gone"])).is_empty());
        assert_eq!(checked_from_hidden(&open, &ids(&["b"])), ids(&["h", "a"]));
        assert_eq!(
            hidden_changes(&open, &[], &ids(&["a"])),
            vec![("h".to_string(), true), ("b".to_string(), true)]
        );
        // Every one checked, or none, hides nothing.
        assert_eq!(
            hidden_changes(&open, &ids(&["b"]), &ids(&["h", "a", "b"])),
            vec![("b".to_string(), false)]
        );
        assert_eq!(
            hidden_changes(&open, &ids(&["b"]), &[]),
            vec![("b".to_string(), false)]
        );
    }
}
