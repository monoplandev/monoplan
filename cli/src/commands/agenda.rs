//! `monoplan agenda`: dated items by day, Today first with overdue
//! deadlines folded in, then the coming days (`spec/calendar-plan.md`
//! "Agenda"). A deadline is a row of its own on its day, beside the
//! row the item's `when` gives it. An Open task whose `when` has passed is still to do and
//! folds into Today as overdue too; an event's past `when` has gone by
//! and places nothing.
//! Done items keep their `when` day and slot (the tick does not erase
//! "happens on"); their deadline is settled and places nothing. Events
//! (`spec/events-plan.md`) place and print by the same rules; an event
//! always has a `when`, so it always has a day.
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
    agenda::{AgendaPlacedBy as PlacedBy, AgendaQuery, AgendaTone as Tone},
};
use crate::workspace::{Candidate, HOME_LABEL};

#[derive(Parser, Debug, Clone, Serialize, Deserialize)]
pub struct AgendaArgs {
    /// How many days past today to show (Today is always shown).
    #[arg(long, default_value_t = 14)]
    pub days: u32,
    /// How many days before today to show: the days that have gone by
    /// (events that happened, done items on the day they were planned
    /// for). 0, the default, shows none.
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
        Tone::Warning => Some("due today"),
        Tone::Overdue => Some("overdue"),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgendaRow {
    pub item: ItemView,
    pub placed_by: PlacedBy,
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
/// - Placement: the two dates place independently, so an Open item
///   with both is on two days. The `when` puts it on its day; an Open
///   task (not an event) whose `when` is over, its whole span ended
///   before today, lands in Today instead. The deadline puts a row of
///   its own (`placed_by: deadline`) on its day, clamped up to today, so
///   an overdue deadline lands in Today. An event's past `when` places
///   nothing: with only that it is dropped, and with a deadline too it
///   has the deadline's row alone.
/// - One row per item per day: where both land on the same day the row
///   is the `when`'s, toned by the more urgent of the two; with both
///   dates behind us it is the deadline's, which is what is owed.
/// - Tone: overdue (deadline < today on the deadline's row, a task's
///   `when` over on the `when`'s), warning (deadline = today), else
///   neutral.
/// - Within a day: overdue rows (oldest first, by the date that made
///   them overdue), then the day's own rows in three bands
///   ([`day_band`]): all-day events, timed rows by the clock, then
///   untimed tasks and deadline rows. Inside a band, the raw string of
///   the placing field, then `created_at`. Ticking a row does not move
///   it.
/// - Done items place by `when` only, on that day (today or later),
///   neutral. A done item with no `when`, or a past `when`, is dropped:
///   nothing is owed and past days are not shown. Cancelled and binned
///   items never appear: a cancelled slot is something that did not
///   happen, so it clutters rather than records.
/// - Days gone by (`floor` before today): a day before today holds what
///   has a `when` on it and is settled, an event (it happened) or a
///   done item (on the day it was planned for), neutral, in the day's
///   usual order. Nothing else lands there: an Open task with a past
///   `when` is owed and stays in Today's overdue fold, and a deadline
///   never places in the past. An event with a past `when` and a
///   deadline still to come is on both days.
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
            // A day gone by: what happened on it (an event) or was done
            // for it. The rest of the placement is from today on.
            if let Some(w) = item.when.as_deref() {
                let day = &w[..w.len().min(10)];
                if day < today && day >= floor && (item.is_event() || item.is_done()) {
                    placed.push((
                        day.to_string(),
                        1,
                        day_band(item, PlacedBy::When),
                        w.to_string(),
                        item.created_at,
                        AgendaRow {
                            item: item.clone(),
                            placed_by: PlacedBy::When,
                            tone: Tone::Neutral,
                            doc: doc.cloned(),
                        },
                    ));
                }
            }
            // From today on a past `when` places nothing on its own
            // day; a task's lands in Today's overdue fold below.
            let when_day = item
                .when
                .as_deref()
                .map(|w| &w[..w.len().min(10)])
                .filter(|w| *w >= today);
            let open = !item.is_done();
            // Planned for a day that has passed and still Open: overdue.
            let slipped = open
                && !item.is_event()
                && item
                    .when
                    .as_deref()
                    .is_some_and(|w| when_end_day(w, item.duration).as_str() < today);
            // The two dates place independently. The `when` row: its
            // day from today on, done or not, or Today for a slipped
            // task.
            let when_row = if slipped { Some(today) } else { when_day };
            // The deadline's own row, Open items only (Done settles
            // it): its day, an overdue one clamped up to Today.
            let deadline_day = item.deadline.as_deref().filter(|_| open);
            let deadline_overdue = deadline_day.is_some_and(|d| d < today);
            let deadline_row = deadline_day.map(|d| if d < today { today } else { d });
            let when_tone = if slipped {
                Tone::Overdue
            } else {
                Tone::Neutral
            };
            let deadline_tone = if deadline_overdue {
                Tone::Overdue
            } else if deadline_day == Some(today) {
                Tone::Warning
            } else {
                Tone::Neutral
            };
            // (day, placing field, tone, the raw that orders the row)
            let mut rows: Vec<(&str, PlacedBy, Tone, &str)> = Vec::new();
            match (when_row, deadline_row) {
                // Both on one day: one row. It is the `when`'s, toned
                // by the more urgent of the two, unless both dates are
                // behind us: then the deadline's, it is what is owed.
                (Some(w), Some(d)) if w == d => {
                    let placed_by = if slipped && deadline_overdue {
                        PlacedBy::Deadline
                    } else {
                        PlacedBy::When
                    };
                    let raw = if deadline_overdue {
                        deadline_day.unwrap()
                    } else {
                        item.when.as_deref().unwrap()
                    };
                    rows.push((w, placed_by, when_tone.max(deadline_tone), raw));
                }
                (w, d) => {
                    if let Some(w) = w {
                        rows.push((w, PlacedBy::When, when_tone, item.when.as_deref().unwrap()));
                    }
                    if let Some(d) = d {
                        rows.push((d, PlacedBy::Deadline, deadline_tone, deadline_day.unwrap()));
                    }
                }
            }
            for (day, placed_by, tone, raw) in rows {
                if day > horizon {
                    continue;
                }
                // Today's fold: overdue rows lead (by their own date),
                // then today's own rows, done or not.
                // The fold is by date alone: no bands in it.
                let (group, band) = if tone == Tone::Overdue {
                    (0, 0)
                } else {
                    (1, day_band(item, placed_by))
                };
                placed.push((
                    day.to_string(),
                    group,
                    band,
                    raw.to_string(),
                    item.created_at,
                    AgendaRow {
                        item: item.clone(),
                        placed_by,
                        tone,
                        doc: doc.cloned(),
                    },
                ));
            }
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
/// backdrop; then what has a clock time, tasks and events alike (1);
/// then what is for the day at no particular time, an untimed task or
/// a deadline's row (2). Mirrors `dayBand` in the web's `dayGroups.ts`.
fn day_band(item: &ItemView, placed_by: PlacedBy) -> u8 {
    match (placed_by, item.when.as_deref()) {
        (PlacedBy::When, Some(w)) if w.len() > 10 => 1,
        (PlacedBy::When, _) if item.is_event() => 0,
        _ => 2,
    }
}

/// The last day a `when` covers, as `YYYY-MM-DD`: its start day, pushed
/// on by the `duration` beside it (whole days beside an all-day start,
/// the clock end beside a timed one). Mirrors `whenSpan` in the web's
/// `dayGroups.ts`. A value that does not parse yields its own day.
fn when_end_day(when: &str, duration: Option<u32>) -> String {
    let day = &when[..when.len().min(10)];
    let minutes = u64::from(duration.unwrap_or(0));
    let offset = match (when.get(11..13), when.get(14..16)) {
        (Some(h), Some(m)) => match (h.parse::<u64>(), m.parse::<u64>()) {
            (Ok(h), Ok(m)) => (h * 60 + m + minutes) / 1440,
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
            when_suggested: None,
            event: false,
            duration: None,
            place: None,
            created_at,
            created_by: None,
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
            item("h-late", Some("2026-09-12T15:00"), None, 1),
            item("h-over", None, Some("2026-09-01"), 2),
        ];
        let work_items = [
            item("w-early", Some("2026-09-12T09:00"), None, 3),
            item("w-today", Some(TODAY), None, 4),
        ];
        let days = build_agenda_from(
            &[(Some(&home), &home_items), (Some(&work), &work_items)],
            TODAY,
            TODAY,
            HORIZON,
        );
        // Today: Home's overdue deadline leads, then Work's own row;
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
        let days = build_agenda(&[item("a", Some("2026-09-12"), None, 1)], TODAY, HORIZON);
        assert_eq!(days.len(), 2);
        assert_eq!(days[0].day, TODAY);
        assert!(days[0].rows.is_empty());
        assert_eq!(days[1].day, "2026-09-12");
        assert_eq!(ids(&days[1]), vec!["a"]);
    }

    #[test]
    fn each_date_places_its_own_row_clamped_to_today() {
        let items = [
            // past when on a task → Today, overdue
            item("past-when", Some("2026-09-01"), None, 1),
            // past deadline → Today, overdue
            item("overdue", None, Some("2026-09-05"), 2),
            // future when, past deadline → Today (owed now), overdue
            // tone, and its own day besides
            item("both-late", Some("2026-09-20"), Some("2026-09-08"), 3),
            // when Saturday, deadline past the horizon → the when's row
            item("both", Some("2026-09-12"), Some("2026-09-30"), 4),
            // deadline before when → a row for each
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
                (
                    TODAY,
                    vec!["past-when", "overdue", "both-late", "due-today"]
                ),
                ("2026-09-11", vec!["dl-first"]),
                ("2026-09-12", vec!["both"]),
                ("2026-09-15", vec!["dl-first"]),
                ("2026-09-20", vec!["both-late"]),
            ]
        );
        let today = &days[0];
        let tones: Vec<Tone> = today.rows.iter().map(|r| r.tone).collect();
        assert_eq!(
            tones,
            vec![Tone::Overdue, Tone::Overdue, Tone::Overdue, Tone::Warning]
        );
        assert_eq!(today.rows[0].placed_by, PlacedBy::When);
        assert_eq!(days[1].rows[0].placed_by, PlacedBy::Deadline);
        assert_eq!(days[1].rows[0].tone, Tone::Neutral);
        assert_eq!(days[2].rows[0].placed_by, PlacedBy::When);
        // The `when` rows of the two that also have a deadline row.
        for day in &days[3..] {
            assert_eq!(day.rows[0].placed_by, PlacedBy::When);
            assert_eq!(day.rows[0].tone, Tone::Neutral);
        }
        assert_eq!(today.rows[2].placed_by, PlacedBy::Deadline);
    }

    #[test]
    fn both_dates_on_one_day_are_one_row() {
        let items = [
            // same day ahead → the when's row, neutral
            item("same-day", Some("2026-09-12T10:00"), Some("2026-09-12"), 1),
            // both today → the when's row, due today
            item("today", Some("2026-09-09T10:00"), Some(TODAY), 2),
            // on today, deadline behind → the when's row, overdue, in
            // the fold
            item("owed-today", Some(TODAY), Some("2026-09-01"), 3),
        ];
        let days = build_agenda(&items, TODAY, HORIZON);
        let rows: Vec<(&str, &str, PlacedBy, Tone)> = days
            .iter()
            .flat_map(|d| {
                d.rows
                    .iter()
                    .map(|r| (d.day.as_str(), r.item.id.as_str(), r.placed_by, r.tone))
            })
            .collect();
        assert_eq!(
            rows,
            vec![
                (TODAY, "owed-today", PlacedBy::When, Tone::Overdue),
                (TODAY, "today", PlacedBy::When, Tone::Warning),
                ("2026-09-12", "same-day", PlacedBy::When, Tone::Neutral),
            ]
        );
    }

    #[test]
    fn within_day_all_day_events_then_timed_then_untimed() {
        let mut birthday = item("birthday", Some("2026-09-12"), None, 8);
        birthday.event = true;
        let mut gig09 = item("gig09", Some("2026-09-12T09:00"), None, 7);
        gig09.event = true;
        let mut done_task = item("done-task", Some("2026-09-12"), None, 10);
        done_task.state = WorkflowState::Done;
        let items = [
            item("t14", Some("2026-09-12T14:00"), None, 1),
            item("task-late", Some("2026-09-12"), None, 9),
            item("task-early", Some("2026-09-12"), None, 2),
            item("due", None, Some("2026-09-12"), 3),
            birthday,
            gig09,
            done_task,
        ];
        let days = build_agenda(&items, TODAY, HORIZON);
        // Inside the last band: raw string (one day here), then
        // `created_at`.
        assert_eq!(
            ids(&days[1]),
            vec![
                "birthday",
                "gig09",
                "t14",
                "task-early",
                "due",
                "task-late",
                "done-task"
            ]
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
            vec![
                "over-old",
                "past-old",
                "past-new",
                "over-new",
                "own-timed",
                "own-allday"
            ]
        );
    }

    #[test]
    fn a_tasks_past_when_is_overdue_whatever_its_deadline() {
        // Started yesterday, runs three days: under way, not slipped.
        let mut running = item("running", Some("2026-09-08"), None, 5);
        running.duration = Some(3 * 1440);
        // Ended yesterday.
        let mut span_over = item("span-over", Some("2026-09-06"), None, 6);
        span_over.duration = Some(3 * 1440);
        // Overnight: 23:00 + 2h ends today.
        let mut overnight = item("overnight", Some("2026-09-08T23:00"), None, 7);
        overnight.duration = Some(120);
        let items = [
            item("past-when", Some("2026-09-07T08:00"), None, 1),
            // past when, overdue deadline → by the deadline: it is owed
            item("past-overdue", Some("2026-09-01"), Some("2026-09-02"), 2),
            // past when, deadline today → one row in Today, overdue, by
            // the when; a later deadline has its own row on its day
            item("past-due-today", Some("2026-09-01"), Some(TODAY), 3),
            item("past-due-later", Some("2026-09-01"), Some("2026-09-20"), 4),
            running,
            span_over,
            overnight,
        ];
        let days = build_agenda(&items, TODAY, HORIZON);
        let by_day: Vec<(&str, Vec<&str>)> =
            days.iter().map(|d| (d.day.as_str(), ids(d))).collect();
        assert_eq!(
            by_day,
            vec![
                (
                    TODAY,
                    vec![
                        "past-due-today",
                        "past-due-later",
                        "past-overdue",
                        "span-over",
                        "past-when",
                    ]
                ),
                ("2026-09-20", vec!["past-due-later"]),
            ]
        );
        assert_eq!(days[1].rows[0].placed_by, PlacedBy::Deadline);
        assert_eq!(days[1].rows[0].tone, Tone::Neutral);
        let rows: Vec<(PlacedBy, Tone)> =
            days[0].rows.iter().map(|r| (r.placed_by, r.tone)).collect();
        assert_eq!(
            rows,
            vec![
                (PlacedBy::When, Tone::Overdue),
                (PlacedBy::When, Tone::Overdue),
                (PlacedBy::Deadline, Tone::Overdue),
                (PlacedBy::When, Tone::Overdue),
                (PlacedBy::When, Tone::Overdue),
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
                (TODAY, vec!["done-early", "open-today", "done-today"]),
                ("2026-09-10", vec!["done-both", "future"]),
            ]
        );
        assert_eq!(days[1].rows[0].placed_by, PlacedBy::When);
        assert_eq!(days[1].rows[0].tone, Tone::Neutral);
    }

    #[test]
    fn events_place_by_the_same_rules_as_tasks_and_go_by() {
        let mut event = item("event", Some("2026-09-12T19:00"), None, 1);
        event.event = true;
        // A deadline beside an event's date is legitimate: its own row.
        let mut tickets = item("tickets", Some("2026-09-12"), Some("2026-09-10"), 2);
        tickets.event = true;
        // A past event went by; a past task is overdue.
        let mut past = item("past", Some("2026-09-01"), None, 3);
        past.event = true;
        let slipped = item("slipped", Some("2026-09-01"), None, 4);
        let items = [event, tickets, past, slipped];
        let days = build_agenda(&items, TODAY, HORIZON);
        let by_day: Vec<(&str, Vec<&str>)> =
            days.iter().map(|d| (d.day.as_str(), ids(d))).collect();
        assert_eq!(
            by_day,
            vec![
                (TODAY, vec!["slipped"]),
                ("2026-09-10", vec!["tickets"]),
                ("2026-09-12", vec!["tickets", "event"]),
            ]
        );
    }

    #[test]
    fn days_gone_by_hold_events_and_done_items_from_the_floor() {
        let mut gig = item("gig", Some("2026-09-01T19:00"), None, 1);
        gig.event = true;
        let mut done = item("done", Some("2026-09-01"), None, 2);
        done.state = WorkflowState::Done;
        // Still owed: Today's overdue fold only, not its own day too.
        let slipped = item("slipped", Some("2026-09-01"), None, 3);
        // A deadline never places in the past, done or not.
        let mut done_deadline = item("done-deadline", None, Some("2026-09-02"), 4);
        done_deadline.state = WorkflowState::Done;
        // Happened then, and its deadline is still to come: both days.
        let mut tickets = item("tickets", Some("2026-09-03"), Some("2026-09-10"), 5);
        tickets.event = true;
        // Before the floor: dropped.
        let mut ancient = item("ancient", Some("2026-08-01"), None, 6);
        ancient.event = true;
        let mut cancelled = item("cancelled", Some("2026-09-01"), None, 7);
        cancelled.state = WorkflowState::Cancelled;
        let mut binned = item("binned", Some("2026-09-01"), None, 8);
        binned.event = true;
        binned.binned_at = Some(5);
        let items = [
            gig,
            done,
            slipped,
            done_deadline,
            tickets,
            ancient,
            cancelled,
            binned,
        ];
        let days = build_agenda_from(&[(None, &items)], "2026-08-26", TODAY, HORIZON);
        let by_day: Vec<(&str, Vec<&str>)> =
            days.iter().map(|d| (d.day.as_str(), ids(d))).collect();
        assert_eq!(
            by_day,
            vec![
                ("2026-09-01", vec!["gig", "done"]),
                ("2026-09-03", vec!["tickets"]),
                (TODAY, vec!["slipped"]),
                ("2026-09-10", vec!["tickets"]),
            ]
        );
        assert!(
            days[..2]
                .iter()
                .flat_map(|d| &d.rows)
                .all(|r| r.tone == Tone::Neutral && r.placed_by == PlacedBy::When)
        );
        // An empty Today still takes its place after the days gone by,
        // and no floor means none of them.
        let days = build_agenda_from(&[(None, &items[..2])], "2026-08-26", TODAY, HORIZON);
        let order: Vec<&str> = days.iter().map(|d| d.day.as_str()).collect();
        assert_eq!(order, vec!["2026-09-01", TODAY]);
        assert!(days[1].rows.is_empty());
        let days = build_agenda(&items[..2], TODAY, HORIZON);
        assert_eq!(days.len(), 1);
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
