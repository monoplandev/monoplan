//! Item commands: `item add`, `item ls`, `item set` (every per-item
//! field as a flag), `item bin` and `item restore`.
//!
//! Every action is a pure function of the doc and its args that writes
//! its output to `out`: the daemon runs it against the resident doc
//! and ships the buffer back to the client (`spec/daemon-plan.md`).

use std::io::{BufRead, IsTerminal, Write};

use clap::{ArgGroup, Parser, ValueEnum};
use monoplan_core::{
    Doc, ItemLifecycle, ItemView, LIST_EVENTS, LIST_INBOX, Place, PlaceSuggestion, WorkflowState,
    fold_label, place_matches, place_suggestions,
};
use serde::{Deserialize, Serialize};

use crate::workspace::{Candidate, HOME_LABEL};

// ---------- add ----------

#[derive(Parser, Debug, Clone, Serialize, Deserialize)]
pub struct AddArgs {
    /// Item text. Use `-` to read one item per non-blank line from stdin.
    pub text: String,
    /// Target list. Defaults to `inbox`.
    #[arg(long, default_value = LIST_INBOX)]
    pub list: String,
    /// Put the new items at the top of the list instead of the end.
    #[arg(long)]
    pub top: bool,
}

/// Daemon side of `add`: `texts` were already expanded from the
/// argument (or stdin) by the client via [`collect_texts`].
/// `top` puts them, in the order given, above the list's open items
/// (one commit); otherwise each is appended.
pub fn add(
    doc: &Doc,
    list: &str,
    texts: &[String],
    top: bool,
    out: &mut dyn Write,
) -> anyhow::Result<()> {
    if texts.is_empty() {
        anyhow::bail!("no item text provided");
    }
    let ids = if top {
        let texts: Vec<&str> = texts.iter().map(String::as_str).collect();
        doc.add_items_at(list, &texts, 0)?
    } else {
        let mut ids = Vec::with_capacity(texts.len());
        for text in texts {
            ids.push(doc.add_item(list, text)?);
        }
        ids
    };
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
pub(super) struct ItemJson<'a> {
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
    /// Pending date suggestion (`when`-shaped), when one is unanswered.
    #[serde(skip_serializing_if = "Option::is_none")]
    when_suggested: Option<&'a str>,
    /// Duration in whole minutes, when set.
    #[serde(skip_serializing_if = "Option::is_none")]
    duration: Option<u32>,
    /// Place (`{label, lat?, lon?, address?, ref?}`), when set.
    #[serde(skip_serializing_if = "Option::is_none")]
    place: Option<&'a Place>,
}

pub(super) fn item_json(item: &ItemView) -> ItemJson<'_> {
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
        when_suggested: item.when_suggested.as_deref(),
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

// ---------- show ----------

#[derive(Parser, Debug, Clone, Serialize, Deserialize)]
pub struct ShowArgs {
    pub item_id: String,
    /// Machine-parseable output.
    #[arg(long)]
    pub json: bool,
}

/// The `ls --json` row plus what only a single item's page needs: its
/// notes (plain text) and its list's name and emoji. `list_name` is
/// absent for the reserved lists, which have no `ListMeta` row; clients
/// label those. `list_icon` is absent when the list has none. `starred`
/// is whether the item shows in Starred (`focus::Lens::has`).
#[derive(Serialize)]
struct ItemDetailJson<'a> {
    #[serde(flatten)]
    item: ItemJson<'a>,
    notes: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    list_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    list_icon: Option<String>,
    starred: bool,
}

/// One item, whatever its state (open, closed or binned): the `ls` row,
/// then its notes under a blank line when it has any.
pub fn show(
    doc: &Doc,
    lens: &super::focus::Lens,
    args: ShowArgs,
    out: &mut dyn Write,
) -> anyhow::Result<()> {
    let Some(item) = doc.get_item(&args.item_id) else {
        anyhow::bail!("item not found: {}", args.item_id);
    };
    if args.json {
        let list = doc.get_list_meta(&item.list_id);
        write_json(
            out,
            &ItemDetailJson {
                item: item_json(&item),
                notes: &item.notes,
                list_icon: list.as_ref().and_then(|l| l.icon.clone()),
                list_name: list.map(|l| l.name),
                starred: lens.has(&item),
            },
        )?;
    } else {
        print_items(out, std::slice::from_ref(&item))?;
        if !item.notes.is_empty() {
            writeln!(out, "\n{}", item.notes.trim_end())?;
        }
    }
    Ok(())
}

// ---------- places ----------

#[derive(Parser, Debug, Clone, Serialize, Deserialize)]
pub struct PlacesArgs {
    /// Only places matching this text (label or address); all when omitted.
    pub query: Option<String>,
    /// At most this many; 0 is no limit.
    #[arg(short = 'n', long = "limit", default_value_t = 0)]
    pub limit: usize,
    /// Machine-parseable output.
    #[arg(long)]
    pub json: bool,
    /// The Open items that carry a place, instead of the places: every
    /// workspace this install can open, rows tagged with theirs.
    #[arg(long)]
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub items: bool,
}

