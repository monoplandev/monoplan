//! `monoplan done`: what got finished, newest first, grouped by the
//! local day it was ticked (`spec/cli.md` §Items). The mirror of
//! `agenda`: that looks ahead over dated items, this looks back over
//! completed ones, across every list of the selected workspace.
//!
//! An item is listed while its workflow register says Done and it is
//! not binned, at the register's `at` (the moment it entered Done).
//! The `done_at` reflection stamp is not read: it is never cleared, so
//! a reopened item still carries one. Cancelled is not a completion
//! and is left out.

use std::collections::HashMap;
use std::io::Write;

use chrono::{DateTime, Datelike, Days, Local, NaiveDate, TimeZone};
use clap::Parser;
use monoplan_core::{Doc, ItemView, LIST_EVENTS, LIST_INBOX};
use serde::{Deserialize, Serialize};

use super::items::{date_tags, item_json, row_box, write_json};

/// Rows shown when neither `-n` nor `--since` bounds the list.
const DEFAULT_LIMIT: usize = 20;

#[derive(Parser, Debug, Clone, Serialize, Deserialize)]
pub struct DoneArgs {
    /// At most this many items, newest first. Defaults to 20, or to no
    /// limit when `--since` is given.
    #[arg(short = 'n', long = "limit", value_name = "N")]
    pub limit: Option<usize>,
    /// Only items done on or after this local day: `YYYY-MM-DD`, or
    /// relative as `7d` / `4w` (`0d` is today).
    #[arg(long, value_name = "DATE")]
    pub since: Option<String>,
    /// Machine-parseable output.
    #[arg(long)]
    pub json: bool,
}

/// Resolve `--since` to a day: `YYYY-MM-DD` as written, `<N>d` /
/// `<N>w` counted back from `today`.
pub fn since_day(since: &str, today: NaiveDate) -> anyhow::Result<NaiveDate> {
    let bad = || anyhow::anyhow!("--since must be YYYY-MM-DD, or relative like 7d / 4w: {since:?}");
    let s = since.trim();
    let relative = match s.char_indices().last() {
        Some((i, 'd')) => Some((&s[..i], 1)),
        Some((i, 'w')) => Some((&s[..i], 7)),
        _ => None,
    };
    match relative {
        Some((n, unit)) => {
            let n: u64 = n.parse().map_err(|_| bad())?;
            let days = n.checked_mul(unit).ok_or_else(bad)?;
            today.checked_sub_days(Days::new(days)).ok_or_else(bad)
        }
        None => NaiveDate::parse_from_str(s, "%Y-%m-%d").map_err(|_| bad()),
    }
}

/// Unix millis of local midnight opening `day`. Where a DST jump
/// skips midnight the day's first instant is not on the clock, so the
/// cutoff falls back to midnight UTC of that date.
fn day_start_millis(day: NaiveDate) -> i64 {
    let midnight = day.and_hms_opt(0, 0, 0).expect("midnight is a valid time");
    match midnight.and_local_timezone(Local).earliest() {
        Some(t) => t.timestamp_millis(),
        None => midnight.and_utc().timestamp_millis(),
    }
}

/// The Done, not-binned items entered Done at or after `since` (unix
/// millis; `None` is unbounded), newest first, cut to `limit`.
pub fn select_done(items: &[ItemView], since: Option<i64>, limit: Option<usize>) -> Vec<&ItemView> {
    let mut done: Vec<&ItemView> = items
        .iter()
        .filter(|i| i.is_done() && !i.is_binned())
        .filter(|i| since.is_none_or(|s| i.lifecycle_at >= s))
        .collect();
    done.sort_by(|a, b| {
        b.lifecycle_at
            .cmp(&a.lifecycle_at)
            .then_with(|| a.id.cmp(&b.id))
    });
    if let Some(n) = limit {
        done.truncate(n);
    }
    done
}

