//! Starred commands: `starred` (list, `mv` to reorder).
//!
//! The curated single-tier Focus lens (`spec/focus.md`), surfaced to the
//! user as "Starred" (the spec's naming note). References items across
//! lists; the referenced item stays in its home list. `monoplan starred`
//! lists the view; `item set --star [POS]` / `--unstar` (in `items`)
//! and `starred mv` mutate the reference container. `monoplan focus
//! [add|rm|mv]` is the hidden pre-rename spelling: `focus` aliases
//! `starred`, and `add` / `rm` stay as hidden sub-subcommands. The IPC
//! frame is still `DocCmd::Focus(FocusArgs)`.
//!
//! Positions on the CLI are **1-based** to match the numbered listing a
//! user sees from `monoplan starred`; they are converted to the doc
//! layer's 0-based visible index here.
//!
//! `starred --all` is the host's half of `spec/focus.md` "Foreign refs":
//! the Home doc's full entry list with each foreign ref resolved against
//! the doc it names, non-Open and unresolvable entries left out, every
//! row tagged with its workspace and list. The daemon runs it over its
//! open docs; it is what the iOS Starred tab reads. Pruning dead foreign
//! refs stays with the clients that observe the close (the web does);
//! this read never writes.

use std::collections::HashMap;
use std::io::Write;

use clap::{Parser, Subcommand};
use monoplan_core::{Doc, DocId, ItemView, LIST_EVENTS, LIST_INBOX, Place};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::items::write_json;
use crate::workspace::{Candidate, HOME_LABEL};

#[derive(Parser, Debug, Clone, Serialize, Deserialize)]
pub struct FocusArgs {
    #[command(subcommand)]
    pub cmd: Option<FocusCmd>,
    /// Machine-parseable output for the list view.
    #[arg(long, global = true)]
    pub json: bool,
    /// The full lens: starred items in every workspace this install can
    /// open (Starred is personal, so this is Home's list with its
    /// cross-workspace entries resolved). Lists only; takes no `-w`.
    #[arg(long)]
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub all: bool,
}

#[derive(Subcommand, Debug, Clone, Serialize, Deserialize)]
pub enum FocusCmd {
    /// Hidden pre-rename spelling of `item set --star`. No-op if the item is
    /// already starred or is not Open (Done / binned items cannot be
    /// starred).
    #[command(hide = true)]
    Add {
        item_id: String,
        /// 1-based position to insert at (default: append).
        pos: Option<usize>,
    },
    /// Hidden pre-rename spelling of `item set --unstar`.
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
            .map(|(i, item)| item_json(i + 1, item, None, None))
            .collect();
        write_json(out, &rows)?;
    } else {
        print_items(out, &items)?;
    }
    Ok(())
}

/// One resolved row of the full lens: its workspace, the item, and
/// its list's name in that workspace (`None` when the list is gone).
pub struct StarredRow<'a> {
    pub doc: &'a Candidate,
    pub item: ItemView,
    pub list_name: Option<String>,
}

/// The full lens over `docs`, every workspace the caller has open,
/// `primary` being Home. Home's entries in curated order; a foreign
/// entry resolves in the doc it names and is skipped when that doc is
/// not open (not synced yet, or left) or the item is missing; any
/// entry is skipped when its item is not Open. The core already dedups
/// by item id.
pub fn resolve_all<'a>(
    docs: &'a [(Candidate, &'a Doc)],
    primary: DocId,
) -> anyhow::Result<Vec<StarredRow<'a>>> {
    let by_id: HashMap<DocId, &'a (Candidate, &'a Doc)> =
        docs.iter().map(|e| (e.0.doc_id, e)).collect();
    let (home_tag, home) = by_id
        .get(&primary)
        .map(|(c, d)| (c, *d))
        .ok_or_else(|| anyhow::anyhow!("the Home workspace is not open"))?;
    let mut list_names: HashMap<DocId, HashMap<String, String>> = HashMap::new();
    let mut rows = Vec::new();
    for entry in home.focus_entries() {
        let (tag, doc) = match &entry.doc_id {
            None => (home_tag, home),
            Some(id) => match Uuid::parse_str(id).ok().and_then(|u| by_id.get(&DocId(u))) {
                Some((c, d)) => (c, *d),
                None => continue,
            },
        };
        let Some(item) = doc.get_item(&entry.item_id) else {
            continue;
        };
        if !item.is_open() {
            continue;
        }
        // The reserved lists have no `ListMeta` row (`all_lists` skips
        // them), so they are named here as the clients label them.
        let names = list_names.entry(tag.doc_id).or_insert_with(|| {
            doc.all_lists()
                .into_iter()
                .map(|l| (l.id, l.name))
                .collect()
        });
        let list_name = match item.list_id.as_str() {
            LIST_INBOX => Some("Inbox".to_string()),
            LIST_EVENTS => Some("Events".to_string()),
            other => names.get(other).cloned(),
        };
        rows.push(StarredRow {
            doc: tag,
            item,
            list_name,
        });
    }
    Ok(rows)
}

