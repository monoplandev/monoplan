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
use monoplan_core::{Doc, DocId, ItemView};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::items::write_json;
use crate::api::{self, Ctx, lists::ListNames, starred::StarredItem};
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
        /// 1-based position to insert at (default: the top).
        pos: Option<usize>,
    },
    /// Hidden pre-rename spelling of `item set --unstar`.
    #[command(hide = true)]
    Rm { item_id: String },
    /// Reorder a starred item: to a 1-based position, or directly after another starred item.
    Mv {
        item_id: String,
        #[arg(required_unless_present = "after")]
        pos: Option<usize>,
        /// Put it directly after this starred item instead of at a position.
        #[arg(long, value_name = "ITEM_ID", conflicts_with = "pos")]
        #[serde(default)]
        after: Option<String>,
    },
}

/// Starred as seen from the doc a command runs on. The lens lives in
/// the Home doc alone (`spec/focus.md` "Foreign refs"): an item of any
/// other workspace is starred by a foreign ref in Home, never by a
/// write to its own doc. This is the host's half of that rule for
/// `item set --star` / `--unstar` and `item show`.
pub struct Lens<'a> {
    home: &'a Doc,
    /// The id of the doc the command runs on, when it is not Home.
    foreign: Option<String>,
}

impl<'a> Lens<'a> {
    /// The command runs on Home itself.
    pub fn home(home: &'a Doc) -> Self {
        Lens {
            home,
            foreign: None,
        }
    }

    /// The command runs on `doc_id`, another workspace than `home`.
    pub fn foreign(home: &'a Doc, doc_id: DocId) -> Self {
        Lens {
            home,
            foreign: Some(doc_id.0.to_string()),
        }
    }

    /// Whether `item` (of the command's doc) shows in Starred: Open and
    /// referenced. A foreign ref outlives its item closing until a
    /// client prunes it, hence the Open check here.
    pub fn has(&self, item: &ItemView) -> bool {
        item.is_open()
            && self
                .home
                .focus_entries()
                .iter()
                .any(|e| e.item_id == item.id)
    }

    /// Star `item_id` of `doc` (the command's doc) at entry `index`
    /// (`usize::MAX` appends). No-op when already starred or not Open,
    /// as `Doc::add_to_focus`; an unknown item is an error.
    pub fn star(&self, doc: &Doc, item_id: &str, index: usize) -> anyhow::Result<()> {
        let Some(doc_id) = &self.foreign else {
            self.home.add_to_focus(item_id, index)?;
            return Ok(());
        };
        // Home cannot see the item, so the checks its own
        // `add_to_focus` makes are made here against the item's doc.
        let Some(item) = doc.get_item(item_id) else {
            anyhow::bail!("item not found: {item_id}");
        };
        if item.is_open() {
            self.home.add_foreign_to_focus(doc_id, item_id, index)?;
        }
        Ok(())
    }

    /// Where `item_id` sits among Starred's entries (0-based), if it
    /// is referenced, in either form.
    pub fn position(&self, item_id: &str) -> Option<usize> {
        self.home
            .focus_entries()
            .iter()
            .position(|e| e.item_id == item_id)
    }

    /// Unstar `item_id`, whichever doc it lives in (refs dedup by item
    /// id across both forms). The item is untouched.
    pub fn unstar(&self, item_id: &str) -> anyhow::Result<()> {
        self.home.remove_from_focus(item_id)?;
        Ok(())
    }
}

/// `starred`, and with `--all` the full lens (`api::starred`); `mv`
/// and the hidden `add` / `rm` write the command's own doc.
pub fn run(cx: &Ctx, args: FocusArgs, out: &mut dyn Write) -> anyhow::Result<()> {
    let doc = cx.doc;
    match args.cmd {
        None if args.all => show(&api::starred::starred_everywhere(cx)?, args.json, out),
        None => show(&api::starred::starred(cx)?, args.json, out),
        Some(_) if args.all => anyhow::bail!("starred --all only lists; mv takes -w instead"),
        Some(FocusCmd::Add { item_id, pos }) => add(doc, &item_id, pos, out),
        Some(FocusCmd::Rm { item_id }) => rm(doc, &item_id, out),
        Some(FocusCmd::Mv {
            item_id,
            pos,
            after,
        }) => {
            api::starred::move_starred(cx, &item_id, pos, after.as_deref())?;
            writeln!(out, "{item_id}")?;
            Ok(())
        }
    }
}

/// The numbered listing; rows of the full lens end in their
/// workspace.
fn show(rows: &[StarredItem], json: bool, out: &mut dyn Write) -> anyhow::Result<()> {
    if json {
        return write_json(out, &rows);
    }
    for r in rows {
        // The position matches the `--star` / `starred mv` argument
        // space. Starred is Open-only, so the box carries the workflow
        // state mark.
        let mark = super::items::state_mark(r.state.into());
        write!(out, "{:>3}  {}  [{mark}] {}", r.pos, r.id, r.text)?;
        if r.doc_id.is_some() {
            write!(out, "  [{}]", r.doc_name.as_deref().unwrap_or(HOME_LABEL))?;
        }
        writeln!(out)?;
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
    let mut list_names: HashMap<DocId, ListNames> = HashMap::new();
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
        let list_name = list_names
            .entry(tag.doc_id)
            .or_insert_with(|| ListNames::of(doc))
            .get(&item.list_id);
        rows.push(StarredRow {
            doc: tag,
            item,
            list_name,
        });
    }
    Ok(rows)
}

