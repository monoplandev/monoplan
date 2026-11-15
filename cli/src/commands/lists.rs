//! List management (`monoplan list …`): ls, add, rename, icon,
//! archive/unarchive, delete.
//! `monoplan lists …` is a hidden alias; the IPC frame is still
//! `DocCmd::Lists`.
//!
//! Archive shelves a list intact; delete drops the row and sends its
//! items to Inbox (`spec/inbox-plan.md`). The confirm prompt for delete
//! runs in the thin client (`commands::mod`), before the frame is sent.

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
    /// Delete a user-created list: its open items go to the Bin, its
    /// done and cancelled ones to Inbox, closed. One undo restores it.
    Delete {
        list: String,
        /// Skip the confirmation prompt.
        #[arg(long)]
        yes: bool,
    },
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
        ListsCmd::Delete { list, .. } => {
            api::lists::delete_list(cx, &list, api::lists::DeleteMode::ToBin)?;
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

/// Ask before `list delete`, naming the list and how many open items it
/// sends to the Bin. Runs in the thin client: one `list ls`
/// round trip for the count. `true` means go ahead; a list the read
/// doesn't find goes ahead too, so the delete itself reports the error.
pub(crate) async fn confirm_delete(
    workspace: Option<String>,
    list: &str,
    spawn: crate::client::Spawn,
) -> anyhow::Result<bool> {
    let profile = crate::bootstrap::open_or_init().await?;
    let ls = super::DocCmd::Lists(ListsArgs {
        cmd: ListsCmd::Ls {
            json: true,
            archived: false,
            all: true,
        },
    });
    let (stdout, _) = crate::client::doc(&profile, workspace, ls, spawn).await?;
    let rows: Vec<serde_json::Value> = serde_json::from_str(stdout.trim()).unwrap_or_default();
    let Some(row) = rows.iter().find(|r| r["id"] == list) else {
        return Ok(true);
    };
    let name = row["name"].as_str().unwrap_or(list);
    let open = row["open"].as_u64().unwrap_or(0);
    let items = if open == 1 { "item" } else { "items" };
    Ok(dialoguer::Confirm::new()
        .with_prompt(format!(
            "Delete list {name:?}? Its {open} open {items} go to the Bin."
        ))
        .default(false)
        .interact()?)
}
