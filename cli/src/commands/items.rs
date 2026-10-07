//! Item commands: add / ls / backlog / todo / start / review / done /
//! bin (verb) / restore / mv / edit / when / duration / deadline / place.
//!
//! Every action goes through `Session` (open → mutate → flush). The
//! session reads from and writes to the local Loro doc; it only talks
//! to the server when `-s/--sync` is passed (or `monoplan sync` is run
//! separately).

use std::io::{BufRead, IsTerminal};

use clap::Parser;
use monoplan_core::{ItemLifecycle, ItemView, LIST_INBOX, Place, WorkflowState};
use serde::Serialize;

use crate::sync::Session;

// ---------- add ----------

#[derive(Parser, Debug)]
pub struct AddArgs {
    /// Item text. Use `-` to read one item per non-blank line from stdin.
    pub text: String,
    /// Target list. Defaults to `inbox`.
    #[arg(long, default_value = LIST_INBOX)]
    pub list: String,
}

pub async fn add(args: AddArgs, sync: bool) -> anyhow::Result<()> {
    let session = Session::open(sync).await?;
    let texts = collect_texts(&args.text)?;
    if texts.is_empty() {
        anyhow::bail!("no item text provided");
    }
    let mut ids = Vec::with_capacity(texts.len());
    for text in &texts {
        ids.push(session.doc().add_item(&args.list, text)?);
    }
    session.flush().await?;
    for id in ids {
        println!("{id}");
    }
    Ok(())
}

fn collect_texts(arg: &str) -> anyhow::Result<Vec<String>> {
    if arg == "-" {
        let stdin = std::io::stdin();
        if stdin.is_terminal() {
            anyhow::bail!(
                "`add -` reads from stdin but stdin is a tty — pipe input or pass text directly"
            );
        }
        let mut out = Vec::new();
        for line in stdin.lock().lines() {
            let line = line?;
            let trimmed = line.trim();
            if !trimmed.is_empty() {
                out.push(trimmed.to_string());
            }
        }
        Ok(out)
    } else {
        Ok(vec![arg.to_string()])
    }
}

// ---------- ls ----------

#[derive(Parser, Debug)]
pub struct LsArgs {
    /// List to show. Defaults to `inbox`.
    #[arg(long, default_value = LIST_INBOX)]
    pub list: String,
    /// Include closed items (`Done` and `Cancelled`).
    #[arg(long)]
    pub done: bool,
    /// Machine-parseable output.
    #[arg(long)]
    pub json: bool,
}

pub async fn ls(args: LsArgs, sync: bool) -> anyhow::Result<()> {
    let session = Session::open(sync).await?;
    let mut items = session.doc().items_in_list(&args.list, false);
    if !args.done {
        items.retain(|i| !i.is_closed());
    }
    if args.json {
        print_json(&items.iter().map(item_json).collect::<Vec<_>>())?;
    } else {
        print_items(&items);
    }
    session.flush().await?;
    Ok(())
}

#[derive(Serialize)]
struct ItemJson<'a> {
    id: &'a str,
    text: &'a str,
    list_id: &'a str,
    /// Workflow register state name (`spec/data-model.md` "Lifecycle").
    state: &'static str,
    /// Unix millis the register's state was entered.
    lifecycle_at: i64,
    created_at: i64,
    started_at: Option<i64>,
    done_at: Option<i64>,
    binned_at: Option<i64>,
    /// Date-only deadline (`YYYY-MM-DD`), when set.
    #[serde(skip_serializing_if = "Option::is_none")]
    deadline: Option<&'a str>,
    /// Planned date (`YYYY-MM-DD` or `YYYY-MM-DDTHH:MM`), when set.
    #[serde(skip_serializing_if = "Option::is_none")]
    when: Option<&'a str>,
    /// Duration in whole minutes, when set.
    #[serde(skip_serializing_if = "Option::is_none")]
    duration: Option<u32>,
    /// Place (`{label, lat?, lon?, address?, ref?}`), when set.
    #[serde(skip_serializing_if = "Option::is_none")]
    place: Option<&'a Place>,
}