fn add(doc: &Doc, item_id: &str, pos: Option<usize>, out: &mut dyn Write) -> anyhow::Result<()> {
    // Absent position is the top; a 1-based position converts to the
    // 0-based visible index the doc layer expects.
    let index = pos.map(one_based_to_index).unwrap_or(0);
    doc.add_to_focus(item_id, index)?;
    writeln!(out, "{item_id}")?;
    Ok(())
}

fn rm(doc: &Doc, item_id: &str, out: &mut dyn Write) -> anyhow::Result<()> {
    doc.remove_from_focus(item_id)?;
    writeln!(out, "{item_id}")?;
    Ok(())
}

/// Map a user-facing 1-based position to the 0-based visible index. A `0`
/// (or the absurd) clamps to the first slot; the doc layer clamps the top.
pub fn one_based_to_index(pos: usize) -> usize {
    pos.saturating_sub(1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use monoplan_core::{ItemLifecycle, LIST_INBOX};

    /// `Request.moveStarred`: the frames the apps send for a drag on
    /// the Starred page. `after` lands the item directly under its
    /// neighbour even with an entry between them the page cannot show.
    #[test]
    fn mv_frames_from_the_apps_reorder_past_hidden_entries() {
        use super::super::DocCmd;
        let home = Doc::new().unwrap();
        let ids: Vec<String> = ["a", "b", "c"]
            .iter()
            .map(|t| {
                let id = home.add_item(LIST_INBOX, t).unwrap();
                home.add_to_focus(&id, usize::MAX).unwrap();
                id
            })
            .collect();
        // A ref into a doc that is not open, between a and b.
        let hidden = "01000000000000000000000000000000";
        home.add_foreign_to_focus(&Uuid::from_u128(9).to_string(), hidden, 1)
            .unwrap();
        let order = || -> Vec<String> {
            home.focus_entries()
                .into_iter()
                .map(|e| match home.get_item(&e.item_id) {
                    Some(item) => item.text,
                    None => "hidden".into(),
                })
                .collect()
        };
        assert_eq!(order(), ["a", "hidden", "b", "c"]);
        let run = |mv: serde_json::Value| {
            let cmd: DocCmd = serde_json::from_value(
                serde_json::json!({"Focus": {"cmd": {"Mv": mv}, "json": false}}),
            )
            .unwrap();
            let DocCmd::Focus(args) = cmd else {
                panic!("not a Focus frame")
            };
            run(
                &Ctx::bare(&home, &[&home], &Lens::home(&home)),
                args,
                &mut Vec::new(),
            )
        };
        // Down: a under b.
        run(serde_json::json!({"item_id": ids[0], "pos": null, "after": ids[1]})).unwrap();
        assert_eq!(order(), ["hidden", "b", "a", "c"]);
        // Up: c under b.
        run(serde_json::json!({"item_id": ids[2], "pos": null, "after": ids[1]})).unwrap();
        assert_eq!(order(), ["hidden", "b", "c", "a"]);
        // To the top: a position, above the hidden entry too.
        run(serde_json::json!({"item_id": ids[0], "pos": 1})).unwrap();
        assert_eq!(order(), ["a", "hidden", "b", "c"]);
        // Under the last row.
        run(serde_json::json!({"item_id": ids[0], "pos": null, "after": ids[2]})).unwrap();
        assert_eq!(order(), ["hidden", "b", "c", "a"]);
        assert!(run(serde_json::json!({"item_id": ids[0], "pos": null, "after": "nope"})).is_err());
    }

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

    #[test]
    fn lens_stars_another_workspaces_item_by_a_foreign_ref_in_home() {
        let home = Doc::new().unwrap();
        let work = Doc::new().unwrap();
        let work_id = DocId(Uuid::from_u128(2));
        let h = home.add_item(LIST_INBOX, "home item").unwrap();
        let w = work.add_item(LIST_INBOX, "work item").unwrap();
        let w_done = work.add_item(LIST_INBOX, "work done").unwrap();
        work.set_item_lifecycle(&w_done, ItemLifecycle::Done)
            .unwrap();

        let lens = Lens::foreign(&home, work_id);
        lens.star(&work, &w, usize::MAX).unwrap();
        // Closed items cannot be starred; unknown ones are an error.
        lens.star(&work, &w_done, usize::MAX).unwrap();
        assert_eq!(
            lens.star(&work, "nope", usize::MAX)
                .unwrap_err()
                .to_string(),
            "item not found: nope"
        );
        Lens::home(&home).star(&home, &h, 0).unwrap();

        // Both refs are Home's, in order; Work's doc was never written.
        let entries: Vec<(Option<String>, String)> = home
            .focus_entries()
            .into_iter()
            .map(|e| (e.doc_id, e.item_id))
            .collect();
        assert_eq!(
            entries,
            vec![(None, h.clone()), (Some(work_id.0.to_string()), w.clone())]
        );
        assert!(work.focus_entries().is_empty());
        assert!(lens.has(&work.get_item(&w).unwrap()));
        assert!(!lens.has(&work.get_item(&w_done).unwrap()));
        assert!(Lens::home(&home).has(&home.get_item(&h).unwrap()));

        // A foreign ref whose item has since closed no longer counts.
        work.set_item_lifecycle(&w, ItemLifecycle::Done).unwrap();
        assert!(!lens.has(&work.get_item(&w).unwrap()));
        work.set_item_lifecycle(&w, ItemLifecycle::Backlog).unwrap();

        lens.unstar(&w).unwrap();
        assert!(!lens.has(&work.get_item(&w).unwrap()));
        assert_eq!(home.focus_entries().len(), 1);
    }
}
