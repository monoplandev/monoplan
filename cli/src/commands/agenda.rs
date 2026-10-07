//! `monoplan agenda`: dated items by day, Today first with overdue
//! deadlines folded in, then the coming days (`spec/calendar-plan.md`
//! "Agenda"). A past `when` has gone by like an event and places nothing.
//! Done items keep their `when` day and slot (the tick does not erase
//! "happens on"); their deadline is settled and places nothing. Events
//! (`spec/events-plan.md`) place and print by the same rules; an Open
//! event with no date at all has no day, so it prints in an Unscheduled
//! group after Today.
//!
//! Placement, tone, and ordering are pure functions of the item views
//! and a `today` stamp, so they are unit-tested here without a doc.

use chrono::{Datelike, Days, Local, NaiveDate};
use clap::Parser;
use monoplan_core::{ItemView, Place};
use serde::Serialize;

use crate::sync::Session;

use super::items::{date_tags, print_json, row_box};

#[derive(Parser, Debug)]
pub struct AgendaArgs {
    /// How many days past today to show (Today is always shown).
    #[arg(long, default_value_t = 14)]
    pub days: u32,
    /// Override the local date used as "today" (`YYYY-MM-DD`).
    #[arg(long)]
    pub today: Option<String>,
    /// Machine-parseable output.
    #[arg(long)]
    pub json: bool,
}

/// Row tone, least to most urgent. Both come from the deadline; `when`
/// is never judged and never red.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Tone {
    Neutral,
    Warning,
    Overdue,
}

impl Tone {
    fn label(self) -> Option<&'static str> {
        match self {
            Tone::Neutral => None,
            Tone::Warning => Some("due today"),
            Tone::Overdue => Some("overdue"),
        }
    }
}

/// Which field put the row on its day.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PlacedBy {
    When,
    Deadline,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgendaRow {
    pub item: ItemView,
    pub placed_by: PlacedBy,
    pub tone: Tone,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgendaDay {
    /// `YYYY-MM-DD`.
    pub day: String,
    pub rows: Vec<AgendaRow>,
}

/// Group dated items by placement day. `today` and `horizon` are
/// `YYYY-MM-DD`; days after `horizon` are dropped. Today is always the
/// first group, even when empty; later empty days are skipped.
///
/// - Placement day: the earlier of the `when` day and the `deadline`
///   day, the deadline clamped up to today, so an overdue deadline lands
///   in Today. A past `when` is over and places nothing: an item with
///   only that is dropped, and one with a deadline too places by it.
/// - Tone: overdue (deadline < today), warning (deadline = today), else
///   neutral.
/// - Within a day: overdue deadlines (oldest first), then the day's own
///   rows by the raw string of the placing field (all-day ahead of
///   timed), then `created_at`. Ticking a row does not move it.
/// - Done items place by `when` only, on that day (today or later),
///   neutral. A done item with no `when`, or a past `when`, is dropped:
///   nothing is owed and past days are not shown. Cancelled and binned
///   items never appear: a cancelled slot is something that did not
///   happen, so it clutters rather than records.
pub fn build_agenda<'a>(items: &'a [ItemView], today: &'a str, horizon: &str) -> Vec<AgendaDay> {
    // (day, fold group, placing raw, created_at, row)
    let mut placed: Vec<(String, u8, String, i64, AgendaRow)> = Vec::new();
    for item in items {
        if item.is_binned() || item.is_cancelled() {
            continue;
        }
        // Past days are not shown, so a past `when` places nothing.
        let when_day = item
            .when
            .as_deref()
            .map(|w| &w[..w.len().min(10)])
            .filter(|w| *w >= today);
        let deadline_day = item.deadline.as_deref();
        if when_day.is_none() && deadline_day.is_none() {
            continue;
        }
        if item.is_done() {
            let Some(day) = when_day else {
                continue;
            };
            if day > horizon {
                continue;
            }
            placed.push((
                day.to_string(),
                1,
                item.when.clone().unwrap(),
                item.created_at,
                AgendaRow {
                    item: item.clone(),
                    placed_by: PlacedBy::When,
                    tone: Tone::Neutral,
                },
            ));
            continue;
        }
        let deadline_placed = deadline_day.map(|d| if d < today { today } else { d });
        // `when` wins a tie: it is the "happens on" date and renders first.
        let (day, placed_by, raw) = match (when_day, deadline_placed) {
            (Some(w), Some(d)) if d < w => (d, PlacedBy::Deadline, deadline_day.unwrap()),
            (Some(w), _) => (w, PlacedBy::When, item.when.as_deref().unwrap()),
            (None, Some(d)) => (d, PlacedBy::Deadline, deadline_day.unwrap()),
            (None, None) => unreachable!(),
        };
        if day > horizon {
            continue;
        }
        let deadline_overdue = deadline_day.is_some_and(|d| d < today);
        let tone = if deadline_overdue {
            Tone::Overdue
        } else if deadline_day == Some(today) {
            Tone::Warning
        } else {
            Tone::Neutral
        };
        // Today's fold: overdue deadlines lead (by their own date), then
        // today's own rows, done or not.
        let (group, raw) = if deadline_overdue {
            (0, deadline_day.unwrap().to_string())
        } else {
            (1, raw.to_string())
        };
        placed.push((
            day.to_string(),
            group,
            raw,
            item.created_at,
            AgendaRow {
                item: item.clone(),
                placed_by,
                tone,
            },
        ));
    }
    placed.sort_by(|a, b| {
        a.0.cmp(&b.0)
            .then(a.1.cmp(&b.1))
            .then(a.2.cmp(&b.2))
            .then(a.3.cmp(&b.3))
    });

    let mut out = vec![AgendaDay {
        day: today.to_string(),
        rows: Vec::new(),
    }];
    for (day, _, _, _, row) in placed {
        match out.last_mut() {
            Some(last) if last.day == day => last.rows.push(row),
            _ => out.push(AgendaDay {
                day,
                rows: vec![row],
            }),
        }
    }
    out
}