fn item_json(item: &ItemView) -> ItemJson<'_> {
    ItemJson {
        id: &item.id,
        text: &item.text,
        list_id: &item.list_id,
        state: item.state.name(),
        lifecycle_at: item.lifecycle_at,
        created_at: item.created_at,
        started_at: item.started_at,
        done_at: item.done_at,
        binned_at: item.binned_at,
        deadline: item.deadline.as_deref(),
        when: item.when.as_deref(),
        duration: item.duration,
        place: item.place.as_ref(),
    }
}

/// Trailing tags for a text row: ` @<when>` (with `+<duration>`
/// glued on when a timed `when` carries one, e.g. `@2026-09-12T14:00+1h30m`,
/// or an all-day one spans more than a day, in whole days: `@2026-09-12+3d`)
/// then ` !<deadline>`, then ` at:<place label>`, each only when set.
/// Shared by `ls` and `agenda`.
pub fn date_tags(item: &ItemView) -> String {
    let mut s = String::new();
    if let Some(w) = &item.when {
        s.push_str(" @");
        s.push_str(w);
        if let Some(n) = item.duration {
            if w.len() > 10 {
                s.push('+');
                s.push_str(&format_duration(n));
            } else if let days @ 2.. = n.div_ceil(DAY_MINUTES) {
                // All-day: the length counts whole days
                // (`spec/calendar-plan.md`); a single day is implied.
                s.push_str(&format!("+{days}d"));
            }
        }
    }
    if let Some(d) = &item.deadline {
        s.push_str(" !");
        s.push_str(d);
    }
    if let Some(p) = &item.place {
        s.push_str(" at:");
        s.push_str(&p.label);
    }
    s
}

/// One-character box mark for the workflow register's state.
pub fn state_mark(state: WorkflowState) -> &'static str {
    match state {
        WorkflowState::Backlog => " ",
        WorkflowState::Todo => "-",
        WorkflowState::InProgress => ">",
        WorkflowState::Review => "?",
        WorkflowState::Done => "x",
        WorkflowState::Cancelled => "/",
    }
}

/// The row's state box, `[<mark>]`. Shared by `ls` and `agenda`.
pub fn row_box(item: &ItemView) -> String {
    // `~` (binned) masks the workflow mark in the box.
    if item.is_binned() {
        "[~]".to_string()
    } else {
        format!("[{}]", state_mark(item.state))
    }
}

fn print_items(items: &[ItemView]) {
    for item in items {
        // The preserved state shows as a trailing tag so a binned row
        // stays legible.
        let suffix = if item.is_binned() {
            format!(" ({})", item.state.name())
        } else {
            String::new()
        };
        println!(
            "{}  {} {}{}{suffix}",
            item.id,
            row_box(item),
            item.text,
            date_tags(item)
        );
    }
}

pub fn print_json<T: Serialize>(value: &T) -> anyhow::Result<()> {
    let s = serde_json::to_string_pretty(value)?;
    println!("{s}");
    Ok(())
}

// ---------- backlog / todo / start / review / done / cancel / bin / restore ----------

#[derive(Parser, Debug)]
pub struct IdArg {
    pub item_id: String,
}

/// Shared workflow transition: write the `[state, now]` register (and
/// clear any bin mask) per the transition table in `spec/data-model.md`.
/// One commit.
async fn transition(args: IdArg, sync: bool, lifecycle: ItemLifecycle) -> anyhow::Result<()> {
    let session = Session::open(sync).await?;
    session.doc().set_item_lifecycle(&args.item_id, lifecycle)?;
    session.flush().await?;
    println!("{}", args.item_id);
    Ok(())
}

/// Workflow → Backlog.
pub async fn backlog(args: IdArg, sync: bool) -> anyhow::Result<()> {
    transition(args, sync, ItemLifecycle::Backlog).await
}

