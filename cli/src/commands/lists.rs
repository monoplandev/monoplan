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

use monoplan_core::{Doc, ListView};
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
    let doc = cx.doc;
    match args.cmd {
        ListsCmd::Ls {
            json,
            archived,
            all,
        } => show(doc, cx, json, archived, all, out),
        ListsCmd::Add { name } => add(doc, &name, out),
        ListsCmd::Rename { list, name } => rename(doc, &list, &name, out),
        ListsCmd::Icon { list, icon } => set_icon(doc, &list, icon.as_deref(), out),
        ListsCmd::Archive { list } => set_archived(doc, &list, true, out),
        ListsCmd::Unarchive { list } => set_archived(doc, &list, false, out),
    }
}

fn show(
    doc: &Doc,
    cx: &Ctx,
    json: bool,
    archived_only: bool,
    all: bool,
    out: &mut dyn Write,
) -> anyhow::Result<()> {
    if json {
        let filter = match (all, archived_only) {
            (true, _) => ListFilter::All,
            (false, true) => ListFilter::Archived,
            (false, false) => ListFilter::Active,
        };
        return write_json(out, &api::lists::lists(cx, filter)?);
    }
    let lists: Vec<ListView> = doc
        .all_lists()
        .into_iter()
        .filter(|l| all || (l.archived_at.is_some() == archived_only))
        .collect();
    for l in &lists {
        if l.archived_at.is_some() {
            writeln!(out, "{}  {}  (archived)", l.id, l.name)?;
        } else {
            writeln!(out, "{}  {}", l.id, l.name)?;
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

fn set_icon(
    doc: &Doc,
    list_id: &str,
    icon: Option<&str>,
    out: &mut dyn Write,
) -> anyhow::Result<()> {
    doc.set_list_icon(list_id, icon.unwrap_or(""))?;
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

#[cfg(test)]
mod tests {
    use super::super::focus::Lens;
    use monoplan_core::Doc;

    /// `Request.addList` in MonoplanMacKit: the reply is the new list's
    /// id on a line.
    #[test]
    fn add_list_frame_from_the_apps_decodes_and_applies() {
        use crate::commands::{DocCmd, run_doc};
        let doc = Doc::new().unwrap();
        let cmd: DocCmd = serde_json::from_value(
            serde_json::json!({"Lists": {"cmd": {"Add": {"name": "Errands"}}}}),
        )
        .unwrap();
        let mut out = Vec::new();
        run_doc(cmd, &doc, &[&doc], &Lens::home(&doc), &mut out).unwrap();
        let id = String::from_utf8(out).unwrap();
        let lists = doc.all_lists();
        let list = lists.iter().find(|l| l.id == id.trim()).unwrap();
        assert_eq!(list.name, "Errands");
    }

    /// `Request.renameList`, `.setListIcon` and `.archiveList` in
    /// MonoplanMacKit.
    #[test]
    fn list_edit_frames_from_the_apps_decode_and_apply() {
        use crate::commands::{DocCmd, run_doc};
        let doc = Doc::new().unwrap();
        let id = doc.add_list("Errands").unwrap();
        let run = |cmd: serde_json::Value| {
            let cmd: DocCmd = serde_json::from_value(serde_json::json!({"Lists": {"cmd": cmd}})).unwrap();
            run_doc(cmd, &doc, &[&doc], &Lens::home(&doc), &mut Vec::new()).unwrap();
        };
        let list = || doc.all_lists().into_iter().find(|l| l.id == id).unwrap();
        run(serde_json::json!({"Rename": {"list": id, "name": "Chores"}}));
        assert_eq!(list().name, "Chores");
        run(serde_json::json!({"Icon": {"list": id, "icon": "🧹"}}));
        assert_eq!(list().icon.as_deref(), Some("🧹"));
        run(serde_json::json!({"Icon": {"list": id, "icon": null}}));
        assert_eq!(list().icon, None);
        run(serde_json::json!({"Archive": {"list": id}}));
        assert!(list().archived_at.is_some());
    }
}
