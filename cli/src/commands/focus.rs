//! Focus lens commands: list (default action), add, rm, mv.
//!
//! The curated single-tier Focus lens (`spec/focus.md`). References items
//! across lists; the referenced item stays in its home list. `monoplan
//! focus` with no sub-subcommand lists the view; `add` / `rm` / `mv`
//! mutate the reference container.
//!
//! Positions on the CLI are **1-based** to match the numbered listing a
//! user sees from `monoplan focus`; they are converted to the doc layer's
//! 0-based visible index here.

use clap::{Parser, Subcommand};
use monoplan_core::{ItemView, Place};
use serde::Serialize;

use crate::sync::Session;

use super::items::print_json;

#[derive(Parser, Debug)]
pub struct FocusArgs {
    #[command(subcommand)]
    pub cmd: Option<FocusCmd>,
    /// Machine-parseable output for the list view.
    #[arg(long, global = true)]
    pub json: bool,
}

#[derive(Subcommand, Debug)]
pub enum FocusCmd {
    /// Add an item reference to Focus. Defaults to append; pass a 1-based
    /// position to insert. No-op if the item is already focused or is not
    /// Open (Done / binned items cannot be focused).
    Add {
        item_id: String,
        /// 1-based position to insert at (default: append).
        pos: Option<usize>,
    },
    /// Remove an item's reference from Focus. The item is untouched.
    Rm { item_id: String },
    /// Reorder an item's reference to a 1-based position within Focus.
    Mv { item_id: String, pos: usize },
}

pub async fn run(args: FocusArgs, sync: bool) -> anyhow::Result<()> {
    match args.cmd {
        None => show(args.json, sync).await,
        Some(FocusCmd::Add { item_id, pos }) => add(&item_id, pos, sync).await,
        Some(FocusCmd::Rm { item_id }) => rm(&item_id, sync).await,
        Some(FocusCmd::Mv { item_id, pos }) => mv(&item_id, pos, sync).await,
    }
}

async fn show(json: bool, sync: bool) -> anyhow::Result<()> {
    let session = Session::open(sync).await?;
    let items = session.doc().focus_view();
    if json {
        let rows: Vec<FocusItemJson> = items
            .iter()
            .enumerate()
            .map(|(i, item)| item_json(i + 1, item))
            .collect();
        print_json(&rows)?;
    } else {
        print_items(&items);
    }
    session.flush().await?;
    Ok(())
}

async fn add(item_id: &str, pos: Option<usize>, sync: bool) -> anyhow::Result<()> {
    let session = Session::open(sync).await?;
    // Absent position appends; a 1-based position converts to the 0-based
    // visible index the doc layer expects.
    let index = pos.map(one_based_to_index).unwrap_or(usize::MAX);
    session.doc().add_to_focus(item_id, index)?;
    session.flush().await?;
    println!("{item_id}");
    Ok(())
}

async fn rm(item_id: &str, sync: bool) -> anyhow::Result<()> {
    let session = Session::open(sync).await?;
    session.doc().remove_from_focus(item_id)?;
    session.flush().await?;
    println!("{item_id}");
    Ok(())
}

async fn mv(item_id: &str, pos: usize, sync: bool) -> anyhow::Result<()> {
    let session = Session::open(sync).await?;
    session
        .doc()
        .move_in_focus(item_id, one_based_to_index(pos))?;
    session.flush().await?;
    println!("{item_id}");
    Ok(())
}

/// Map a user-facing 1-based position to the 0-based visible index. A `0`
/// (or the absurd) clamps to the first slot; the doc layer clamps the top.
fn one_based_to_index(pos: usize) -> usize {
    pos.saturating_sub(1)
}

fn print_items(items: &[ItemView]) {
    for (i, item) in items.iter().enumerate() {
        // 1-based position matches the `mv` / `add` argument space. Focus
        // is Open-only, so the box carries the workflow state mark.
        let mark = super::items::state_mark(item.state);
        println!("{:>3}  {}  [{mark}] {}", i + 1, item.id, item.text);
    }
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