/// Workflow → Todo.
pub async fn todo(args: IdArg, sync: bool) -> anyhow::Result<()> {
    transition(args, sync, ItemLifecycle::Todo).await
}

/// Workflow → In Progress (stamps `started_at` on first entry).
pub async fn start(args: IdArg, sync: bool) -> anyhow::Result<()> {
    transition(args, sync, ItemLifecycle::InProgress).await
}

/// Workflow → Review.
pub async fn review(args: IdArg, sync: bool) -> anyhow::Result<()> {
    transition(args, sync, ItemLifecycle::Review).await
}

/// Workflow → Done (stamps `done_at`).
pub async fn done(args: IdArg, sync: bool) -> anyhow::Result<()> {
    transition(args, sync, ItemLifecycle::Done).await
}

/// Workflow → Cancelled (closed, but stamps nothing: not a completion).
pub async fn cancel(args: IdArg, sync: bool) -> anyhow::Result<()> {
    transition(args, sync, ItemLifecycle::Cancelled).await
}

pub async fn bin(args: IdArg, sync: bool) -> anyhow::Result<()> {
    let session = Session::open(sync).await?;
    session.doc().set_item_binned(&args.item_id, true)?;
    session.flush().await?;
    println!("{}", args.item_id);
    Ok(())
}

/// Restore from the bin: clear the mask only, revealing the preserved
/// workflow state — a done-then-binned item pops back into the Done
/// view, an open one back into its list at its former position.
pub async fn restore(args: IdArg, sync: bool) -> anyhow::Result<()> {
    let session = Session::open(sync).await?;
    session.doc().set_item_binned(&args.item_id, false)?;
    session.flush().await?;
    println!("{}", args.item_id);
    Ok(())
}

// ---------- mv ----------

#[derive(Parser, Debug)]
pub struct MvArgs {
    pub item_id: String,
    pub list: String,
}

pub async fn mv(args: MvArgs, sync: bool) -> anyhow::Result<()> {
    let session = Session::open(sync).await?;
    // Append at the end of the target list (target_index = current
    // length). `move_item` clamps to the existing range, so passing a
    // huge index is safe — but the explicit count here is clearer.
    let target_idx = session.doc().items_in_list(&args.list, true).len();
    session
        .doc()
        .move_item(&args.item_id, &args.list, target_idx)?;
    session.flush().await?;
    println!("{}", args.item_id);
    Ok(())
}

// ---------- when / duration / deadline ----------

#[derive(Parser, Debug)]
pub struct DateArg {
    pub item_id: String,
    /// The value to set, or `-` to clear.
    pub value: String,
}

/// Minutes in a day: the unit an all-day span's length counts in.
const DAY_MINUTES: u32 = 24 * 60;

/// Render minutes as `2h`, `45m`, `1h30m`, and past a day `1d`, `2d3h`,
/// `1d30m`.
pub fn format_duration(minutes: u32) -> String {
    let d = minutes / DAY_MINUTES;
    let h = (minutes % DAY_MINUTES) / 60;
    let m = minutes % 60;
    let mut s = String::new();
    if d > 0 {
        s.push_str(&format!("{d}d"));
    }
    if h > 0 {
        s.push_str(&format!("{h}h"));
    }
    if m > 0 || s.is_empty() {
        s.push_str(&format!("{m}m"));
    }
    s
}