#[derive(Serialize)]
struct KnownPlaceJson<'a> {
    #[serde(flatten)]
    place: &'a Place,
    /// Items (not binned) carrying this place.
    count: usize,
    /// Unix millis of the newest use.
    last_used: i64,
}

/// The account's known places (`spec/place-plan.md` "Reuse"): the
/// places already on items of every open doc, deduped, most recently
/// used first. What a place field offers before any geocoder is asked.
pub fn places(open_docs: &[&Doc], args: PlacesArgs, out: &mut dyn Write) -> anyhow::Result<()> {
    let rows = place_suggestions(
        open_docs.iter().copied(),
        args.query.as_deref().unwrap_or(""),
        args.limit,
    );
    if args.json {
        let json: Vec<KnownPlaceJson> = rows
            .iter()
            .map(|r| KnownPlaceJson {
                place: &r.place,
                count: r.count,
                last_used: r.last_used,
            })
            .collect();
        write_json(out, &json)?;
    } else {
        for r in &rows {
            match &r.place.address {
                Some(address) => writeln!(out, "{}  ({address})  x{}", r.place.label, r.count)?,
                None => writeln!(out, "{}  x{}", r.place.label, r.count)?,
            }
        }
    }
    Ok(())
}

/// A `places --items` row: the `item ls --json` row plus where it is.
#[derive(Serialize)]
struct PlacedItemJson<'a> {
    #[serde(flatten)]
    item: ItemJson<'a>,
    /// Absent for the reserved lists' ids no client labels.
    #[serde(skip_serializing_if = "Option::is_none")]
    list_name: Option<String>,
    /// The row's workspace, when the caller tagged its docs (the
    /// daemon does). `doc_name` is absent for Home and an unnamed doc.
    #[serde(skip_serializing_if = "Option::is_none")]
    doc_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    doc_name: Option<&'a str>,
}

/// `places --items`: the Open items (not closed, not binned) that carry
/// a place, over `docs`, each doc's in its own item order; `query`
/// keeps those whose place matches, as for the places themselves, and
/// `limit` caps the rows. A place without coordinates counts: what can
/// be pinned is the reader's question. What the iOS map reads.
pub fn placed_items(
    docs: &[(Option<&Candidate>, &Doc)],
    args: &PlacesArgs,
    out: &mut dyn Write,
) -> anyhow::Result<()> {
    let query = args.query.as_deref().unwrap_or("");
    let mut rows: Vec<(Option<&Candidate>, ItemView, Option<String>)> = Vec::new();
    for (tag, doc) in docs {
        let names: std::collections::HashMap<String, String> = doc
            .all_lists()
            .into_iter()
            .map(|l| (l.id, l.name))
            .collect();
        for item in doc.all_items() {
            let placed = item.place.as_ref().is_some_and(|p| place_matches(p, query));
            if !item.is_open() || !placed {
                continue;
            }
            // The reserved lists have no `ListMeta` row, so they are
            // named here as the clients label them.
            let list_name = match item.list_id.as_str() {
                LIST_INBOX => Some("Inbox".to_string()),
                LIST_EVENTS => Some("Events".to_string()),
                other => names.get(other).cloned(),
            };
            rows.push((*tag, item, list_name));
        }
    }
    if args.limit > 0 {
        rows.truncate(args.limit);
    }
    if args.json {
        let json: Vec<PlacedItemJson> = rows
            .iter()
            .map(|(tag, item, list_name)| PlacedItemJson {
                item: item_json(item),
                list_name: list_name.clone(),
                doc_id: tag.map(|c| c.doc_id.0.to_string()),
                doc_name: tag.and_then(|c| c.name.as_deref()),
            })
            .collect();
        write_json(out, &json)?;
    } else {
        for (tag, item, _) in &rows {
            let doc = tag
                .map(|c| format!("  [{}]", c.name.as_deref().unwrap_or(HOME_LABEL)))
                .unwrap_or_default();
            writeln!(
                out,
                "{}  {} {}{}{doc}",
                item.id,
                row_box(item),
                item.text,
                date_tags(item)
            )?;
        }
    }
    Ok(())
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

#[derive(Parser, Debug, Clone, Serialize, Deserialize)]
pub struct DuplicateArgs {
    pub item_id: String,
    /// Star the copy too when the original is starred, directly below
    /// it in Starred.
    #[arg(long)]
    #[serde(default)]
    pub star: bool,
}

/// Duplicate an item beside itself (`Doc::duplicate_item`); prints the
/// copy's id, as `add` prints a new item's. With `star`, a starred
/// original's copy is starred in the slot under it (Home's lens, as
/// `item set --star`); an unstarred original's copy is left unstarred.
pub fn duplicate(
    doc: &Doc,
    lens: &super::focus::Lens,
    args: DuplicateArgs,
    out: &mut dyn Write,
) -> anyhow::Result<()> {
    let copy = doc.duplicate_item(&args.item_id)?;
    if args.star
        && let Some(at) = lens.position(&args.item_id)
    {
        lens.star(doc, &copy, at + 1)?;
    }
    writeln!(out, "{copy}")?;
    Ok(())
}

// ---------- set ----------

/// Where `--star` puts the item: a 1-based slot (the top when none is
/// given, as every client's star does), or the end of Starred.
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
    /// 1-based position among the list's open items: with --list, where the item lands there; alone, a reorder within its own list. Past the end clamps to the end.
    #[arg(long, group = "field", value_name = "POS")]
    #[serde(default)]
    pub pos: Option<usize>,
    /// Workflow state: backlog | todo | in_progress | review | done | cancelled.
    #[arg(long, group = "field", value_name = "STATE")]
    pub lifecycle: Option<LifecycleState>,
    /// Planned date, `YYYY-MM-DD` (all-day) or `YYYY-MM-DDTHH:MM` (timed, floating); `-` clears it and the duration.
    #[arg(long, group = "field", value_name = "DATE|-")]
    pub when: Option<String>,
    /// Pending date suggestion, same shape as --when: a date read out of the title, held until answered; `-` dismisses it. `--when` with the same value accepts it.
    #[arg(long, group = "field", value_name = "DATE|-")]
    #[serde(default)]
    pub when_suggested: Option<String>,
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
    /// Star the item: at the top of Starred, or at a 1-based POS (`end` appends). No-op if already starred.
    #[arg(long, group = "field", value_name = "POS", num_args = 0..=1, default_missing_value = "1", value_parser = parse_star_pos, conflicts_with = "unstar")]
    pub star: Option<StarPos>,
    /// Unstar the item (the item itself is untouched).
    #[arg(long, group = "field")]
    pub unstar: bool,
}

