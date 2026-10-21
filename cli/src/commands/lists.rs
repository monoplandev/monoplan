//! List management: list (default action), add, rename, archive/unarchive.
//!
//! There is deliberately no user-facing delete (`spec/cli.md`): archive
//! is the only way to remove a list from the active workspace. The
//! core's destructive `delete_list` stays internal.

use clap::{Parser, Subcommand};
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
        write_json(out, &lists.iter().map(list_json).collect::<Vec<_>>())?;
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
    archived_at: Option<i64>,
    created_at: i64,
}

fn list_json(l: &ListView) -> ListJson<'_> {
    ListJson {
        id: &l.id,
        name: &l.name,
        archived_at: l.archived_at,
        created_at: l.created_at,
    }
}
