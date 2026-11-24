//! `monoplan agenda`: dated items by day, Today first, then the coming
//! days (`spec/calendar-plan.md` "Agenda"). An Open task whose `when`
//! has passed is still to do and folds into Today as overdue; an
//! event's past `when` has gone by and places nothing.
//! A done task is off the agenda (`spec/calendar-plan.md` "Completed
//! today"). A done event keeps its `when` day and slot (it happened
//! then, whatever was ticked). Events (`spec/events-plan.md`) otherwise place and
//! print by the same rules; an event always has a `when`, so it always
//! has a day.
//!
//! Placement, tone, and ordering are pure functions of the item views
//! and a `today` stamp, so they are unit-tested here without a doc.
//!
//! `--all` merges every workspace this install can open into one
//! agenda (the daemon runs it over its open docs, `spec/cli.md`
//! §Agenda): the same placement over every doc's items at once, each
//! row tagged with its workspace, and the Home doc's
//! `calendar_hidden_docs` left out as on the web calendar.

use std::io::Write;

use chrono::{Datelike, Days, NaiveDate};
use clap::Parser;
use monoplan_core::ItemView;
use serde::{Deserialize, Serialize};

use super::items::{date_tags, row_box, write_json};
use crate::api::{
    self, Ctx,
    agenda::{AgendaQuery, AgendaTone as Tone},
};
use crate::workspace::{Candidate, HOME_LABEL};

#[derive(Parser, Debug, Clone, Serialize, Deserialize)]
pub struct AgendaArgs {
    /// How many days past today to show (Today is always shown).
    #[arg(long, default_value_t = 14)]
    pub days: u32,
    /// How many days before today to show: the days that have gone by
    /// (the events that happened on them). 0, the default, shows none.
    #[arg(long, default_value_t = 0)]
    #[serde(default, skip_serializing_if = "is_zero")]
    pub past: u32,
    /// Override the local date used as "today" (`YYYY-MM-DD`).
    #[arg(long)]
    pub today: Option<String>,
    /// Machine-parseable output.
    #[arg(long)]
    pub json: bool,
    /// Every workspace this install can open, merged; rows carry their
    /// workspace. Takes no `-w`. Workspaces hidden from the calendar
    /// (the Home doc's `calendar_hidden_docs`) are left out.
    #[arg(long)]
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub all: bool,
}

/// Keeps `past` off the wire when unset, as `all` is.
fn is_zero(n: &u32) -> bool {
    *n == 0
}

fn tone_label(tone: Tone) -> Option<&'static str> {
    match tone {
        Tone::Neutral => None,
        Tone::Overdue => Some("overdue"),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgendaRow {
    pub item: ItemView,
    pub tone: Tone,
    /// The row's workspace on a merged agenda; `None` on a single-doc
    /// one, where every row is the selected workspace's.
    pub doc: Option<Candidate>,
}

/// One workspace's items for [`build_agenda_from`]: the tag every row
/// from it carries (`None` tags nothing) and its item views.
pub type Source<'a> = (Option<&'a Candidate>, &'a [ItemView]);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgendaDay {
    /// `YYYY-MM-DD`.
    pub day: String,
    pub rows: Vec<AgendaRow>,
}

/// Group dated items by placement day. `floor`, `today` and `horizon`
/// are `YYYY-MM-DD`; days before `floor` and after `horizon` are
/// dropped. Today is always a group, even when empty, and the first
/// one when `floor` is today; other empty days are skipped. Groups are
/// in date order, so the days gone by come ahead of Today.
///
/// - Placement: the `when` puts the item on its day; an Open task (not
///   an event) whose `when` is over, its whole span ended before today,
///   lands in Today instead. An event's past `when` places nothing.
/// - Tone: overdue for a task whose `when` is over, else neutral.
/// - Within a day: overdue rows (oldest first), then the day's own rows
///   in three bands ([`day_band`]): all-day events, timed rows by the
///   clock, then untimed tasks. Inside a band, the raw `when`, then
///   `created_at`.
/// - A done task (anything done but an event with a `when`) is off the
///   agenda, done today or not: the Done view is its record. Mirrors
///   `groupByDay` in the web's `dayGroups.ts`.
/// - A done event places on its `when` day, neutral, where it was:
///   ticking it does not move it.
///   Cancelled and binned items never appear: a cancelled slot is
///   something that did not happen, so it clutters rather than records.
/// - Days gone by (`floor` before today): a day before today holds the
///   events whose `when` is on it, done or not, neutral, in the day's
///   usual order. Nothing else lands there: an Open task with a past
///   `when` is owed and stays in Today's overdue fold, and a done task
///   is off the agenda.
///
/// [`build_agenda`] is the agenda from today on (`floor` = today).
pub fn build_agenda(items: &[ItemView], today: &str, horizon: &str) -> Vec<AgendaDay> {
    build_agenda_from(&[(None, items)], today, today, horizon)
}