/// Parse a duration: plain minutes (`90`), or days, hours and minutes
/// with `d` / `h` / `m` suffixes in that order (`1h30m`, `2h`, `45m`,
/// `3d`, `1d2h`). Whitespace around and between parts is tolerated; the
/// range is the core's.
pub fn parse_duration(raw: &str) -> anyhow::Result<u32> {
    let s: String = raw.chars().filter(|c| !c.is_whitespace()).collect();
    let bad = || anyhow::anyhow!("duration must be minutes or like 1h30m / 3d: {raw:?}");
    if s.is_empty() {
        return Err(bad());
    }
    if s.bytes().all(|b| b.is_ascii_digit()) {
        return s.parse::<u32>().map_err(|_| bad());
    }
    let mut days: u32 = 0;
    let mut hours: u32 = 0;
    let mut minutes: u32 = 0;
    let mut rest = s.as_str();
    if let Some((d, tail)) = rest.split_once('d') {
        days = d.parse().map_err(|_| bad())?;
        rest = tail;
    }
    if let Some((h, tail)) = rest.split_once('h') {
        hours = h.parse().map_err(|_| bad())?;
        rest = tail;
    }
    if let Some(m) = rest.strip_suffix('m') {
        minutes = m.parse().map_err(|_| bad())?;
        rest = "";
    }
    if !rest.is_empty() || (days == 0 && hours == 0 && minutes == 0 && !s.contains(['d', 'h'])) {
        return Err(bad());
    }
    days.checked_mul(DAY_MINUTES)
        .and_then(|d| hours.checked_mul(60).and_then(|h| d.checked_add(h)))
        .and_then(|dh| dh.checked_add(minutes))
        .ok_or_else(bad)
}

/// Set or clear the planned date: `YYYY-MM-DD` (all-day) or
/// `YYYY-MM-DDTHH:MM` (timed); `-` clears. Validation lives in the core.
pub async fn when(args: DateArg, sync: bool) -> anyhow::Result<()> {
    let session = Session::open(sync).await?;
    session
        .doc()
        .set_item_when(&args.item_id, clear_or(&args.value))?;
    session.flush().await?;
    println!("{}", args.item_id);
    Ok(())
}

/// Set or clear the duration: minutes or `1h30m`; `-` clears. Range
/// validation lives in the core.
pub async fn duration(args: DateArg, sync: bool) -> anyhow::Result<()> {
    let value = match clear_or(&args.value) {
        Some(v) => Some(parse_duration(v)?),
        None => None,
    };
    let session = Session::open(sync).await?;
    session.doc().set_item_duration(&args.item_id, value)?;
    session.flush().await?;
    println!("{}", args.item_id);
    Ok(())
}

/// Set or clear the date-only deadline (`YYYY-MM-DD`); `-` clears.
pub async fn deadline(args: DateArg, sync: bool) -> anyhow::Result<()> {
    let session = Session::open(sync).await?;
    session
        .doc()
        .set_item_deadline(&args.item_id, clear_or(&args.value))?;
    session.flush().await?;
    println!("{}", args.item_id);
    Ok(())
}

fn clear_or(value: &str) -> Option<&str> {
    if value == "-" { None } else { Some(value) }
}

// ---------- place ----------

#[derive(Parser, Debug)]
pub struct PlaceArgs {
    pub item_id: String,
    /// The place's label, or `-` to clear.
    pub label: String,
    /// Coordinates as `<lat>,<lon>` in decimal degrees (WGS84).
    #[arg(long, value_name = "LAT,LON")]
    pub at: Option<String>,
    /// A formatted address, shown beside the label.
    #[arg(long)]
    pub address: Option<String>,
}

/// Parse `<lat>,<lon>` in decimal degrees; the range check is the core's.
pub fn parse_coords(raw: &str) -> anyhow::Result<(f64, f64)> {
    let bad = || anyhow::anyhow!("coordinates must be <lat>,<lon> in decimal degrees: {raw:?}");
    let (lat, lon) = raw.split_once(',').ok_or_else(bad)?;
    let lat: f64 = lat.trim().parse().map_err(|_| bad())?;
    let lon: f64 = lon.trim().parse().map_err(|_| bad())?;
    Ok((lat, lon))
}