/// Open events with neither a `when` nor a `deadline`, oldest first.
/// They have no day to sit on, and no list view shows an event, so the
/// agenda is the one place they can surface. A past `when` is not
/// unscheduled: it went by.
pub fn unscheduled_events(items: &[ItemView]) -> Vec<&ItemView> {
    let mut out: Vec<&ItemView> = items
        .iter()
        .filter(|i| i.is_event() && i.is_open() && i.when.is_none() && i.deadline.is_none())
        .collect();
    out.sort_by(|a, b| a.created_at.cmp(&b.created_at).then(a.id.cmp(&b.id)));
    out
}

pub async fn run(args: AgendaArgs, sync: bool) -> anyhow::Result<()> {
    let today = match &args.today {
        Some(t) => NaiveDate::parse_from_str(t, "%Y-%m-%d")
            .map_err(|_| anyhow::anyhow!("--today must be YYYY-MM-DD: {t:?}"))?,
        None => Local::now().date_naive(),
    };
    let horizon = today
        .checked_add_days(Days::new(u64::from(args.days)))
        .ok_or_else(|| anyhow::anyhow!("--days out of range"))?;
    let today_s = today.format("%Y-%m-%d").to_string();
    let horizon_s = horizon.format("%Y-%m-%d").to_string();

    let session = Session::open(sync).await?;
    let items = session.doc().all_items();
    let days = build_agenda(&items, &today_s, &horizon_s);
    let unscheduled = unscheduled_events(&items);
    if args.json {
        let mut out: Vec<DayJson<'_>> = days
            .iter()
            .map(|d| DayJson {
                day: Some(&d.day),
                today: d.day == today_s,
                rows: d.rows.iter().map(row_json).collect(),
            })
            .collect();
        if !unscheduled.is_empty() {
            // After Today, which `build_agenda` always puts first.
            out.insert(
                1,
                DayJson {
                    day: None,
                    today: false,
                    rows: unscheduled.iter().map(|i| unscheduled_json(i)).collect(),
                },
            );
        }
        print_json(&out)?;
    } else {
        print_agenda(&days, &unscheduled, &today_s);
    }
    session.flush().await?;
    Ok(())
}

