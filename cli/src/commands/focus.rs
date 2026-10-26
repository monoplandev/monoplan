//! Starred commands: `starred` (list, `mv` to reorder), `star`, `unstar`.
//!
//! The curated single-tier Focus lens (`spec/focus.md`), surfaced to the
//! user as "Starred" (the spec's naming note). References items across
//! lists; the referenced item stays in its home list. `monoplan starred`
//! lists the view; `star` / `unstar` / `starred mv` mutate the reference
//! container. `monoplan focus [add|rm|mv]` is the hidden pre-rename
//! spelling: `focus` aliases `starred`, and `add` / `rm` stay as hidden
//! sub-subcommands. The IPC frame is still `DocCmd::Focus(FocusArgs)`;
//! `star` / `unstar` are folded into it in `Cli::run`.
//!
//! Positions on the CLI are **1-based** to match the numbered listing a
//! user sees from `monoplan starred`; they are converted to the doc
//! layer's 0-based visible index here.

use clap::{Parser, Subcommand};
use std::io::Write;

use monoplan_core::{Doc, ItemView, Place};
use serde::{Deserialize, Serialize};

use super::items::write_json;

#[derive(Parser, Debug, Clone, Serialize, Deserialize)]
pub struct FocusArgs {
    #[command(subcommand)]
    pub cmd: Option<FocusCmd>,
    /// Machine-parseable output for the list view.
    #[arg(long, global = true)]
    pub json: bool,
}

/// `monoplan star <item_id> [pos]`.
#[derive(Parser, Debug, Clone, Serialize, Deserialize)]
pub struct StarArgs {
    pub item_id: String,
    /// 1-based position to insert at (default: append).
    pub pos: Option<usize>,
}

#[derive(Subcommand, Debug, Clone, Serialize, Deserialize)]
pub enum FocusCmd {
    /// Hidden pre-rename spelling of `monoplan star`. No-op if the item is
    /// already starred or is not Open (Done / binned items cannot be
    /// starred).
    #[command(hide = true)]
    Add {
        item_id: String,
        /// 1-based position to insert at (default: append).
        pos: Option<usize>,
    },
    /// Hidden pre-rename spelling of `monoplan unstar`.
    #[command(hide = true)]
    Rm { item_id: String },
    /// Reorder a starred item to a 1-based position.
    Mv { item_id: String, pos: usize },
}

pub fn run(doc: &Doc, args: FocusArgs, out: &mut dyn Write) -> anyhow::Result<()> {
    match args.cmd {
        None => show(doc, args.json, out),
        Some(FocusCmd::Add { item_id, pos }) => add(doc, &item_id, pos, out),
        Some(FocusCmd::Rm { item_id }) => rm(doc, &item_id, out),
        Some(FocusCmd::Mv { item_id, pos }) => mv(doc, &item_id, pos, out),
    }
}

fn show(doc: &Doc, json: bool, out: &mut dyn Write) -> anyhow::Result<()> {
    let items = doc.focus_view();
    if json {
        let rows: Vec<FocusItemJson> = items
            .iter()
            .enumerate()
            .map(|(i, item)| item_json(i + 1, item))
            .collect();
        write_json(out, &rows)?;
    } else {
        print_items(out, &items)?;
    }
    Ok(())
}

fn add(doc: &Doc, item_id: &str, pos: Option<usize>, out: &mut dyn Write) -> anyhow::Result<()> {
    // Absent position appends; a 1-based position converts to the 0-based
    // visible index the doc layer expects.
    let index = pos.map(one_based_to_index).unwrap_or(usize::MAX);
    doc.add_to_focus(item_id, index)?;
    writeln!(out, "{item_id}")?;
    Ok(())
}

fn rm(doc: &Doc, item_id: &str, out: &mut dyn Write) -> anyhow::Result<()> {
    doc.remove_from_focus(item_id)?;
    writeln!(out, "{item_id}")?;
    Ok(())
}

fn mv(doc: &Doc, item_id: &str, pos: usize, out: &mut dyn Write) -> anyhow::Result<()> {
    doc.move_in_focus(item_id, one_based_to_index(pos))?;
    writeln!(out, "{item_id}")?;
    Ok(())
}

/// Map a user-facing 1-based position to the 0-based visible index. A `0`
/// (or the absurd) clamps to the first slot; the doc layer clamps the top.
fn one_based_to_index(pos: usize) -> usize {
    pos.saturating_sub(1)
}

fn print_items(out: &mut dyn Write, items: &[ItemView]) -> anyhow::Result<()> {
    for (i, item) in items.iter().enumerate() {
        // 1-based position matches the `star` / `starred mv` argument
        // space. Starred is Open-only, so the box carries the workflow
        // state mark.
        let mark = super::items::state_mark(item.state);
        writeln!(out, "{:>3}  {}  [{mark}] {}", i + 1, item.id, item.text)?;
    }
    Ok(())
}

#[derive(Serialize)]
struct FocusItemJson<'a> {
    pos: usize,
    id: &'a str,
    text: &'a str,
    list_id: &'a str,
    /// Workflow register state name (`spec/data-model.md` "Lifecycle").
    state: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    place: Option<&'a Place>,
}

fn item_json(pos: usize, item: &ItemView) -> FocusItemJson<'_> {
    FocusItemJson {
        pos,
        id: &item.id,
        text: &item.text,
        list_id: &item.list_id,
        state: item.state.name(),
        place: item.place.as_ref(),
    }
}
