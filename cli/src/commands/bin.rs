//! Bin namespace + `bin <id>` action.
//!
//! `monoplan bin <id>` (the verb) lives in [`super::items::bin`]; this
//! module owns the namespace operations and the dispatcher that picks
//! between them. Clap's `external_subcommand` captures any token that
//! isn't a known sub-subcommand, which we then treat as an item id
//! and route to the verb.

use clap::{Parser, Subcommand};
use std::io::Write;

use monoplan_core::Doc;
use serde::{Deserialize, Serialize};

use super::items::{IdArg, write_json};

#[derive(Parser, Debug, Clone, Serialize, Deserialize)]
pub struct BinArgs {
    #[command(subcommand)]
    pub sub: BinSub,
    #[arg(long, global = true)]
    pub json: bool,
}

#[derive(Subcommand, Debug, Clone, Serialize, Deserialize)]
pub enum BinSub {
    /// Show items in the bin.
    Show,
    /// Hard-delete every binned item.
    Empty,
    /// Hard-delete a single binned item.
    Rm(IdArg),
    /// Anything else is treated as an item id to bin.
    #[command(external_subcommand)]
    Item(Vec<String>),
}

pub fn run(doc: &Doc, args: BinArgs, out: &mut dyn Write) -> anyhow::Result<()> {
    match args.sub {
        BinSub::Show => show(doc, args.json, out),
        BinSub::Empty => empty(doc, out),
        BinSub::Rm(id) => rm(doc, &id.item_id, out),
        BinSub::Item(tokens) => match tokens.as_slice() {
            [id] => super::items::bin(
                doc,
                IdArg {
                    item_id: id.clone(),
                },
                out,
            ),
            [] => anyhow::bail!("expected an item id; try `monoplan bin show`"),
            _ => anyhow::bail!("unexpected extra arguments after item id"),
        },
    }
}

fn show(doc: &Doc, json: bool, out: &mut dyn Write) -> anyhow::Result<()> {
    let items = doc.binned_items();
    if json {
        let v: Vec<BinnedJson> = items
            .iter()
            .map(|i| BinnedJson {
                id: &i.id,
                text: &i.text,
                list_id: &i.list_id,
                binned_at: i.binned_at.unwrap_or_default(),
            })
            .collect();
        write_json(out, &v)?;
    } else {
        for i in &items {
            writeln!(out, "{}  {}", i.id, i.text)?;
        }
    }
    Ok(())
}

fn empty(doc: &Doc, out: &mut dyn Write) -> anyhow::Result<()> {
    let removed = doc.empty_bin()?;
    writeln!(out, "removed {removed}")?;
    Ok(())
}

fn rm(doc: &Doc, item_id: &str, out: &mut dyn Write) -> anyhow::Result<()> {
    doc.delete_binned(item_id)?;
    writeln!(out, "{item_id}")?;
    Ok(())
}

#[derive(Serialize)]
struct BinnedJson<'a> {
    id: &'a str,
    text: &'a str,
    list_id: &'a str,
    binned_at: i64,
}