fn print_agenda(days: &[AgendaDay], unscheduled: &[&ItemView], today: &str) {
    for (i, day) in days.iter().enumerate() {
        if i > 0 {
            println!();
        }
        // Unscheduled sits after Today, ahead of the coming days.
        if i == 1 && !unscheduled.is_empty() {
            print_unscheduled(unscheduled);
            println!();
        }
        let weekday = NaiveDate::parse_from_str(&day.day, "%Y-%m-%d")
            .map(|d| d.weekday().to_string())
            .unwrap_or_default();
        if day.day == today {
            println!("Today  {}  {weekday}", day.day);
        } else {
            println!("{}  {weekday}", day.day);
        }
        if day.rows.is_empty() {
            println!("  (nothing)");
        }
        for row in &day.rows {
            let tone = row
                .tone
                .label()
                .map(|l| format!("  ({l})"))
                .unwrap_or_default();
            println!(
                "  {}  {} {}{}{tone}",
                row.item.id,
                row_box(&row.item),
                row.item.text,
                date_tags(&row.item)
            );
        }
    }
    // Today was the only day: the group still follows it.
    if days.len() == 1 && !unscheduled.is_empty() {
        println!();
        print_unscheduled(unscheduled);
    }
}

fn print_unscheduled(unscheduled: &[&ItemView]) {
    println!("Unscheduled");
    for item in unscheduled {
        println!("  {}  {} {}", item.id, row_box(item), item.text);
    }
}

#[derive(Serialize)]
struct DayJson<'a> {
    /// `YYYY-MM-DD`, or `null` for the Unscheduled group.
    day: Option<&'a str>,
    today: bool,
    rows: Vec<RowJson<'a>>,
}

#[derive(Serialize)]
struct RowJson<'a> {
    id: &'a str,
    text: &'a str,
    list_id: &'a str,
    state: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    when: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    duration: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    deadline: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    place: Option<&'a Place>,
    /// Absent on an Unscheduled row: no field placed it.
    #[serde(skip_serializing_if = "Option::is_none")]
    placed_by: Option<PlacedBy>,
    tone: Tone,
}

fn row_json(row: &AgendaRow) -> RowJson<'_> {
    RowJson {
        id: &row.item.id,
        text: &row.item.text,
        list_id: &row.item.list_id,
        state: row.item.state.name(),
        when: row.item.when.as_deref(),
        duration: row.item.duration,
        deadline: row.item.deadline.as_deref(),
        place: row.item.place.as_ref(),
        placed_by: Some(row.placed_by),
        tone: row.tone,
    }
}

