//! List management (`monoplan list …`): ls, add, rename, archive/unarchive.
//! `monoplan lists …` is a hidden alias; the IPC frame is still
//! `DocCmd::Lists`.
//!
//! There is deliberately no user-facing delete (`spec/cli.md`): archive
//! is the only way to remove a list from the active workspace. The
//! core's destructive `delete_list` stays internal.

use clap::{Parser, Subcommand};
use std::collections::HashMap;
use std::io::Write;

use monoplan_core::{Doc, ListView};
use serde::{Deserialize, Serialize};

use super::items::write_json;

#[derive(Parser, Debug, Clone, Serialize, Deserialize)]
#[command(subcommand_required = true, arg_required_else_help = true)]
pub struct ListsArgs {
    #[command(subcommand)]
    pub cmd: ListsCmd,
}

#[derive(Subcommand, Debug, Clone, Serialize, Deserialize)]
pub enum ListsCmd {
    /// Show lists. Active lists by default; `--archived` shows only
    /// archived lists, `--all` shows both.
    Ls {
        #[arg(long)]
        json: bool,
        /// Show only archived lists.
        #[arg(long, conflicts_with = "all")]
        archived: bool,
        /// Show active and archived lists.
        #[arg(long)]
        all: bool,
    },
    /// Add a new list.
    Add { name: String },
    /// Rename a user-created list. The reserved `inbox` list is not
    /// renamable from the doc layer (its label is client-side until a
    /// meta CRDT lands).
    Rename { list: String, name: String },
    /// Archive a user-created list: removes it from the active
    /// workspace without touching its items, ordering, or metadata
    /// (`spec/data-model.md` "Archived lists").
    Archive { list: String },
    /// Restore an archived list to the active workspace.
    Unarchive { list: String },
}

pub fn run(doc: &Doc, args: ListsArgs, out: &mut dyn Write) -> anyhow::Result<()> {
    match args.cmd {
        ListsCmd::Ls {
            json,
            archived,
            all,
        } => show(doc, json, archived, all, out),
        ListsCmd::Add { name } => add(doc, &name, out),
        ListsCmd::Rename { list, name } => rename(doc, &list, &name, out),
        ListsCmd::Archive { list } => set_archived(doc, &list, true, out),
        ListsCmd::Unarchive { list } => set_archived(doc, &list, false, out),
    }
}

fn show(
    doc: &Doc,
    json: bool,
    archived_only: bool,
    all: bool,
    out: &mut dyn Write,
) -> anyhow::Result<()> {
    let lists: Vec<ListView> = doc
        .all_lists()
        .into_iter()
        .filter(|l| all || (l.archived_at.is_some() == archived_only))
        .collect();
    if json {
        // One walk of the doc's items for every list's open count.
        let mut open = HashMap::<String, usize>::new();
        for item in doc.all_items() {
            if item.is_open() {
                *open.entry(item.list_id).or_insert(0) += 1;
            }
        }
        let rows: Vec<_> = lists
            .iter()
            .map(|l| list_json(l, open.get(&l.id).copied().unwrap_or(0)))
            .collect();
        write_json(out, &rows)?;
    } else {
        for l in &lists {
            if l.archived_at.is_some() {
                writeln!(out, "{}  {}  (archived)", l.id, l.name)?;
            } else {
                writeln!(out, "{}  {}", l.id, l.name)?;
            }
        }
    }
    Ok(())
}

fn add(doc: &Doc, name: &str, out: &mut dyn Write) -> anyhow::Result<()> {
    let id = doc.add_list(name)?;
    writeln!(out, "{id}")?;
    Ok(())
}

fn rename(doc: &Doc, list_id: &str, name: &str, out: &mut dyn Write) -> anyhow::Result<()> {
    doc.rename_list(list_id, name)?;
    writeln!(out, "{list_id}")?;
    Ok(())
}

fn set_archived(
    doc: &Doc,
    list_id: &str,
    archived: bool,
    out: &mut dyn Write,
) -> anyhow::Result<()> {
    doc.set_list_archived(list_id, archived)?;
    writeln!(out, "{list_id}")?;
    Ok(())
}

#[derive(Serialize)]
struct ListJson<'a> {
    id: &'a str,
    name: &'a str,
    /// The list's emoji, when the user picked one.
    #[serde(skip_serializing_if = "Option::is_none")]
    icon: Option<&'a str>,
    archived_at: Option<i64>,
    created_at: i64,
    /// How many Open items the list holds (not closed, not binned).
    open: usize,
}

fn list_json(l: &ListView, open: usize) -> ListJson<'_> {
    ListJson {
        id: &l.id,
        name: &l.name,
        icon: l.icon.as_deref(),
        archived_at: l.archived_at,
        created_at: l.created_at,
        open,
    }
}
