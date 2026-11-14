//! Bin namespace: `bin show` / `bin empty` / `bin rm <id>`. Binning an
//! item is `item bin <id>` ([`super::items::bin`]). The behaviour is
//! `api::bin`; this is the arguments and the rendering.

use clap::{Parser, Subcommand};
use std::io::Write;

use serde::{Deserialize, Serialize};

use super::items::{IdArg, write_json};
use crate::api::{self, Ctx};

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
}

pub fn run(cx: &Ctx, args: BinArgs, out: &mut dyn Write) -> anyhow::Result<()> {
    match args.sub {
        BinSub::Show => {
            let rows = api::bin::binned(cx)?;
            if args.json {
                write_json(out, &rows)?;
            } else {
                for row in &rows {
                    writeln!(out, "{}  {}", row.id, row.text)?;
                }
            }
        }
        BinSub::Empty => writeln!(out, "removed {}", api::bin::empty_bin(cx)?)?,
        BinSub::Rm(id) => {
            api::bin::delete_binned(cx, &id.item_id)?;
            writeln!(out, "{}", id.item_id)?;
        }
    }
    Ok(())
}