fn unscheduled_json(item: &ItemView) -> RowJson<'_> {
    RowJson {
        id: &item.id,
        text: &item.text,
        list_id: &item.list_id,
        state: item.state.name(),
        when: None,
        duration: None,
        deadline: None,
        place: item.place.as_ref(),
        placed_by: None,
        tone: Tone::Neutral,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use monoplan_core::{LIST_EVENTS, LIST_INBOX, WorkflowState};

    fn item(id: &str, when: Option<&str>, deadline: Option<&str>, created_at: i64) -> ItemView {
        ItemView {
            id: id.to_string(),
            text: id.to_string(),
            notes: String::new(),
            list_id: LIST_INBOX.to_string(),
            state: WorkflowState::Backlog,
            lifecycle_at: created_at,
            deadline: deadline.map(str::to_string),
            when: when.map(str::to_string),
            duration: None,
            place: None,
            created_at,
            started_at: None,
            done_at: None,
            binned_at: None,
        }
    }

    fn ids(day: &AgendaDay) -> Vec<&str> {
        day.rows.iter().map(|r| r.item.id.as_str()).collect()
    }

    const TODAY: &str = "2026-09-09";
    const HORIZON: &str = "2026-09-23";

    #[test]
    fn today_is_always_first_even_when_empty() {
        let days = build_agenda(&[item("a", Some("2026-09-12"), None, 1)], TODAY, HORIZON);
        assert_eq!(days.len(), 2);
        assert_eq!(days[0].day, TODAY);
        assert!(days[0].rows.is_empty());
        assert_eq!(days[1].day, "2026-09-12");
        assert_eq!(ids(&days[1]), vec!["a"]);
    }

    #[test]
    fn placement_is_earlier_field_clamped_to_today() {
        let items = [
            // past when → over, dropped
            item("past-when", Some("2026-09-01"), None, 1),
            // past deadline → Today, overdue
            item("overdue", None, Some("2026-09-05"), 2),
            // future when, past deadline → Today (owed now), overdue tone
            item("both-late", Some("2026-09-20"), Some("2026-09-08"), 3),
            // when Saturday, deadline later → placed by when
            item("both", Some("2026-09-12"), Some("2026-09-30"), 4),
            // deadline before when → placed by deadline
            item("dl-first", Some("2026-09-15"), Some("2026-09-11"), 5),
            // deadline today → Today, warning
            item("due-today", None, Some(TODAY), 6),
            // beyond horizon → dropped
            item("far", Some("2026-10-01"), None, 7),
            // undated → dropped
            item("undated", None, None, 8),
        ];
        let days = build_agenda(&items, TODAY, HORIZON);
        let by_day: Vec<(&str, Vec<&str>)> =
            days.iter().map(|d| (d.day.as_str(), ids(d))).collect();
        assert_eq!(
            by_day,
            vec![
                (TODAY, vec!["overdue", "both-late", "due-today"]),
                ("2026-09-11", vec!["dl-first"]),
                ("2026-09-12", vec!["both"]),
            ]
        );
        let today = &days[0];
        let tones: Vec<Tone> = today.rows.iter().map(|r| r.tone).collect();
        assert_eq!(tones, vec![Tone::Overdue, Tone::Overdue, Tone::Warning]);
        assert_eq!(days[1].rows[0].placed_by, PlacedBy::Deadline);
        assert_eq!(days[1].rows[0].tone, Tone::Neutral);
        assert_eq!(days[2].rows[0].placed_by, PlacedBy::When);
    }

    #[test]
    fn within_day_all_day_leads_timed_then_created_at() {
        let items = [
            item("t14", Some("2026-09-12T14:00"), None, 1),
            item("allday-late", Some("2026-09-12"), None, 9),
            item("allday-early", Some("2026-09-12"), None, 2),
            item("t09", Some("2026-09-12T09:00"), None, 3),
        ];
        let days = build_agenda(&items, TODAY, HORIZON);
        assert_eq!(
            ids(&days[1]),
            vec!["allday-early", "allday-late", "t09", "t14"]
        );
    }

    #[test]
    fn today_fold_orders_overdue_oldest_first_then_own() {
        let items = [
            item("own-timed", Some("2026-09-09T10:00"), None, 1),
            item("past-new", Some("2026-09-07T08:00"), None, 2),
            item("over-new", None, Some("2026-09-08"), 3),
            item("past-old", Some("2026-09-01"), None, 4),
            item("over-old", None, Some("2026-08-20"), 5),
            item("own-allday", Some(TODAY), None, 6),
        ];
        let days = build_agenda(&items, TODAY, HORIZON);
        assert_eq!(
            ids(&days[0]),
            vec!["over-old", "over-new", "own-allday", "own-timed"]
        );
    }

    #[test]
    fn past_when_places_nothing_but_a_deadline_still_does() {
        let items = [
            item("past-when", Some("2026-09-07T08:00"), None, 1),
            // past when, overdue deadline → Today by the deadline, overdue
            item("past-overdue", Some("2026-09-01"), Some("2026-09-02"), 2),
            // past when, deadline today → Today by the deadline, warning
            item("past-due-today", Some("2026-09-01"), Some(TODAY), 3),
            // past when, later deadline → the deadline's day
            item("past-due-later", Some("2026-09-01"), Some("2026-09-20"), 4),
        ];
        let days = build_agenda(&items, TODAY, HORIZON);
        let by_day: Vec<(&str, Vec<&str>)> =
            days.iter().map(|d| (d.day.as_str(), ids(d))).collect();
        assert_eq!(
            by_day,
            vec![
                (TODAY, vec!["past-overdue", "past-due-today"]),
                ("2026-09-20", vec!["past-due-later"]),
            ]
        );
        let rows: Vec<(PlacedBy, Tone)> = days
            .iter()
            .flat_map(|d| d.rows.iter().map(|r| (r.placed_by, r.tone)))
            .collect();
        assert_eq!(
            rows,
            vec![
                (PlacedBy::Deadline, Tone::Overdue),
                (PlacedBy::Deadline, Tone::Warning),
                (PlacedBy::Deadline, Tone::Neutral),
            ]
        );
    }

    #[test]
    fn done_rows_keep_their_when_day_and_slot() {
        let mut done_today = item("done-today", Some(TODAY), None, 1);
        done_today.state = WorkflowState::Done;
        let mut done_early = item("done-early", Some("2026-09-09T08:00"), None, 2);
        done_early.state = WorkflowState::Done;
        // Deadline is settled: places nothing even when earlier than `when`.
        let mut done_both = item("done-both", Some("2026-09-10"), Some("2026-09-05"), 3);
        done_both.state = WorkflowState::Done;
        // No `when`: nothing to show once done.
        let mut done_deadline = item("done-deadline", None, Some("2026-09-12"), 4);
        done_deadline.state = WorkflowState::Done;
        // Past `when`: not folded into Today, not shown.
        let mut done_past = item("done-past", Some("2026-09-01"), None, 5);
        done_past.state = WorkflowState::Done;
        let mut binned = item("binned", Some(TODAY), None, 6);
        binned.binned_at = Some(5);
        // Cancelled: dropped even with a future `when` — it did not happen.
        let mut cancelled = item("cancelled", Some("2026-09-10"), Some("2026-09-10"), 9);
        cancelled.state = WorkflowState::Cancelled;
        let items = [
            done_today,
            item("open-today", Some("2026-09-09T15:00"), None, 7),
            done_early,
            item("future", Some("2026-09-10"), None, 8),
            done_both,
            done_deadline,
            done_past,
            binned,
            cancelled,
        ];
        let days = build_agenda(&items, TODAY, HORIZON);
        let by_day: Vec<(&str, Vec<&str>)> =
            days.iter().map(|d| (d.day.as_str(), ids(d))).collect();
        assert_eq!(
            by_day,
            vec![
                (TODAY, vec!["done-today", "done-early", "open-today"]),
                ("2026-09-10", vec!["done-both", "future"]),
            ]
        );
        assert_eq!(days[1].rows[0].placed_by, PlacedBy::When);
        assert_eq!(days[1].rows[0].tone, Tone::Neutral);
    }

    #[test]
    fn events_place_by_the_same_rules_as_tasks() {
        let mut event = item("event", Some("2026-09-12T19:00"), None, 1);
        event.list_id = LIST_EVENTS.to_string();
        // A deadline on an event is legitimate and places it as usual.
        let mut tickets = item("tickets", None, Some("2026-09-10"), 2);
        tickets.list_id = LIST_EVENTS.to_string();
        // A past event went by.
        let mut past = item("past", Some("2026-09-01"), None, 3);
        past.list_id = LIST_EVENTS.to_string();
        let items = [event, tickets, past];
        let days = build_agenda(&items, TODAY, HORIZON);
        let by_day: Vec<(&str, Vec<&str>)> =
            days.iter().map(|d| (d.day.as_str(), ids(d))).collect();
        assert_eq!(
            by_day,
            vec![
                (TODAY, vec![]),
                ("2026-09-10", vec!["tickets"]),
                ("2026-09-12", vec!["event"]),
            ]
        );
        assert!(unscheduled_events(&items).is_empty());
    }

    #[test]
    fn unscheduled_is_open_undated_events_oldest_first() {
        let event = |id: &str, when: Option<&str>, deadline: Option<&str>, at: i64| {
            let mut i = item(id, when, deadline, at);
            i.list_id = LIST_EVENTS.to_string();
            i
        };
        let mut done = event("done", None, None, 1);
        done.state = WorkflowState::Done;
        let mut cancelled = event("cancelled", None, None, 2);
        cancelled.state = WorkflowState::Cancelled;
        let mut binned = event("binned", None, None, 3);
        binned.binned_at = Some(5);
        let items = [
            event("newer", None, None, 9),
            event("older", None, None, 4),
            // Dated either way: has a day, or went by.
            event("dated", Some("2026-09-12"), None, 5),
            event("due", None, Some("2026-09-12"), 6),
            event("past", Some("2026-09-01"), None, 7),
            // An undated task is just a task; its list shows it.
            item("task", None, None, 8),
            done,
            cancelled,
            binned,
        ];
        let got: Vec<&str> = unscheduled_events(&items)
            .iter()
            .map(|i| i.id.as_str())
            .collect();
        assert_eq!(got, vec!["older", "newer"]);
    }

    #[test]
    fn binned_are_excluded() {
        let mut binned = item("binned", Some(TODAY), None, 2);
        binned.binned_at = Some(5);
        let days = build_agenda(&[binned], TODAY, HORIZON);
        assert_eq!(days.len(), 1);
        assert!(days[0].rows.is_empty());
    }
}
