//! `monoplan agenda`: dated items by day, Today first with overdue
//! deadlines folded in, then the coming days (`spec/calendar-plan.md`
//! "Agenda"). An Open task whose `when` has passed is still to do and
//! folds into Today as overdue too; an event's past `when` has gone by
//! and places nothing.
//! Done items keep their `when` day and slot (the tick does not erase
//! "happens on"); their deadline is settled and places nothing. Events
//! (`spec/events-plan.md`) place and print by the same rules; an Open
//! event with no date at all has no day, so it prints in an Unscheduled
//! group after Today.
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

/// Group dated items by placement day. `today` and `horizon` are
/// `YYYY-MM-DD`; days after `horizon` are dropped. Today is always the
/// first group, even when empty; later empty days are skipped.
///
/// - Placement day: the earlier of the `when` day and the `deadline`
///   day, the deadline clamped up to today, so an overdue deadline lands
///   in Today. An Open task (not an event) whose `when` is over, its
///   whole span ended before today, lands in Today as well, placed by
///   the `when` unless its deadline is overdue too. An event's past
///   `when` places nothing: with only that it is dropped, and with a
///   deadline too it places by the deadline.
/// - Tone: overdue (deadline < today, or a task's `when` over), warning
///   (deadline = today), else neutral.
/// - Within a day: overdue rows (oldest first, by the date that made
///   them overdue), then the day's own rows by the raw string of the placing field (all-day ahead of
///   timed), then `created_at`. Ticking a row does not move it.
/// - Done items place by `when` only, on that day (today or later),
///   neutral. A done item with no `when`, or a past `when`, is dropped:
///   nothing is owed and past days are not shown. Cancelled and binned
///   items never appear: a cancelled slot is something that did not
///   happen, so it clutters rather than records.
pub fn build_agenda(items: &[ItemView], today: &str, horizon: &str) -> Vec<AgendaDay> {
    build_agenda_from(&[(None, items)], today, horizon)
}

/// [`build_agenda`] over several workspaces at once: one placement and
/// one order across all of them (a stable sort, so equal keys keep
/// source order), each row tagged with its source.
pub fn build_agenda_from(sources: &[Source<'_>], today: &str, horizon: &str) -> Vec<AgendaDay> {
    // (day, fold group, placing raw, created_at, row)
    let mut placed: Vec<(String, u8, String, i64, AgendaRow)> = Vec::new();
    for (doc, items) in sources {
        for item in *items {
            if item.is_binned() || item.is_cancelled() {
                continue;
            }
            // Past days are not shown, so a past `when` places nothing on
            // its own day; a task's lands in Today's overdue fold below.
            let when_day = item
                .when
                .as_deref()
                .map(|w| &w[..w.len().min(10)])
                .filter(|w| *w >= today);
            let deadline_day = item.deadline.as_deref();
            let deadline_overdue = deadline_day.is_some_and(|d| d < today);
            // Planned for a day that has passed and still Open: overdue,
            // even with a deadline still ahead. An overdue deadline keeps
            // the row instead: it is what is owed.
            let slipped = !item.is_done()
                && !item.is_event()
                && item
                    .when
                    .as_deref()
                    .is_some_and(|w| when_end_day(w, item.duration).as_str() < today);
            if slipped && !deadline_overdue {
                placed.push((
                    today.to_string(),
                    0,
                    item.when.clone().unwrap(),
                    item.created_at,
                    AgendaRow {
                        item: item.clone(),
                        placed_by: PlacedBy::When,
                        tone: Tone::Overdue,
                        doc: doc.cloned(),
                    },
                ));
                continue;
            }
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
                        doc: doc.cloned(),
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
            let tone = if deadline_overdue {
                Tone::Overdue
            } else if deadline_day == Some(today) {
                Tone::Warning
            } else {
                Tone::Neutral
            };
            // Today's fold: overdue rows lead (by their own date), then
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

/// [`unscheduled_events`] across sources, oldest first, each tagged.
pub fn unscheduled_from<'a>(sources: &[Source<'a>]) -> Vec<(Option<Candidate>, &'a ItemView)> {
    let mut out: Vec<(Option<Candidate>, &'a ItemView)> = sources
        .iter()
        .flat_map(|(doc, items)| {
            unscheduled_events(items)
                .into_iter()
                .map(move |i| (doc.cloned(), i))
        })
        .collect();
    out.sort_by(|a, b| {
        a.1.created_at
            .cmp(&b.1.created_at)
            .then(a.1.id.cmp(&b.1.id))
    });
    out
}

/// `agenda`, and with `--all` one agenda over every workspace the
/// install has open (`api::agenda::place`). Rows of the merged one
/// carry their workspace.
pub fn run(cx: &Ctx, args: AgendaArgs, out: &mut dyn Write) -> anyhow::Result<()> {
    let placed = api::agenda::place(
        cx,
        &AgendaQuery {
            days: args.days,
            today: args.today.as_deref(),
            everywhere: args.all,
        },
    )?;
    if args.json {
        write_json(out, &api::agenda::groups(&placed))
    } else {
        print_agenda(out, &placed.days, &placed.unscheduled, &placed.today)
    }
}

/// `  [Work]` after a merged row, `  [Personal]` for Home; nothing on
/// a single-doc agenda.
fn doc_tag(doc: Option<&Candidate>) -> String {
    doc.map(|c| format!("  [{}]", c.name.as_deref().unwrap_or(HOME_LABEL)))
        .unwrap_or_default()
}

fn print_agenda(
    out: &mut dyn Write,
    days: &[AgendaDay],
    unscheduled: &[(Option<Candidate>, ItemView)],
    today: &str,
) -> anyhow::Result<()> {
    for (i, day) in days.iter().enumerate() {
        if i > 0 {
            writeln!(out)?;
        }
        // Unscheduled sits after Today, ahead of the coming days.
        if i == 1 && !unscheduled.is_empty() {
            print_unscheduled(out, unscheduled)?;
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
    // Today was the only day: the group still follows it.
    if days.len() == 1 && !unscheduled.is_empty() {
        writeln!(out)?;
        print_unscheduled(out, unscheduled)?;
    }
    Ok(())
}

fn print_unscheduled(
    out: &mut dyn Write,
    unscheduled: &[(Option<Candidate>, ItemView)],
) -> anyhow::Result<()> {
    writeln!(out, "Unscheduled")?;
    for (doc, item) in unscheduled {
        writeln!(
            out,
            "  {}  {} {}{}",
            item.id,
            row_box(item),
            item.text,
            doc_tag(doc.as_ref())
        )?;
    }
    Ok(())
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
            when_suggested: None,
            duration: None,
            place: None,
            created_at,
            created_by: None,
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
    fn placement_is_earlier_field_clamped_to_today() {
        let items = [
            // past when on a task → Today, overdue
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
                (
                    TODAY,
                    vec!["past-when", "overdue", "both-late", "due-today"]
                ),
                ("2026-09-11", vec!["dl-first"]),
                ("2026-09-12", vec!["both"]),
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
            vec![
                "over-old",
                "past-old",
                "past-new",
                "over-new",
                "own-allday",
                "own-timed"
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
            // past when, deadline today or later → still overdue, by the when
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
            vec![(
                TODAY,
                vec![
                    "past-due-today",
                    "past-due-later",
                    "past-overdue",
                    "span-over",
                    "past-when",
                ]
            )]
        );
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
