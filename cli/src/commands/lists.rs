//! List management (`monoplan list …`): ls, add, rename, icon,
//! archive/unarchive.
//! `monoplan lists …` is a hidden alias; the IPC frame is still
//! `DocCmd::Lists`.
//!
//! There is deliberately no user-facing delete (`spec/cli.md`): archive
//! is the only way to remove a list from the active workspace. The
//! core's destructive `delete_list` stays internal.

use clap::{Parser, Subcommand};
use std::io::Write;

use serde::{Deserialize, Serialize};

use super::items::write_json;
use crate::api::{self, Ctx, lists::ListFilter};

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
    /// Set a user-created list's emoji, or clear it when none is given.
    Icon { list: String, icon: Option<String> },
    /// Archive a user-created list: removes it from the active
    /// workspace without touching its items, ordering, or metadata
    /// (`spec/data-model.md` "Archived lists").
    Archive { list: String },
    /// Restore an archived list to the active workspace.
    Unarchive { list: String },
}

pub fn run(cx: &Ctx, args: ListsArgs, out: &mut dyn Write) -> anyhow::Result<()> {
    // Every edit prints the list's id, as `add` prints the new one's.
    let id = match args.cmd {
        ListsCmd::Ls {
            json,
            archived,
            all,
        } => return show(cx, json, archived, all, out),
        ListsCmd::Add { name } => api::lists::add_list(cx, &name)?,
        ListsCmd::Rename { list, name } => {
            api::lists::rename_list(cx, &list, &name)?;
            list
        }
        ListsCmd::Icon { list, icon } => {
            api::lists::set_list_icon(cx, &list, icon.as_deref())?;
            list
        }
        ListsCmd::Archive { list } => {
            api::lists::set_list_archived(cx, &list, true)?;
            list
        }
        ListsCmd::Unarchive { list } => {
            api::lists::set_list_archived(cx, &list, false)?;
            list
        }
    };
    writeln!(out, "{id}")?;
    Ok(())
}

fn show(
    cx: &Ctx,
    json: bool,
    archived_only: bool,
    all: bool,
    out: &mut dyn Write,
) -> anyhow::Result<()> {
    let filter = match (all, archived_only) {
        (true, _) => ListFilter::All,
        (false, true) => ListFilter::Archived,
        (false, false) => ListFilter::Active,
    };
    let lists = api::lists::lists(cx, filter)?;
    if json {
        return write_json(out, &lists);
    }
    for l in &lists {
        if l.archived_at.is_some() {
            writeln!(out, "{}  {}  (archived)", l.id, l.name)?;
        } else {
            writeln!(out, "{}  {}", l.id, l.name)?;
        }
    }
    Ok(())
}
