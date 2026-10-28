//! Item commands: `item add`, `item ls`, `item set` (every per-item
//! field as a flag), `item bin`, `item restore`, and `places`.
//!
//! Every action is a pure function of the doc and its args that writes
//! its output to `out`: the daemon runs it against the resident doc
//! and ships the buffer back to the client (`spec/daemon-plan.md`).

use std::io::{BufRead, IsTerminal, Write};

use clap::{ArgGroup, Parser, ValueEnum};
use monoplan_core::{
    Doc, ItemLifecycle, ItemView, LIST_INBOX, Place, PlaceSuggestion, WorkflowState, fold_label,
};
use serde::{Deserialize, Serialize};

// ---------- add ----------

#[derive(Parser, Debug, Clone, Serialize, Deserialize)]
pub struct AddArgs {
    /// Item text. Use `-` to read one item per non-blank line from stdin.
    pub text: String,
    /// Target list. Defaults to `inbox`.
    #[arg(long, default_value = LIST_INBOX)]
    pub list: String,
}

/// Daemon side of `add`: `texts` were already expanded from the
/// argument (or stdin) by the client via [`collect_texts`].
pub fn add(doc: &Doc, list: &str, texts: &[String], out: &mut dyn Write) -> anyhow::Result<()> {
    if texts.is_empty() {
        anyhow::bail!("no item text provided");
    }
    let mut ids = Vec::with_capacity(texts.len());
    for text in texts {
        ids.push(doc.add_item(list, text)?);
    }
    for id in ids {
        writeln!(out, "{id}")?;
    }
    Ok(())
}