/// `starred --all`: [`resolve_all`], printed like [`show`] with a
/// workspace tag on every row.
pub fn show_all(
    docs: &[(Candidate, &Doc)],
    primary: DocId,
    json: bool,
    out: &mut dyn Write,
) -> anyhow::Result<()> {
    let rows = resolve_all(docs, primary)?;
    if json {
        let rows: Vec<FocusItemJson> = rows
            .iter()
            .enumerate()
            .map(|(i, r)| item_json(i + 1, &r.item, Some(r.doc), r.list_name.as_deref()))
            .collect();
        write_json(out, &rows)?;
    } else {
        for (i, r) in rows.iter().enumerate() {
            let mark = super::items::state_mark(r.item.state);
            writeln!(
                out,
                "{:>3}  {}  [{mark}] {}  [{}]",
                i + 1,
                r.item.id,
                r.item.text,
                r.doc.name.as_deref().unwrap_or(HOME_LABEL)
            )?;
        }
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
pub fn one_based_to_index(pos: usize) -> usize {
    pos.saturating_sub(1)
}

fn print_items(out: &mut dyn Write, items: &[ItemView]) -> anyhow::Result<()> {
    for (i, item) in items.iter().enumerate() {
        // 1-based position matches the `--star` / `starred mv` argument
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
    /// `--all` only: the row's workspace (`doc_name` absent for Home and
    /// an unnamed doc) and its list's name there.
    #[serde(skip_serializing_if = "Option::is_none")]
    doc_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    doc_name: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    list_name: Option<&'a str>,
}

fn item_json<'a>(
    pos: usize,
    item: &'a ItemView,
    doc: Option<&'a Candidate>,
    list_name: Option<&'a str>,
) -> FocusItemJson<'a> {
    FocusItemJson {
        pos,
        id: &item.id,
        text: &item.text,
        list_id: &item.list_id,
        state: item.state.name(),
        place: item.place.as_ref(),
        doc_id: doc.map(|c| c.doc_id.0.to_string()),
        doc_name: doc.and_then(|c| c.name.as_deref()),
        list_name,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use monoplan_core::{ItemLifecycle, LIST_INBOX};

    #[test]
    fn full_lens_resolves_foreign_refs_in_home_order_and_skips_the_rest() {
        let home = Doc::new().unwrap();
        let work = Doc::new().unwrap();
        work.set_doc_name(Some("Work")).unwrap();
        let home_id = DocId(Uuid::from_u128(1));
        let work_id = DocId(Uuid::from_u128(2));
        let gone_id = DocId(Uuid::from_u128(3));

        let h = home.add_item(LIST_INBOX, "home item").unwrap();
        let w = work.add_item(LIST_INBOX, "work item").unwrap();
        let w_done = work.add_item(LIST_INBOX, "work done").unwrap();
        work.set_item_lifecycle(&w_done, ItemLifecycle::Done)
            .unwrap();
        // Curated order: Work's first, then Home's own, then a ref into
        // a doc this install does not hold and one to a closed item.
        home.add_foreign_to_focus(&work_id.0.to_string(), &w, usize::MAX)
            .unwrap();
        home.add_to_focus(&h, usize::MAX).unwrap();
        home.add_foreign_to_focus(
            &gone_id.0.to_string(),
            "01000000000000000000000000000000",
            usize::MAX,
        )
        .unwrap();
        home.add_foreign_to_focus(&work_id.0.to_string(), &w_done, usize::MAX)
            .unwrap();

        let docs = [
            (
                Candidate {
                    doc_id: home_id,
                    name: None,
                },
                &home,
            ),
            (
                Candidate {
                    doc_id: work_id,
                    name: Some("Work".into()),
                },
                &work,
            ),
        ];
        let rows = resolve_all(&docs, home_id).unwrap();
        let got: Vec<(&str, Option<&str>, Option<&str>)> = rows
            .iter()
            .map(|r| {
                (
                    r.item.text.as_str(),
                    r.doc.name.as_deref(),
                    r.list_name.as_deref(),
                )
            })
            .collect();
        assert_eq!(
            got,
            vec![
                ("work item", Some("Work"), Some("Inbox")),
                ("home item", None, Some("Inbox")),
            ]
        );
        // The single-doc view stays local: Home's own item only.
        assert_eq!(home.focus_view().len(), 1);
    }
}