pub fn run(doc: &Doc, args: DoneArgs, out: &mut dyn Write) -> anyhow::Result<()> {
    let since = match &args.since {
        Some(s) => Some(day_start_millis(since_day(s, Local::now().date_naive())?)),
        None => None,
    };
    // A window with no `-n` is the whole window; a bare call is the
    // recent few.
    let limit = args
        .limit
        .or_else(|| since.is_none().then_some(DEFAULT_LIMIT));

    let items = doc.all_items();
    let done = select_done(&items, since, limit);
    if args.json {
        return write_json(out, &done.iter().map(|i| item_json(i)).collect::<Vec<_>>());
    }
    if done.is_empty() {
        writeln!(out, "Nothing done.")?;
        return Ok(());
    }
    let lists: HashMap<String, String> = doc
        .all_lists()
        .into_iter()
        .map(|l| (l.id, l.name))
        .collect();
    let mut current: Option<NaiveDate> = None;
    for item in done {
        let day = local_day(item.lifecycle_at);
        if current != Some(day) {
            if current.is_some() {
                writeln!(out)?;
            }
            writeln!(out, "{}  {}", day.format("%Y-%m-%d"), day.weekday())?;
            current = Some(day);
        }
        // The reserved lists have no `ListMeta` row, so they are named
        // here, as `starred --all` names them.
        let list = match item.list_id.as_str() {
            LIST_INBOX => "Inbox",
            LIST_EVENTS => "Events",
            other => lists.get(other).map_or(other, String::as_str),
        };
        writeln!(
            out,
            "  {}  {} {}{}  ({list})",
            item.id,
            row_box(item),
            item.text,
            date_tags(item)
        )?;
    }
    Ok(())
}

/// The local calendar day a unix-millis stamp falls on.
fn local_day(millis: i64) -> NaiveDate {
    match Local.timestamp_millis_opt(millis).single() {
        Some(t) => t.date_naive(),
        None => DateTime::from_timestamp_millis(millis)
            .unwrap_or_default()
            .date_naive(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use monoplan_core::WorkflowState;

    fn item(id: &str, state: WorkflowState, at: i64) -> ItemView {
        ItemView {
            id: id.to_string(),
            text: id.to_string(),
            notes: String::new(),
            list_id: LIST_INBOX.to_string(),
            state,
            lifecycle_at: at,
            deadline: None,
            when: None,
            duration: None,
            place: None,
            created_at: 0,
            created_by: None,
            started_at: None,
            done_at: None,
            binned_at: None,
        }
    }

    fn ids(items: &[&ItemView]) -> Vec<String> {
        items.iter().map(|i| i.id.clone()).collect()
    }

    #[test]
    fn only_done_unbinned_newest_first() {
        let mut binned = item("binned", WorkflowState::Done, 50);
        binned.binned_at = Some(60);
        // Reopened: the reflection stamp survives, the register moved on.
        let mut reopened = item("reopened", WorkflowState::Todo, 70);
        reopened.done_at = Some(40);
        let items = vec![
            item("old", WorkflowState::Done, 10),
            item("new", WorkflowState::Done, 30),
            item("mid", WorkflowState::Done, 20),
            item("cancelled", WorkflowState::Cancelled, 25),
            item("open", WorkflowState::Backlog, 5),
            binned,
            reopened,
        ];
        assert_eq!(ids(&select_done(&items, None, None)), ["new", "mid", "old"]);
    }

    #[test]
    fn since_is_inclusive_and_limit_cuts_the_oldest() {
        let items = vec![
            item("a", WorkflowState::Done, 10),
            item("b", WorkflowState::Done, 20),
            item("c", WorkflowState::Done, 30),
        ];
        assert_eq!(ids(&select_done(&items, Some(20), None)), ["c", "b"]);
        assert_eq!(ids(&select_done(&items, None, Some(1))), ["c"]);
        assert_eq!(ids(&select_done(&items, Some(20), Some(1))), ["c"]);
        assert!(select_done(&items, Some(31), None).is_empty());
    }

    #[test]
    fn equal_stamps_order_by_id() {
        let items = vec![
            item("b", WorkflowState::Done, 10),
            item("a", WorkflowState::Done, 10),
        ];
        assert_eq!(ids(&select_done(&items, None, None)), ["a", "b"]);
    }

    #[test]
    fn since_day_parses_absolute_and_relative() {
        let today = NaiveDate::from_ymd_opt(2026, 11, 11).unwrap();
        let day = |y, m, d| NaiveDate::from_ymd_opt(y, m, d).unwrap();
        assert_eq!(since_day("2026-10-01", today).unwrap(), day(2026, 10, 1));
        assert_eq!(since_day("0d", today).unwrap(), today);
        assert_eq!(since_day("7d", today).unwrap(), day(2026, 11, 4));
        assert_eq!(since_day("4w", today).unwrap(), day(2026, 10, 14));
        for bad in ["", "d", "-3d", "7", "7m", "2026-13-01", "yesterday"] {
            assert!(since_day(bad, today).is_err(), "{bad:?} should not parse");
        }
    }
}