/// Client side of `add`: `-` reads one item per non-blank line from
/// stdin, anything else is a single item.
pub fn collect_texts(arg: &str) -> anyhow::Result<Vec<String>> {
    if arg == "-" {
        let stdin = std::io::stdin();
        if stdin.is_terminal() {
            anyhow::bail!(
                "`item add -` reads from stdin but stdin is a tty — pipe input or pass text directly"
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

#[derive(Parser, Debug, Clone, Serialize, Deserialize)]
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

pub fn ls(doc: &Doc, args: LsArgs, out: &mut dyn Write) -> anyhow::Result<()> {
    let mut items = doc.items_in_list(&args.list, false);
    if !args.done {
        items.retain(|i| !i.is_closed());
    }
    if args.json {
        write_json(out, &items.iter().map(item_json).collect::<Vec<_>>())?;
    } else {
        print_items(out, &items)?;
    }
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
    /// Creator's account id, when the creating client stamped one.
    #[serde(skip_serializing_if = "Option::is_none")]
    created_by: Option<&'a str>,
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
        created_by: item.created_by.as_deref(),
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

fn print_items(out: &mut dyn Write, items: &[ItemView]) -> anyhow::Result<()> {
    for item in items {
        // The preserved state shows as a trailing tag so a binned row
        // stays legible.
        let suffix = if item.is_binned() {
            format!(" ({})", item.state.name())
        } else {
            String::new()
        };
        writeln!(
            out,
            "{}  {} {}{}{suffix}",
            item.id,
            row_box(item),
            item.text,
            date_tags(item)
        )?;
    }
    Ok(())
}

pub fn write_json<T: Serialize>(out: &mut dyn Write, value: &T) -> anyhow::Result<()> {
    let s = serde_json::to_string_pretty(value)?;
    writeln!(out, "{s}")?;
    Ok(())
}

/// `write_json` straight to this process's stdout, for the commands
/// that stay local (`status`, `db`).
pub fn print_json<T: Serialize>(value: &T) -> anyhow::Result<()> {
    write_json(&mut std::io::stdout().lock(), value)
}

// ---------- lifecycle / bin / restore ----------

#[derive(Parser, Debug, Clone, Serialize, Deserialize)]
pub struct IdArg {
    pub item_id: String,
}

/// The workflow register's six states as `item set --lifecycle` spells
/// them: the core's canonical names (`WorkflowState::name`), so they
/// match what `ls --json` / `agenda --json` print back.
#[derive(ValueEnum, Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum LifecycleState {
    Backlog,
    Todo,
    /// Stamps `started_at` on first entry.
    #[value(name = "in_progress", alias = "in-progress")]
    InProgress,
    Review,
    /// Stamps `done_at`.
    Done,
    /// Closed like Done, but stamps nothing: not a completion.
    Cancelled,
}

impl From<LifecycleState> for ItemLifecycle {
    fn from(s: LifecycleState) -> Self {
        match s {
            LifecycleState::Backlog => ItemLifecycle::Backlog,
            LifecycleState::Todo => ItemLifecycle::Todo,
            LifecycleState::InProgress => ItemLifecycle::InProgress,
            LifecycleState::Review => ItemLifecycle::Review,
            LifecycleState::Done => ItemLifecycle::Done,
            LifecycleState::Cancelled => ItemLifecycle::Cancelled,
        }
    }
}

pub fn bin(doc: &Doc, args: IdArg, out: &mut dyn Write) -> anyhow::Result<()> {
    doc.set_item_binned(&args.item_id, true)?;
    writeln!(out, "{}", args.item_id)?;
    Ok(())
}

/// Restore from the bin: clear the mask only, revealing the preserved
/// workflow state — a done-then-binned item pops back into the Done
/// view, an open one back into its list at its former position.
pub fn restore(doc: &Doc, args: IdArg, out: &mut dyn Write) -> anyhow::Result<()> {
    doc.set_item_binned(&args.item_id, false)?;
    writeln!(out, "{}", args.item_id)?;
    Ok(())
}

// ---------- set ----------

/// Where `--star` puts the item: the end of Starred, or a 1-based slot.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum StarPos {
    Append,
    At(usize),
}

fn parse_star_pos(raw: &str) -> Result<StarPos, String> {
    if raw == "end" {
        return Ok(StarPos::Append);
    }
    raw.parse::<usize>()
        .map(StarPos::At)
        .map_err(|_| format!("position must be a 1-based number (or `end`): {raw:?}"))
}

/// `monoplan item set <id> …`: every per-item field as a flag, so one
/// call can set several. At least one field flag is required (the
/// `field` group). Each field is its own core setter and its own
/// commit, applied in declaration order; the first error stops the
/// sequence and the writes before it stand. What can be parsed up
/// front (duration, coordinates) is, before anything is written; dates
/// are validated by the core at write time.
#[derive(Parser, Debug, Clone, Serialize, Deserialize)]
#[command(group = ArgGroup::new("field").required(true).multiple(true))]
pub struct SetArgs {
    pub item_id: String,
    /// New item text.
    #[arg(long, group = "field")]
    pub text: Option<String>,
    /// Move to this list (`events` makes the item an event; moving it back out makes it a task again, dates intact).
    #[arg(long, group = "field", value_name = "LIST")]
    pub list: Option<String>,
    /// Workflow state: backlog | todo | in_progress | review | done | cancelled.
    #[arg(long, group = "field", value_name = "STATE")]
    pub lifecycle: Option<LifecycleState>,
    /// Planned date, `YYYY-MM-DD` (all-day) or `YYYY-MM-DDTHH:MM` (timed, floating); `-` clears it and the duration.
    #[arg(long, group = "field", value_name = "DATE|-")]
    pub when: Option<String>,
    /// Duration: minutes, or `1h30m` / `2h` / `45m` / `3d` / `1d2h`; `-` clears.
    #[arg(long, group = "field", value_name = "LEN|-")]
    pub duration: Option<String>,
    /// Deadline, `YYYY-MM-DD`; `-` clears.
    #[arg(long, group = "field", value_name = "DATE|-")]
    pub deadline: Option<String>,
    /// Place label; `-` clears. A bare label that matches a known place takes that place whole.
    #[arg(long, group = "field", value_name = "LABEL|-")]
    pub place: Option<String>,
    /// Coordinates for --place, `<lat>,<lon>` in decimal degrees (WGS84).
    #[arg(long, value_name = "LAT,LON", requires = "place")]
    pub coords: Option<String>,
    /// A formatted address for --place, shown beside the label.
    #[arg(long, requires = "place")]
    pub address: Option<String>,
    /// Write --place as given, even if a known place matches it.
    #[arg(long, requires = "place")]
    pub new_place: bool,
    /// Star the item: append to Starred, or insert at a 1-based POS. No-op if already starred.
    #[arg(long, group = "field", value_name = "POS", num_args = 0..=1, default_missing_value = "end", value_parser = parse_star_pos, conflicts_with = "unstar")]
    pub star: Option<StarPos>,
    /// Unstar the item (the item itself is untouched).
    #[arg(long, group = "field")]
    pub unstar: bool,
}

pub fn set(doc: &Doc, args: SetArgs, out: &mut dyn Write) -> anyhow::Result<()> {
    let id = args.item_id.as_str();
    // Parse first, so a typo in one flag writes nothing.
    let duration = match args.duration.as_deref().map(clear_or) {
        Some(Some(v)) => Some(Some(parse_duration(v)?)),
        Some(None) => Some(None),
        None => None,
    };
    let coords = args.coords.as_deref().map(parse_coords).transpose()?;

    if let Some(text) = &args.text {
        doc.edit_item_text(id, text)?;
    }
    if let Some(list) = &args.list {
        // Append at the end of the target list (target_index = current
        // length); `move_item` clamps to the existing range anyway.
        let target_idx = doc.items_in_list(list, true).len();
        doc.move_item(id, list, target_idx)?;
    }
    if let Some(state) = args.lifecycle {
        doc.set_item_lifecycle(id, state.into())?;
    }
    if let Some(when) = &args.when {
        doc.set_item_when(id, clear_or(when))?;
    }
    if let Some(duration) = duration {
        doc.set_item_duration(id, duration)?;
    }
    if let Some(deadline) = &args.deadline {
        doc.set_item_deadline(id, clear_or(deadline))?;
    }
    if let Some(label) = &args.place {
        let value = match clear_or(label) {
            None => None,
            // A bare label that matches a place already on some item
            // (case-insensitively, `spec/place-plan.md` "Reuse") takes
            // that place whole, coordinates and address included, so
            // `--place gym` finds the Gym the web client geocoded;
            // --coords / --address / --new-place write exactly what was
            // given. Validation lives in the core.
            Some(label) => {
                let known = if coords.is_none() && args.address.is_none() && !args.new_place {
                    known_place(&doc.place_suggestions(label, 0), label)
                } else {
                    None
                };
                Some(known.unwrap_or_else(|| Place {
                    label: label.to_string(),
                    lat: coords.map(|c| c.0),
                    lon: coords.map(|c| c.1),
                    address: args.address.clone(),
                    reference: None,
                }))
            }
        };
        doc.set_item_place(id, value.as_ref())?;
    }
    if let Some(pos) = args.star {
        let index = match pos {
            StarPos::Append => usize::MAX,
            StarPos::At(p) => super::focus::one_based_to_index(p),
        };
        doc.add_to_focus(id, index)?;
    }
    if args.unstar {
        doc.remove_from_focus(id)?;
    }
    writeln!(out, "{id}")?;
    Ok(())
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

fn clear_or(value: &str) -> Option<&str> {
    if value == "-" { None } else { Some(value) }
}

// ---------- place helpers ----------

/// Parse `<lat>,<lon>` in decimal degrees; the range check is the core's.
pub fn parse_coords(raw: &str) -> anyhow::Result<(f64, f64)> {
    let bad = || anyhow::anyhow!("coordinates must be <lat>,<lon> in decimal degrees: {raw:?}");
    let (lat, lon) = raw.split_once(',').ok_or_else(bad)?;
    let lat: f64 = lat.trim().parse().map_err(|_| bad())?;
    let lon: f64 = lon.trim().parse().map_err(|_| bad())?;
    Ok((lat, lon))
}

/// The top-ranked known place whose folded label equals `label`'s, if
/// any. A prefix match is not enough: `--place gy` must not become
/// "Gym".
pub fn known_place(suggestions: &[PlaceSuggestion], label: &str) -> Option<Place> {
    let want = fold_label(label);
    suggestions
        .iter()
        .find(|s| fold_label(&s.place.label) == want)
        .map(|s| s.place.clone())
}

// ---------- places ----------

#[derive(Parser, Debug, Clone, Serialize, Deserialize)]
pub struct PlacesArgs {
    /// Narrow to places whose label or address starts with these words.
    pub query: Option<String>,
    /// At most this many rows (0 = all).
    #[arg(long, default_value_t = 0)]
    pub limit: usize,
    #[arg(long)]
    pub json: bool,
}

/// The places already on items, newest use first (`spec/place-plan.md`
/// "Reuse"): one row per distinct place, with its address and how many
/// items carry it.
pub fn places(doc: &Doc, args: PlacesArgs, out: &mut dyn Write) -> anyhow::Result<()> {
    let rows = doc.place_suggestions(args.query.as_deref().unwrap_or(""), args.limit);
    if args.json {
        write_json(out, &rows)?;
    } else {
        for row in &rows {
            writeln!(out, "{}", format_place_row(row))?;
        }
    }
    Ok(())
}

/// `<label>  <address>  (<count>)`: address only when set, count only
/// past one, and `@<lat>,<lon>` instead of an address when there are
/// coordinates but no address to show for them.
pub fn format_place_row(row: &PlaceSuggestion) -> String {
    let p = &row.place;
    let mut s = p.label.clone();
    match (&p.address, p.lat, p.lon) {
        (Some(addr), _, _) => {
            s.push_str("  ");
            s.push_str(addr);
        }
        (None, Some(lat), Some(lon)) => {
            s.push_str(&format!("  @{lat},{lon}"));
        }
        _ => {}
    }
    if row.count > 1 {
        s.push_str(&format!("  ({})", row.count));
    }
    s
}

// ---------- edit ----------

#[cfg(test)]
mod tests {
    use super::*;

    fn set_args(item_id: &str) -> SetArgs {
        SetArgs {
            item_id: item_id.to_string(),
            text: None,
            list: None,
            lifecycle: None,
            when: None,
            duration: None,
            deadline: None,
            place: None,
            coords: None,
            address: None,
            new_place: false,
            star: None,
            unstar: false,
        }
    }

    #[test]
    fn set_applies_every_field_in_one_call() {
        let doc = Doc::new().unwrap();
        let id = doc.add_item(LIST_INBOX, "draft").unwrap();
        let mut out = Vec::new();
        set(
            &doc,
            SetArgs {
                text: Some("final".into()),
                lifecycle: Some(LifecycleState::InProgress),
                when: Some("2026-11-02T09:00".into()),
                duration: Some("1h30m".into()),
                deadline: Some("2026-11-05".into()),
                place: Some("Office".into()),
                coords: Some("-33.87,151.21".into()),
                star: Some(StarPos::Append),
                ..set_args(&id)
            },
            &mut out,
        )
        .unwrap();
        assert_eq!(String::from_utf8(out).unwrap().trim(), id);
        let item = doc.get_item(&id).unwrap();
        assert_eq!(item.text, "final");
        assert_eq!(item.state, WorkflowState::InProgress);
        assert_eq!(item.when.as_deref(), Some("2026-11-02T09:00"));
        assert_eq!(item.duration, Some(90));
        assert_eq!(item.deadline.as_deref(), Some("2026-11-05"));
        let place = item.place.unwrap();
        assert_eq!(place.label, "Office");
        assert_eq!((place.lat, place.lon), (Some(-33.87), Some(151.21)));
        assert_eq!(doc.focus_view().len(), 1);

        // Clearing with `-`, and --unstar, through the same path.
        let mut out = Vec::new();
        set(
            &doc,
            SetArgs {
                when: Some("-".into()),
                deadline: Some("-".into()),
                place: Some("-".into()),
                unstar: true,
                ..set_args(&id)
            },
            &mut out,
        )
        .unwrap();
        let item = doc.get_item(&id).unwrap();
        assert_eq!(item.when, None);
        assert_eq!(item.duration, None, "clearing when clears the duration");
        assert_eq!(item.deadline, None);
        assert_eq!(item.place, None);
        assert!(doc.focus_view().is_empty());
    }

    #[test]
    fn set_rejects_bad_duration_before_writing_anything() {
        let doc = Doc::new().unwrap();
        let id = doc.add_item(LIST_INBOX, "draft").unwrap();
        let mut out = Vec::new();
        let err = set(
            &doc,
            SetArgs {
                text: Some("changed".into()),
                duration: Some("soon".into()),
                ..set_args(&id)
            },
            &mut out,
        )
        .unwrap_err();
        assert!(err.to_string().contains("duration"), "{err}");
        assert_eq!(doc.get_item(&id).unwrap().text, "draft");
    }

    fn suggestion(
        label: &str,
        coords: Option<(f64, f64)>,
        address: Option<&str>,
        count: usize,
    ) -> PlaceSuggestion {
        PlaceSuggestion {
            place: Place {
                label: label.to_string(),
                lat: coords.map(|c| c.0),
                lon: coords.map(|c| c.1),
                address: address.map(str::to_string),
                reference: None,
            },
            count,
            last_used: 0,
        }
    }

    #[test]
    fn known_place_needs_a_whole_label_match() {
        let rows = [
            suggestion("Gym", Some((1.0, 2.0)), None, 3),
            suggestion("Gymnasium", Some((3.0, 4.0)), None, 1),
        ];
        assert_eq!(known_place(&rows, "gym").unwrap().lat, Some(1.0));
        assert_eq!(known_place(&rows, "GYM ").unwrap().lat, Some(1.0));
        assert!(known_place(&rows, "gy").is_none());
        assert!(known_place(&rows, "home").is_none());
    }

    #[test]
    fn place_rows_show_address_or_coords_and_count() {
        assert_eq!(format_place_row(&suggestion("Home", None, None, 1)), "Home");
        assert_eq!(
            format_place_row(&suggestion("Gym", Some((1.5, -2.0)), None, 2)),
            "Gym  @1.5,-2  (2)"
        );
        assert_eq!(
            format_place_row(&suggestion(
                "Work",
                Some((1.5, -2.0)),
                Some("1 George St"),
                1
            )),
            "Work  1 George St"
        );
    }

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
            created_by: None,
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