/// Set or clear the place: a label, optionally with `--at lat,lon` and
/// `--address`; `-` clears. Validation lives in the core.
pub async fn place(args: PlaceArgs, sync: bool) -> anyhow::Result<()> {
    let value = match clear_or(&args.label) {
        None => None,
        Some(label) => {
            let coords = args.at.as_deref().map(parse_coords).transpose()?;
            Some(Place {
                label: label.to_string(),
                lat: coords.map(|c| c.0),
                lon: coords.map(|c| c.1),
                address: args.address.clone(),
                reference: None,
            })
        }
    };
    let session = Session::open(sync).await?;
    session
        .doc()
        .set_item_place(&args.item_id, value.as_ref())?;
    session.flush().await?;
    println!("{}", args.item_id);
    Ok(())
}

// ---------- edit ----------

#[derive(Parser, Debug)]
pub struct EditArgs {
    pub item_id: String,
    pub text: String,
}

pub async fn edit(args: EditArgs, sync: bool) -> anyhow::Result<()> {
    let session = Session::open(sync).await?;
    session.doc().edit_item_text(&args.item_id, &args.text)?;
    session.flush().await?;
    println!("{}", args.item_id);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn duration_parses_minutes_and_hm_forms() {
        assert_eq!(parse_duration("90").unwrap(), 90);
        assert_eq!(parse_duration("1h30m").unwrap(), 90);
        assert_eq!(parse_duration("2h").unwrap(), 120);
        assert_eq!(parse_duration("45m").unwrap(), 45);
        assert_eq!(parse_duration(" 1h 5m ").unwrap(), 65);
        assert_eq!(parse_duration("0h").unwrap(), 0);
        assert_eq!(parse_duration("3d").unwrap(), 3 * 1440);
        assert_eq!(parse_duration("1d2h").unwrap(), 1440 + 120);
        assert_eq!(parse_duration("1d 30m").unwrap(), 1440 + 30);
        assert_eq!(parse_duration("1d2h30m").unwrap(), 1440 + 150);
        for bad in [
            "", "h", "m", "d", "1x", "30m1h", "1h30", "-5", "1.5h", "2h1d", "1d1d",
        ] {
            assert!(parse_duration(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn coords_parse_as_lat_lon_pair() {
        assert_eq!(
            parse_coords("-33.8688,151.2093").unwrap(),
            (-33.8688, 151.2093)
        );
        assert_eq!(parse_coords(" 10 , 20 ").unwrap(), (10.0, 20.0));
        for bad in ["", "1", "1,", "a,b", "1;2"] {
            assert!(parse_coords(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn duration_formats_compactly() {
        assert_eq!(format_duration(45), "45m");
        assert_eq!(format_duration(120), "2h");
        assert_eq!(format_duration(90), "1h30m");
        assert_eq!(format_duration(1440), "1d");
        assert_eq!(format_duration(2 * 1440 + 180), "2d3h");
        assert_eq!(format_duration(1440 + 30), "1d30m");
    }

    #[test]
    fn date_tags_show_all_day_spans_in_days() {
        let base = ItemView {
            id: "i".into(),
            text: "t".into(),
            notes: String::new(),
            list_id: "l".into(),
            state: WorkflowState::Backlog,
            lifecycle_at: 0,
            deadline: None,
            when: Some("2026-10-06".into()),
            duration: None,
            place: None,
            created_at: 0,
            started_at: None,
            done_at: None,
            binned_at: None,
        };
        assert_eq!(date_tags(&base), " @2026-10-06");
        let single = ItemView {
            duration: Some(1440),
            ..base.clone()
        };
        assert_eq!(date_tags(&single), " @2026-10-06");
        let three = ItemView {
            duration: Some(3 * 1440),
            ..base.clone()
        };
        assert_eq!(date_tags(&three), " @2026-10-06+3d");
        let timed = ItemView {
            when: Some("2026-10-06T09:00".into()),
            duration: Some(2 * 1440 + 60),
            ..base
        };
        assert_eq!(date_tags(&timed), " @2026-10-06T09:00+2d1h");
    }
}