/// [`build_agenda`] over several workspaces at once: one placement and
/// one order across all of them (a stable sort, so equal keys keep
/// source order), each row tagged with its source.
pub fn build_agenda_from(
    sources: &[Source<'_>],
    floor: &str,
    today: &str,
    horizon: &str,
) -> Vec<AgendaDay> {
    // (day, fold group, day band, placing raw, created_at, row)
    let mut placed: Vec<(String, u8, u8, String, i64, AgendaRow)> = Vec::new();
    for (doc, items) in sources {
        for item in *items {
            if item.is_binned() || item.is_cancelled() {
                continue;
            }
            // A done task is off the agenda: the Done view has it.
            if item.is_done() && !(item.is_event() && item.when.is_some()) {
                continue;
            }
            // A day gone by: what happened on it, an event. The rest of
            // the placement is from today on.
            if let Some(w) = item.when.as_deref() {
                let day = &w[..w.len().min(10)];
                if day < today && day >= floor && item.is_event() {
                    placed.push((
                        day.to_string(),
                        1,
                        day_band(item),
                        w.to_string(),
                        item.created_at,
                        AgendaRow {
                            item: item.clone(),
                            tone: Tone::Neutral,
                            doc: doc.cloned(),
                        },
                    ));
                }
            }
            // From today on a past `when` places nothing on its own
            // day; a task's lands in Today's overdue fold.
            let Some(when) = item.when.as_deref() else {
                continue;
            };
            let when_day = &when[..when.len().min(10)];
            // Planned for a day that has passed and still Open: overdue.
            let slipped = !item.is_done()
                && !item.is_event()
                && when_end_day(when, item.duration).as_str() < today;
            let day = if slipped {
                today
            } else if when_day >= today {
                when_day
            } else {
                continue;
            };
            if day > horizon {
                continue;
            }
            // Today's fold: overdue rows lead (by their own date, no
            // bands), then today's own rows, done or not.
            let (group, band, tone) = if slipped {
                (0, 0, Tone::Overdue)
            } else {
                (1, day_band(item), Tone::Neutral)
            };
            placed.push((
                day.to_string(),
                group,
                band,
                when.to_string(),
                item.created_at,
                AgendaRow {
                    item: item.clone(),
                    tone,
                    doc: doc.cloned(),
                },
            ));
        }
    }
    placed.sort_by(|a, b| {
        a.0.cmp(&b.0)
            .then(a.1.cmp(&b.1))
            .then(a.2.cmp(&b.2))
            .then(a.3.cmp(&b.3))
            .then(a.4.cmp(&b.4))
    });

    let mut out: Vec<AgendaDay> = Vec::new();
    for (day, _, _, _, _, row) in placed {
        match out.last_mut() {
            Some(last) if last.day == day => last.rows.push(row),
            _ => out.push(AgendaDay {
                day,
                rows: vec![row],
            }),
        }
    }
    // Today is always there, in its place among the days.
    if !out.iter().any(|d| d.day == today) {
        let at = out.partition_point(|d| d.day.as_str() < today);
        out.insert(
            at,
            AgendaDay {
                day: today.to_string(),
                rows: Vec::new(),
            },
        );
    }
    out
}

/// Where a row sits among its day's own rows (`spec/calendar-plan.md`
/// "Within a day"): all-day events lead (0), they are the day's
/// backdrop; then what has a clock time, tasks and events
/// alike (1); then untimed tasks, for the day at no particular time (2).
/// Mirrors `dayBand` in the web's `dayGroups.ts`.
fn day_band(item: &ItemView) -> u8 {
    match item.when.as_deref() {
        Some(w) if w.len() > 10 => 1,
        _ if item.is_event() => 0,
        _ => 2,
    }
}

/// The last day a `when` covers, as `YYYY-MM-DD`: its start day, pushed
/// on by the `duration` beside it (whole days beside an all-day start,
/// the clock end beside a timed one). The end is exclusive: a timed
/// span ending at 00:00 exactly stops on the day before. Mirrors
/// `whenSpan` in the web's `dayGroups.ts`. A value that does not parse
/// yields its own day.
fn when_end_day(when: &str, duration: Option<u32>) -> String {
    let day = &when[..when.len().min(10)];
    let minutes = u64::from(duration.unwrap_or(0));
    let offset = match (when.get(11..13), when.get(14..16)) {
        (Some(h), Some(m)) => match (h.parse::<u64>(), m.parse::<u64>()) {
            (Ok(h), Ok(m)) => (h * 60 + m + minutes).saturating_sub(1) / 1440,
            _ => 0,
        },
        _ => minutes.div_ceil(1440).saturating_sub(1),
    };
    NaiveDate::parse_from_str(day, "%Y-%m-%d")
        .ok()
        .and_then(|d| d.checked_add_days(Days::new(offset)))
        .map(|d| d.format("%Y-%m-%d").to_string())
        .unwrap_or_else(|| day.to_string())
}

/// The Calendar nav entry's count: what is owed or on today. Open
/// items with a task `when` that is over, or a `when` on today (a span
/// covering it included), each counted once. An event's past `when` has gone by and counts
/// for nothing; days ahead are the view's business. Mirrors
/// `attentionBadge` in the web's `dayGroups.ts`.
pub fn attention_count(items: &[ItemView], today: &str) -> u32 {
    let mut count = 0;
    for item in items {
        if !item.is_open() {
            continue;
        }
        let span = item
            .when
            .as_deref()
            .map(|w| (&w[..w.len().min(10)], when_end_day(w, item.duration)));
        let slipped =
            !item.is_event() && span.as_ref().is_some_and(|(_, end)| end.as_str() < today);
        let on_today = span
            .as_ref()
            .is_some_and(|(start, end)| *start <= today && today <= end.as_str());
        if slipped || on_today {
            count += 1;
        }
    }
    count
}

/// `agenda`, and with `--all` one agenda over every workspace the
/// install has open (`api::agenda::place`). Rows of the merged one
/// carry their workspace.
pub fn run(cx: &Ctx, args: AgendaArgs, out: &mut dyn Write) -> anyhow::Result<()> {
    let placed = api::agenda::place(
        cx,
        &AgendaQuery {
            days: args.days,
            past_days: args.past,
            today: args.today.as_deref(),
            everywhere: args.all,
        },
    )?;
    if args.json {
        write_json(out, &api::agenda::groups(&placed))
    } else {
        print_agenda(out, &placed.days, &placed.today)
    }
}

/// `  [Work]` after a merged row, `  [Personal]` for Home; nothing on
/// a single-doc agenda.
fn doc_tag(doc: Option<&Candidate>) -> String {
    doc.map(|c| format!("  [{}]", c.name.as_deref().unwrap_or(HOME_LABEL)))
        .unwrap_or_default()
}

fn print_agenda(out: &mut dyn Write, days: &[AgendaDay], today: &str) -> anyhow::Result<()> {
    for (i, day) in days.iter().enumerate() {
        if i > 0 {
            writeln!(out)?;
        }
        let weekday = NaiveDate::parse_from_str(&day.day, "%Y-%m-%d")
            .map(|d| d.weekday().to_string())
            .unwrap_or_default();
        if day.day == today {
            writeln!(out, "Today  {}  {weekday}", day.day)?;
        } else {
            writeln!(out, "{}  {weekday}", day.day)?;
        }
        if day.rows.is_empty() {
            writeln!(out, "  (nothing)")?;
        }
        for row in &day.rows {
            let tone = tone_label(row.tone)
                .map(|l| format!("  ({l})"))
                .unwrap_or_default();
            writeln!(
                out,
                "  {}  {} {}{}{}{tone}",
                row.item.id,
                row_box(&row.item),
                row.item.text,
                date_tags(&row.item),
                doc_tag(row.doc.as_ref())
            )?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use monoplan_core::{LIST_INBOX, WorkflowState};

    fn item(id: &str, when: Option<&str>, created_at: i64) -> ItemView {
        ItemView {
            id: id.to_string(),
            text: id.to_string(),
            notes: String::new(),
            list_id: LIST_INBOX.to_string(),
            state: WorkflowState::Backlog,
            lifecycle_at: created_at,
            when: when.map(str::to_string),
            when_zoned: None,
            when_suggested: None,
            event: false,
            duration: None,
            place: None,
            created_at,
            created_by: None,
            assignee: None,
            subtasks: Vec::new(),
            started_at: None,
            done_at: None,
            binned_at: None,
            inbox_at: None,
        }
    }

    fn ids(day: &AgendaDay) -> Vec<&str> {
        day.rows.iter().map(|r| r.item.id.as_str()).collect()
    }

    const TODAY: &str = "2026-09-09";
    const HORIZON: &str = "2026-09-23";

    #[test]
    fn attention_counts_what_is_owed_or_on_today_once() {
        let mut gone_event = item("gone-event", Some("2026-09-01"), 1);
        gone_event.event = true;
        let mut done = item("done", Some(TODAY), 2);
        done.state = WorkflowState::Done;
        let mut binned = item("binned", Some(TODAY), 3);
        binned.binned_at = Some(1);
        let mut span = item("span", Some("2026-09-08"), 4);
        span.duration = Some(3 * 1440);
        let items = [
            item("slipped", Some("2026-09-02"), 6),
            item("on-today", Some("2026-09-09T10:00"), 8),
            item("all-day-today", Some(TODAY), 9),
            span,
            item("ahead", Some("2026-09-12"), 10),
            item("undated", None, 11),
            gone_event,
            done,
            binned,
        ];
        assert_eq!(attention_count(&items, TODAY), 4);
    }

    #[test]
    fn merged_sources_share_one_order_and_carry_their_tag() {
        use monoplan_core::DocId;
        let home = Candidate {
            doc_id: DocId(uuid::Uuid::from_u128(1)),
            name: None,
        };
        let work = Candidate {
            doc_id: DocId(uuid::Uuid::from_u128(2)),
            name: Some("Work".into()),
        };
        let home_items = [
            item("h-late", Some("2026-09-12T15:00"), 1),
            item("h-over", Some("2026-09-01"), 2),
        ];
        let work_items = [
            item("w-early", Some("2026-09-12T09:00"), 3),
            item("w-today", Some(TODAY), 4),
        ];
        let days = build_agenda_from(
            &[(Some(&home), &home_items), (Some(&work), &work_items)],
            TODAY,
            TODAY,
            HORIZON,
        );
        // Today: Home's slipped task leads, then Work's own row;
        // Saturday interleaves the two docs by time.
        assert_eq!(ids(&days[0]), vec!["h-over", "w-today"]);
        assert_eq!(ids(&days[1]), vec!["w-early", "h-late"]);
        assert_eq!(days[0].rows[0].doc.as_ref(), Some(&home));
        assert_eq!(days[1].rows[0].doc.as_ref(), Some(&work));
        // The single-doc path tags nothing.
        let alone = build_agenda(&work_items, TODAY, HORIZON);
        assert!(alone[1].rows.iter().all(|r| r.doc.is_none()));
    }

    #[test]
    fn today_is_always_first_even_when_empty() {
        let days = build_agenda(&[item("a", Some("2026-09-12"), 1)], TODAY, HORIZON);
        assert_eq!(days.len(), 2);
        assert_eq!(days[0].day, TODAY);
        assert!(days[0].rows.is_empty());
        assert_eq!(days[1].day, "2026-09-12");
        assert_eq!(ids(&days[1]), vec!["a"]);
    }

    #[test]
    fn the_when_places_the_row_and_a_past_task_folds_into_today() {
        let items = [
            // past when on a task → Today, overdue
            item("past-when", Some("2026-09-01"), 1),
            item("sat", Some("2026-09-12"), 4),
            // beyond horizon → dropped
            item("far", Some("2026-10-01"), 7),
            // undated → dropped
            item("undated", None, 8),
        ];
        let days = build_agenda(&items, TODAY, HORIZON);
        let by_day: Vec<(&str, Vec<&str>)> =
            days.iter().map(|d| (d.day.as_str(), ids(d))).collect();
        assert_eq!(
            by_day,
            vec![(TODAY, vec!["past-when"]), ("2026-09-12", vec!["sat"])]
        );
        assert_eq!(days[0].rows[0].tone, Tone::Overdue);
        assert_eq!(days[1].rows[0].tone, Tone::Neutral);
    }

    #[test]
    fn within_day_all_day_events_then_timed_then_untimed() {
        let mut birthday = item("birthday", Some("2026-09-12"), 8);
        birthday.event = true;
        let mut gig09 = item("gig09", Some("2026-09-12T09:00"), 7);
        gig09.event = true;
        let items = [
            item("t14", Some("2026-09-12T14:00"), 1),
            item("task-late", Some("2026-09-12"), 9),
            item("task-early", Some("2026-09-12"), 2),
            birthday,
            gig09,
        ];
        let days = build_agenda(&items, TODAY, HORIZON);
        // Inside a band: raw string (one day here), then `created_at`.
        assert_eq!(
            ids(&days[1]),
            vec!["birthday", "gig09", "t14", "task-early", "task-late"]
        );
    }

    #[test]
    fn today_fold_orders_overdue_oldest_first_then_own() {
        let items = [
            item("own-timed", Some("2026-09-09T10:00"), 1),
            item("past-new", Some("2026-09-07T08:00"), 2),
            item("past-old", Some("2026-09-01"), 4),
            item("own-allday", Some(TODAY), 6),
        ];
        let days = build_agenda(&items, TODAY, HORIZON);
        assert_eq!(
            ids(&days[0]),
            vec!["past-old", "past-new", "own-timed", "own-allday"]
        );
    }

    #[test]
    fn a_span_still_running_is_not_overdue() {
        // Started yesterday, runs three days: under way, not slipped.
        let mut running = item("running", Some("2026-09-08"), 5);
        running.duration = Some(3 * 1440);
        // Ended yesterday.
        let mut span_over = item("span-over", Some("2026-09-06"), 6);
        span_over.duration = Some(3 * 1440);
        // Overnight: 23:00 + 2h ends today.
        let mut overnight = item("overnight", Some("2026-09-08T23:00"), 7);
        overnight.duration = Some(120);
        let items = [
            item("past-when", Some("2026-09-07T08:00"), 1),
            running,
            span_over,
            overnight,
        ];
        let days = build_agenda(&items, TODAY, HORIZON);
        let by_day: Vec<(&str, Vec<&str>)> =
            days.iter().map(|d| (d.day.as_str(), ids(d))).collect();
        assert_eq!(by_day, vec![(TODAY, vec!["span-over", "past-when"])]);
        assert!(days[0].rows.iter().all(|r| r.tone == Tone::Overdue));
    }

    #[test]
    fn a_timed_span_ending_at_midnight_stops_on_the_day_before() {
        assert_eq!(when_end_day("2026-09-10T21:00", Some(180)), "2026-09-10");
        assert_eq!(when_end_day("2026-09-10T21:00", Some(181)), "2026-09-11");
        assert_eq!(when_end_day("2026-09-10T23:00", Some(120)), "2026-09-11");
        assert_eq!(when_end_day("2026-09-10T00:00", Some(1440)), "2026-09-10");
        assert_eq!(when_end_day("2026-09-10T00:00", None), "2026-09-10");
        assert_eq!(when_end_day("2026-09-10", Some(3 * 1440)), "2026-09-12");
    }

    #[test]
    fn a_done_task_is_off_the_agenda_and_a_done_event_stays() {
        let done = |id: &str, when: Option<&str>, at: i64| {
            let mut it = item(id, when, at);
            it.state = WorkflowState::Done;
            it.lifecycle_at = at;
            it
        };
        // Done today or before: none is placed.
        let done_today = done("done-today", Some(TODAY), 1_300);
        let done_early = done("done-early", Some("2026-09-09T08:00"), 1_100);
        let done_ahead = done("done-ahead", Some("2026-09-10"), 1_200);
        // A done event stays on its `when`.
        let mut done_event = done("done-event", Some("2026-09-10"), 1_500);
        done_event.event = true;
        let mut binned = item("binned", Some(TODAY), 6);
        binned.binned_at = Some(5);
        // Cancelled: dropped even with a future `when` — it did not happen.
        let mut cancelled = item("cancelled", Some("2026-09-10"), 9);
        cancelled.state = WorkflowState::Cancelled;
        let items = [
            done_today,
            item("open-today", Some("2026-09-09T15:00"), 7),
            done_early,
            item("future", Some("2026-09-10"), 8),
            done_ahead,
            done_event,
            binned,
            cancelled,
        ];
        let days = build_agenda(&items, TODAY, HORIZON);
        let by_day: Vec<(&str, Vec<&str>)> =
            days.iter().map(|d| (d.day.as_str(), ids(d))).collect();
        assert_eq!(
            by_day,
            vec![
                (TODAY, vec!["open-today"]),
                ("2026-09-10", vec!["done-event", "future"]),
            ]
        );
    }

    #[test]
    fn events_place_by_the_same_rules_as_tasks_and_go_by() {
        let mut event = item("event", Some("2026-09-12T19:00"), 1);
        event.event = true;
        // A past event went by; a past task is overdue.
        let mut past = item("past", Some("2026-09-01"), 3);
        past.event = true;
        let slipped = item("slipped", Some("2026-09-01"), 4);
        let items = [event, past, slipped];
        let days = build_agenda(&items, TODAY, HORIZON);
        let by_day: Vec<(&str, Vec<&str>)> =
            days.iter().map(|d| (d.day.as_str(), ids(d))).collect();
        assert_eq!(
            by_day,
            vec![(TODAY, vec!["slipped"]), ("2026-09-12", vec!["event"])]
        );
    }

    #[test]
    fn days_gone_by_hold_events_from_the_floor() {
        let mut gig = item("gig", Some("2026-09-01T19:00"), 1);
        gig.event = true;
        let mut done = item("done", Some("2026-09-01"), 2);
        done.state = WorkflowState::Done;
        // Still owed: Today's overdue fold only, not its own day too.
        let slipped = item("slipped", Some("2026-09-01"), 3);
        // Before the floor: dropped.
        let mut ancient = item("ancient", Some("2026-08-01"), 6);
        ancient.event = true;
        let mut cancelled = item("cancelled", Some("2026-09-01"), 7);
        cancelled.state = WorkflowState::Cancelled;
        let mut binned = item("binned", Some("2026-09-01"), 8);
        binned.event = true;
        binned.binned_at = Some(5);
        let items = [gig, done, slipped, ancient, cancelled, binned];
        let days = build_agenda_from(&[(None, &items)], "2026-08-26", TODAY, HORIZON);
        let by_day: Vec<(&str, Vec<&str>)> =
            days.iter().map(|d| (d.day.as_str(), ids(d))).collect();
        assert_eq!(
            by_day,
            vec![("2026-09-01", vec!["gig"]), (TODAY, vec!["slipped"])]
        );
        assert_eq!(days[0].rows[0].tone, Tone::Neutral);
        // An empty Today still takes its place after the days gone by,
        // and no floor means none of them.
        let days = build_agenda_from(&[(None, &items[..1])], "2026-08-26", TODAY, HORIZON);
        let order: Vec<&str> = days.iter().map(|d| d.day.as_str()).collect();
        assert_eq!(order, vec!["2026-09-01", TODAY]);
        assert!(days[1].rows.is_empty());
        let days = build_agenda(&items[..1], TODAY, HORIZON);
        assert_eq!(days.len(), 1);
    }

    #[test]
    fn binned_are_excluded() {
        let mut binned = item("binned", Some(TODAY), 2);
        binned.binned_at = Some(5);
        let days = build_agenda(&[binned], TODAY, HORIZON);
        assert_eq!(days.len(), 1);
        assert!(days[0].rows.is_empty());
    }
}