/// `open_docs` is every open doc, `doc` included: known places are
/// account-scoped (`spec/place-plan.md` "Reuse"), so `--place gym` in
/// one workspace finds the Gym geocoded in another. `lens` is where
/// `--star` / `--unstar` write: Starred is Home's, whatever `doc` is.
pub fn set(
    doc: &Doc,
    open_docs: &[&Doc],
    lens: &super::focus::Lens,
    args: SetArgs,
    out: &mut dyn Write,
) -> anyhow::Result<()> {
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
    if args.list.is_some() || args.pos.is_some() {
        // `--pos` alone reorders within the item's own list.
        let list = match &args.list {
            Some(list) => list.clone(),
            None => {
                doc.get_item(id)
                    .ok_or_else(|| anyhow::anyhow!("item not found: {id}"))?
                    .list_id
            }
        };
        // No position appends (target_index = current length);
        // `move_item` clamps to the existing range anyway. The index
        // is the slot the item ends up in: `move_item` counts it over
        // the projection without the moving item.
        let target_idx = match args.pos {
            Some(pos) => super::focus::one_based_to_index(pos),
            None => doc.items_in_list(&list, true).len(),
        };
        doc.move_item(id, &list, target_idx)?;
    }
    if let Some(state) = args.lifecycle {
        doc.set_item_lifecycle(id, state.into())?;
    }
    if let Some(when) = &args.when {
        doc.set_item_when(id, clear_or(when))?;
    }
    if let Some(suggested) = &args.when_suggested {
        doc.set_item_when_suggested(id, clear_or(suggested))?;
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
            // of any open workspace
            // (case-insensitively, `spec/place-plan.md` "Reuse") takes
            // that place whole, coordinates and address included, so
            // `--place gym` finds the Gym the web client geocoded;
            // --coords / --address / --new-place write exactly what was
            // given. Validation lives in the core.
            Some(label) => {
                let known = if coords.is_none() && args.address.is_none() && !args.new_place {
                    known_place(
                        &place_suggestions(open_docs.iter().copied(), label, 0),
                        label,
                    )
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
        lens.star(doc, id, index)?;
    }
    if args.unstar {
        lens.unstar(id)?;
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

// ---------- edit ----------

#[cfg(test)]
mod tests {
    use super::super::focus::Lens;
    use super::*;

    fn set_args(item_id: &str) -> SetArgs {
        SetArgs {
            item_id: item_id.to_string(),
            text: None,
            list: None,
            pos: None,
            lifecycle: None,
            when: None,
            when_suggested: None,
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
    fn places_lists_the_known_places_across_open_docs() {
        let home = Doc::new().unwrap();
        let work = Doc::new().unwrap();
        let a = home.add_item(LIST_INBOX, "lift").unwrap();
        home.set_item_place(
            &a,
            Some(&Place {
                label: "Gym".into(),
                lat: Some(1.5),
                lon: Some(2.5),
                address: Some("1 High St".into()),
                reference: None,
            }),
        )
        .unwrap();
        let b = work.add_item(LIST_INBOX, "standup").unwrap();
        work.set_item_place(
            &b,
            Some(&Place {
                label: "Office".into(),
                lat: None,
                lon: None,
                address: None,
                reference: None,
            }),
        )
        .unwrap();

        let run = |query: Option<&str>| {
            let mut out = Vec::new();
            places(
                &[&home, &work],
                PlacesArgs {
                    query: query.map(str::to_string),
                    limit: 0,
                    json: true,
                    items: false,
                },
                &mut out,
            )
            .unwrap();
            serde_json::from_slice::<serde_json::Value>(&out).unwrap()
        };
        let all = run(None);
        let mut labels: Vec<&str> = all
            .as_array()
            .unwrap()
            .iter()
            .map(|r| r["label"].as_str().unwrap())
            .collect();
        labels.sort();
        assert_eq!(labels, ["Gym", "Office"]);

        let gym = run(Some("gy"));
        assert_eq!(gym.as_array().unwrap().len(), 1);
        assert_eq!(gym[0]["label"], "Gym");
        assert_eq!(gym[0]["lat"], 1.5);
        assert_eq!(gym[0]["address"], "1 High St");
        assert_eq!(gym[0]["count"], 1);
        assert!(gym[0]["last_used"].is_i64());
    }

    /// The frame `Request.places` hand-encodes.
    #[test]
    fn places_frame_from_the_apps_decodes() {
        use crate::commands::DocCmd;
        let cmd: DocCmd = serde_json::from_value(
            serde_json::json!({"Places": {"query": "gy", "limit": 8, "json": true}}),
        )
        .unwrap();
        assert!(
            matches!(cmd, DocCmd::Places(a) if a.query.as_deref() == Some("gy") && a.limit == 8)
        );
    }

    /// The frame `Request.placedItems` hand-encodes.
    #[test]
    fn placed_items_frame_from_the_apps_decodes() {
        use crate::commands::DocCmd;
        let cmd: DocCmd = serde_json::from_value(serde_json::json!(
            {"Places": {"query": null, "limit": 0, "json": true, "items": true}}
        ))
        .unwrap();
        assert!(matches!(cmd, DocCmd::Places(a) if a.items && a.query.is_none()));
    }

    #[test]
    fn placed_items_are_the_open_ones_with_a_place_tagged_by_workspace() {
        use crate::workspace::Candidate;
        use monoplan_core::DocId;
        let place = |label: &str, coords: Option<(f64, f64)>| Place {
            label: label.into(),
            lat: coords.map(|c| c.0),
            lon: coords.map(|c| c.1),
            address: None,
            reference: None,
        };
        let home = Doc::new().unwrap();
        let gym = home.add_item(LIST_INBOX, "lift").unwrap();
        home.set_item_place(&gym, Some(&place("Gym", Some((1.5, 2.5)))))
            .unwrap();
        home.add_item(LIST_INBOX, "no place").unwrap();
        let done = home.add_item(LIST_INBOX, "done there").unwrap();
        home.set_item_place(&done, Some(&place("Gym", Some((1.5, 2.5)))))
            .unwrap();
        home.set_item_lifecycle(&done, ItemLifecycle::Done).unwrap();
        let binned = home.add_item(LIST_INBOX, "binned there").unwrap();
        home.set_item_place(&binned, Some(&place("Gym", Some((1.5, 2.5)))))
            .unwrap();
        home.set_item_binned(&binned, true).unwrap();

        let work = Doc::new().unwrap();
        let list = work.add_list("Ops").unwrap();
        let standup = work.add_item(&list, "standup").unwrap();
        work.set_item_place(&standup, Some(&place("Office", None)))
            .unwrap();

        let home_tag = Candidate {
            doc_id: DocId(uuid::Uuid::from_u128(1)),
            name: None,
        };
        let work_tag = Candidate {
            doc_id: DocId(uuid::Uuid::from_u128(2)),
            name: Some("Work".into()),
        };
        let run = |query: Option<&str>| {
            let mut out = Vec::new();
            placed_items(
                &[(Some(&home_tag), &home), (Some(&work_tag), &work)],
                &PlacesArgs {
                    query: query.map(str::to_string),
                    limit: 0,
                    json: true,
                    items: true,
                },
                &mut out,
            )
            .unwrap();
            serde_json::from_slice::<serde_json::Value>(&out).unwrap()
        };
        let all = run(None);
        let rows = all.as_array().unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0]["id"], gym.as_str());
        assert_eq!(rows[0]["place"]["lat"], 1.5);
        assert_eq!(rows[0]["list_name"], "Inbox");
        assert_eq!(rows[0]["doc_id"], home_tag.doc_id.0.to_string());
        assert!(rows[0].get("doc_name").is_none());
        assert_eq!(rows[1]["id"], standup.as_str());
        assert_eq!(rows[1]["list_name"], "Ops");
        assert_eq!(rows[1]["doc_name"], "Work");

        let office = run(Some("off"));
        assert_eq!(office.as_array().unwrap().len(), 1);
        assert_eq!(office[0]["id"], standup.as_str());
    }

    #[test]
    fn show_prints_one_item_with_its_notes_and_list_name() {
        let doc = Doc::new().unwrap();
        let list = doc.add_list("Work").unwrap();
        let id = doc.add_item(&list, "draft").unwrap();
        doc.apply_notes_delta(
            &id,
            &[monoplan_core::NotesDeltaOp::Insert {
                insert: "line one\nline two".into(),
            }],
        )
        .unwrap();
        doc.set_item_when(&id, Some("2026-09-12")).unwrap();

        let mut out = Vec::new();
        let args = |json| ShowArgs {
            item_id: id.clone(),
            json,
        };
        show(&doc, &Lens::home(&doc), args(true), &mut out).unwrap();
        let v: serde_json::Value = serde_json::from_slice(&out).unwrap();
        assert_eq!(v["id"], id.as_str());
        assert_eq!(v["text"], "draft");
        assert_eq!(v["notes"], "line one\nline two");
        assert_eq!(v["list_name"], "Work");
        assert!(v.get("list_icon").is_none());
        doc.set_list_icon(&list, "🧰").unwrap();
        let mut out = Vec::new();
        show(&doc, &Lens::home(&doc), args(true), &mut out).unwrap();
        let v: serde_json::Value = serde_json::from_slice(&out).unwrap();
        assert_eq!(v["list_icon"], "🧰");
        assert_eq!(v["when"], "2026-09-12");

        let mut out = Vec::new();
        show(&doc, &Lens::home(&doc), args(false), &mut out).unwrap();
        let text = String::from_utf8(out).unwrap();
        assert!(text.ends_with("\n\nline one\nline two\n"), "{text:?}");

        // The Inbox has no ListMeta row: no name on the wire.
        let inbox = doc.add_item(LIST_INBOX, "loose").unwrap();
        let mut out = Vec::new();
        show(
            &doc,
            &Lens::home(&doc),
            ShowArgs {
                item_id: inbox,
                json: true,
            },
            &mut out,
        )
        .unwrap();
        let v: serde_json::Value = serde_json::from_slice(&out).unwrap();
        assert!(v.get("list_name").is_none());
        assert_eq!(v["notes"], "");

        let err = show(
            &doc,
            &Lens::home(&doc),
            ShowArgs {
                item_id: "nope".into(),
                json: true,
            },
            &mut Vec::new(),
        )
        .unwrap_err();
        assert_eq!(err.to_string(), "item not found: nope");
    }

    /// The frame the native clients hand-encode to capture an item
    /// (`Request.addItem` in MonoplanMacKit): one text, and the reply
    /// is the new id on a line, which the apps open the item by.
    #[test]
    fn add_frame_from_the_apps_decodes_and_applies() {
        use crate::commands::DocCmd;
        let doc = Doc::new().unwrap();
        let cmd: DocCmd = serde_json::from_value(serde_json::json!({"Add": {
            "list": "inbox", "texts": ["Call the bank"],
        }}))
        .unwrap();
        let DocCmd::Add { list, texts, top } = cmd else {
            panic!("not an Add frame")
        };
        assert!(!top, "an absent `top` appends");
        let mut out = Vec::new();
        add(&doc, &list, &texts, top, &mut out).unwrap();
        let id = String::from_utf8(out).unwrap();
        let item = doc.get_item(id.trim()).unwrap();
        assert_eq!(item.text, "Call the bank");
        assert_eq!(item.list_id, LIST_INBOX);
        assert_eq!(item.state.name(), "backlog");

        // The iOS capture's frame: `top` puts the item first.
        let cmd: DocCmd = serde_json::from_value(serde_json::json!({"Add": {
            "list": "inbox", "texts": ["Now this"], "top": true,
        }}))
        .unwrap();
        let mut out = Vec::new();
        crate::commands::run_doc(cmd, &doc, &[&doc], &Lens::home(&doc), &mut out).unwrap();
        let first = String::from_utf8(out).unwrap();
        assert_eq!(
            doc.open_item_ids(LIST_INBOX),
            vec![first.trim().to_string(), id.trim().to_string()]
        );
    }

    /// `Request.setText` with a suggestion edit, and
    /// `Request.setWhenSuggested`: the title and the date suggestion it
    /// raised in one frame, then the two answers (a dismissal clears
    /// the register; a `when` of the same value is the yes, and clears
    /// it too).
    #[test]
    fn when_suggested_frame_from_the_apps_decodes_and_applies() {
        use crate::commands::DocCmd;
        let doc = Doc::new().unwrap();
        let id = doc.add_item(LIST_INBOX, "dentist").unwrap();
        let send = |text: Option<&str>, when: Option<&str>, suggested: Option<&str>| {
            let cmd: DocCmd = serde_json::from_value(serde_json::json!({"Set": {
                "item_id": id, "text": text, "list": null, "lifecycle": null,
                "when": when, "when_suggested": suggested, "duration": null,
                "deadline": null, "place": null, "coords": null, "address": null,
                "new_place": false, "star": null, "unstar": false,
            }}))
            .unwrap();
            let DocCmd::Set(args) = cmd else {
                panic!("not a Set frame")
            };
            set(&doc, &[&doc], &Lens::home(&doc), *args, &mut Vec::new()).unwrap();
            doc.get_item(&id).unwrap()
        };
        let item = send(Some("dentist friday 8:30"), None, Some("2026-11-20T08:30"));
        assert_eq!(item.text, "dentist friday 8:30");
        assert_eq!(item.when_suggested.as_deref(), Some("2026-11-20T08:30"));
        assert_eq!(item.when, None);
        let item = send(None, None, Some("-"));
        assert_eq!(item.when_suggested, None);
        send(None, None, Some("2026-11-20T08:30"));
        let item = send(None, Some("2026-11-20T08:30"), None);
        assert_eq!(item.when.as_deref(), Some("2026-11-20T08:30"));
        assert_eq!(item.when_suggested, None);
    }

    /// The frame the native clients hand-encode to tick and reopen an
    /// item (`Request.setDone` in MonoplanMacKit): unset fields as nil,
    /// the state by its variant name.
    #[test]
    fn set_lifecycle_frame_from_the_apps_decodes_and_applies() {
        use crate::commands::DocCmd;
        let doc = Doc::new().unwrap();
        let id = doc.add_item(LIST_INBOX, "tick me").unwrap();
        for (state, done) in [("Done", true), ("Backlog", false)] {
            let cmd: DocCmd = serde_json::from_value(serde_json::json!({"Set": {
                "item_id": id, "text": null, "list": null, "lifecycle": state,
                "when": null, "duration": null, "deadline": null, "place": null,
                "coords": null, "address": null, "new_place": false,
                "star": null, "unstar": false,
            }}))
            .unwrap();
            let DocCmd::Set(args) = cmd else {
                panic!("not a Set frame")
            };
            set(&doc, &[&doc], &Lens::home(&doc), *args, &mut Vec::new()).unwrap();
            assert_eq!(doc.get_item(&id).unwrap().is_done(), done);
        }
        // `Request.setLifecycle`: every variant name the apps send.
        for (state, want) in [
            ("Todo", "todo"),
            ("InProgress", "in_progress"),
            ("Review", "review"),
            ("Cancelled", "cancelled"),
            ("Backlog", "backlog"),
        ] {
            let cmd: DocCmd = serde_json::from_value(serde_json::json!({"Set": {
                "item_id": id, "text": null, "list": null, "lifecycle": state,
                "when": null, "duration": null, "deadline": null, "place": null,
                "coords": null, "address": null, "new_place": false,
                "star": null, "unstar": false,
            }}))
            .unwrap();
            let DocCmd::Set(args) = cmd else {
                panic!("not a Set frame")
            };
            set(&doc, &[&doc], &Lens::home(&doc), *args, &mut Vec::new()).unwrap();
            assert_eq!(doc.get_item(&id).unwrap().state.name(), want);
        }
        // `Request.moveItem`: the same frame with `list` set instead.
        let list = doc.add_list("Work").unwrap();
        let cmd: DocCmd = serde_json::from_value(serde_json::json!({"Set": {
            "item_id": id, "text": null, "list": list, "lifecycle": null,
            "when": null, "duration": null, "deadline": null, "place": null,
            "coords": null, "address": null, "new_place": false,
            "star": null, "unstar": false,
        }}))
        .unwrap();
        let DocCmd::Set(args) = cmd else {
            panic!("not a Set frame")
        };
        set(&doc, &[&doc], &Lens::home(&doc), *args, &mut Vec::new()).unwrap();
        assert_eq!(doc.get_item(&id).unwrap().list_id, list);
        // `Request.setWhen` / `setDuration`: strings as the flags take
        // them, `-` to clear.
        let frame = |when: Option<&str>, duration: Option<&str>| {
            let cmd: DocCmd = serde_json::from_value(serde_json::json!({"Set": {
                "item_id": id, "text": null, "list": null, "lifecycle": null,
                "when": when, "duration": duration, "deadline": null, "place": null,
                "coords": null, "address": null, "new_place": false,
                "star": null, "unstar": false,
            }}))
            .unwrap();
            let DocCmd::Set(args) = cmd else {
                panic!("not a Set frame")
            };
            set(&doc, &[&doc], &Lens::home(&doc), *args, &mut Vec::new()).unwrap();
            let item = doc.get_item(&id).unwrap();
            (item.when, item.duration)
        };
        assert_eq!(
            frame(Some("2026-09-12T14:00"), Some("90")),
            (Some("2026-09-12T14:00".into()), Some(90))
        );
        assert_eq!(
            frame(None, Some("-")),
            (Some("2026-09-12T14:00".into()), None)
        );
        assert_eq!(
            frame(Some("2026-09-12"), Some("4320")),
            (Some("2026-09-12".into()), Some(4320))
        );
        assert_eq!(frame(Some("-"), None), (None, None));
    }

    /// `Request.setBinned`, `Request.deleteBinned`, `Request.bin` and
    /// `Request.emptyBin` in MonoplanMacKit: bin, restore, then bin
    /// again and hard-delete from the bin; then list the bin and empty
    /// it.
    /// `Request.reorderItem` / `moveItem(…, pos:)`: `pos` is the
    /// 1-based slot the item ends up in among the list's open items,
    /// whichever way it travels; the frames without it still decode.
    #[test]
    fn pos_frame_from_the_apps_reorders_and_places() {
        use super::super::DocCmd;
        let doc = Doc::new().unwrap();
        let ids: Vec<String> = ["a", "b", "c", "d"]
            .iter()
            .map(|t| doc.add_item(LIST_INBOX, t).unwrap())
            .collect();
        let order = |list: &str| -> Vec<String> {
            doc.items_in_list(list, false)
                .into_iter()
                .map(|i| i.text)
                .collect()
        };
        let frame = |id: &str, list: Option<&str>, pos: usize| {
            let cmd: DocCmd = serde_json::from_value(serde_json::json!({"Set": {
                "item_id": id, "text": null, "list": list, "pos": pos, "lifecycle": null,
                "when": null, "duration": null, "deadline": null, "place": null,
                "coords": null, "address": null, "new_place": false,
                "star": null, "unstar": false,
            }}))
            .unwrap();
            let DocCmd::Set(args) = cmd else {
                panic!("not a Set frame")
            };
            set(&doc, &[&doc], &Lens::home(&doc), *args, &mut Vec::new())
        };
        // Down: a to the third slot.
        frame(&ids[0], None, 3).unwrap();
        assert_eq!(order(LIST_INBOX), ["b", "c", "a", "d"]);
        // Up: d to the first.
        frame(&ids[3], None, 1).unwrap();
        assert_eq!(order(LIST_INBOX), ["d", "b", "c", "a"]);
        // Past the end clamps to the end.
        frame(&ids[3], None, 99).unwrap();
        assert_eq!(order(LIST_INBOX), ["b", "c", "a", "d"]);
        // With a list: lands at that slot there.
        let work = doc.add_list("Work").unwrap();
        frame(&ids[0], Some(&work), 1).unwrap();
        frame(&ids[1], Some(&work), 1).unwrap();
        frame(&ids[2], Some(&work), 2).unwrap();
        assert_eq!(order(&work), ["b", "c", "a"]);
        assert_eq!(order(LIST_INBOX), ["d"]);
        assert!(frame("nope", None, 1).is_err());
    }

    #[test]
    fn bin_frames_from_the_apps_decode_and_apply() {
        use crate::commands::{DocCmd, run_doc};
        let doc = Doc::new().unwrap();
        let id = doc.add_item(LIST_INBOX, "bin me").unwrap();
        let run = |frame: serde_json::Value| {
            let cmd: DocCmd = serde_json::from_value(frame).unwrap();
            run_doc(cmd, &doc, &[&doc], &Lens::home(&doc), &mut Vec::new()).unwrap();
        };
        run(serde_json::json!({"ItemBin": {"item_id": id}}));
        assert!(doc.get_item(&id).unwrap().binned_at.is_some());
        run(serde_json::json!({"Restore": {"item_id": id}}));
        assert!(doc.get_item(&id).unwrap().binned_at.is_none());
        run(serde_json::json!({"ItemBin": {"item_id": id}}));
        run(serde_json::json!({"Bin": {"sub": {"Rm": {"item_id": id}}, "json": false}}));
        assert!(doc.get_item(&id).is_none());

        let read = |frame: serde_json::Value| {
            let cmd: DocCmd = serde_json::from_value(frame).unwrap();
            let mut out = Vec::new();
            run_doc(cmd, &doc, &[&doc], &Lens::home(&doc), &mut out).unwrap();
            String::from_utf8(out).unwrap()
        };
        let a = doc.add_item(LIST_INBOX, "a").unwrap();
        let b = doc.add_item(LIST_INBOX, "b").unwrap();
        run(serde_json::json!({"ItemBin": {"item_id": a}}));
        run(serde_json::json!({"ItemBin": {"item_id": b}}));
        let shown: serde_json::Value =
            serde_json::from_str(&read(serde_json::json!({"Bin": {"sub": "Show", "json": true}})))
                .unwrap();
        let rows = shown.as_array().unwrap();
        assert_eq!(rows.len(), 2);
        for row in rows {
            assert!(row["id"].is_string() && row["text"].is_string());
            assert_eq!(row["list_id"], LIST_INBOX);
            assert!(row["binned_at"].as_i64().unwrap() > 0);
        }
        assert_eq!(
            read(serde_json::json!({"Bin": {"sub": "Empty", "json": false}})).trim(),
            "removed 2"
        );
        assert!(doc.binned_items().is_empty());
    }

    /// `Request.duplicateItem` in MonoplanMacKit: the reply is the
    /// copy's id on a line.
    #[test]
    fn duplicate_frame_from_the_apps_decodes_and_applies() {
        use crate::commands::{DocCmd, run_doc};
        let doc = Doc::new().unwrap();
        let id = doc.add_item(LIST_INBOX, "twice").unwrap();
        let cmd: DocCmd =
            serde_json::from_value(serde_json::json!({"Duplicate": {"item_id": id}})).unwrap();
        let mut out = Vec::new();
        run_doc(cmd, &doc, &[&doc], &Lens::home(&doc), &mut out).unwrap();
        let copy = String::from_utf8(out).unwrap();
        assert_eq!(doc.get_item(copy.trim()).unwrap().text, "twice");
        assert!(doc.focus_refs().is_empty(), "no `star`, no star");
        assert_eq!(
            doc.open_item_ids(LIST_INBOX),
            vec![id, copy.trim().to_string()]
        );
    }

    /// `Request.duplicateItem` with `star` (the iOS Starred page's
    /// menu): the copy is starred directly under a starred original,
    /// and not at all under an unstarred one.
    #[test]
    fn duplicate_star_frame_stars_the_copy_under_the_original() {
        use crate::commands::{DocCmd, run_doc};
        let doc = Doc::new().unwrap();
        let a = doc.add_item(LIST_INBOX, "a").unwrap();
        let b = doc.add_item(LIST_INBOX, "b").unwrap();
        let plain = doc.add_item(LIST_INBOX, "plain").unwrap();
        doc.add_to_focus(&a, usize::MAX).unwrap();
        doc.add_to_focus(&b, usize::MAX).unwrap();
        let run = |id: &str| {
            let cmd: DocCmd = serde_json::from_value(
                serde_json::json!({"Duplicate": {"item_id": id, "star": true}}),
            )
            .unwrap();
            let mut out = Vec::new();
            run_doc(cmd, &doc, &[&doc], &Lens::home(&doc), &mut out).unwrap();
            String::from_utf8(out).unwrap().trim().to_string()
        };
        let copy = run(&a);
        let starred: Vec<String> = doc.focus_entries().into_iter().map(|e| e.item_id).collect();
        assert_eq!(starred, vec![a, copy, b]);
        run(&plain);
        assert_eq!(doc.focus_entries().len(), 3);
    }

    /// `Request.setStarred` in MonoplanMacKit: `star` at slot 1 (the
    /// top of Starred), or `unstar` true. In a workspace other than Home the ref
    /// is Home's (`focus::Lens`), and `item show` reports it.
    #[test]
    fn star_frame_from_the_apps_writes_home_from_any_workspace() {
        use crate::commands::DocCmd;
        let home = Doc::new().unwrap();
        let work = Doc::new().unwrap();
        let work_id = monoplan_core::DocId(uuid::Uuid::from_u128(2));
        let id = work.add_item(LIST_INBOX, "shared task").unwrap();
        // Already starred, to be starred above.
        let first = home.add_item(LIST_INBOX, "older star").unwrap();
        home.add_to_focus(&first, usize::MAX).unwrap();
        let lens = Lens::foreign(&home, work_id);
        let starred = |lens: &Lens| {
            let mut out = Vec::new();
            let args = ShowArgs {
                item_id: id.clone(),
                json: true,
            };
            show(&work, lens, args, &mut out).unwrap();
            let v: serde_json::Value = serde_json::from_slice(&out).unwrap();
            v["starred"].as_bool().unwrap()
        };
        assert!(!starred(&lens));
        let top = serde_json::json!({"At": 1});
        for (star, unstar, want) in [(Some(top), false, true), (None, true, false)] {
            let cmd: DocCmd = serde_json::from_value(serde_json::json!({"Set": {
                "item_id": id, "text": null, "list": null, "lifecycle": null,
                "when": null, "duration": null, "deadline": null, "place": null,
                "coords": null, "address": null, "new_place": false,
                "star": star, "unstar": unstar,
            }}))
            .unwrap();
            let DocCmd::Set(args) = cmd else {
                panic!("not a Set frame")
            };
            set(&work, &[&home, &work], &lens, *args, &mut Vec::new()).unwrap();
            assert_eq!(starred(&lens), want);
            let order: Vec<String> = home
                .focus_entries()
                .into_iter()
                .map(|e| e.item_id)
                .collect();
            if want {
                assert_eq!(order, vec![id.clone(), first.clone()]);
            } else {
                assert_eq!(order, vec![first.clone()]);
            }
            assert!(work.focus_entries().is_empty());
        }
    }

    #[test]
    fn set_applies_every_field_in_one_call() {
        let doc = Doc::new().unwrap();
        let id = doc.add_item(LIST_INBOX, "draft").unwrap();
        let mut out = Vec::new();
        set(
            &doc,
            &[&doc],
            &Lens::home(&doc),
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
            &[&doc],
            &Lens::home(&doc),
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
            &[&doc],
            &Lens::home(&doc),
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
    fn place_label_resolves_against_every_open_doc() {
        let home = Doc::new().unwrap();
        let work = Doc::new().unwrap();
        let geocoded = work.add_item(LIST_INBOX, "standup").unwrap();
        work.set_item_place(
            &geocoded,
            Some(&Place {
                label: "Office".into(),
                lat: Some(-33.87),
                lon: Some(151.21),
                address: Some("1 George St".into()),
                reference: None,
            }),
        )
        .unwrap();
        let id = home.add_item(LIST_INBOX, "drop off keys").unwrap();
        let mut out = Vec::new();
        set(
            &home,
            &[&home, &work],
            &Lens::home(&home),
            SetArgs {
                place: Some("office".into()),
                ..set_args(&id)
            },
            &mut out,
        )
        .unwrap();
        let place = home.get_item(&id).unwrap().place.unwrap();
        assert_eq!(place.label, "Office");
        assert_eq!(place.lat, Some(-33.87));
        assert_eq!(place.address.as_deref(), Some("1 George St"));
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
            when_suggested: None,
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
