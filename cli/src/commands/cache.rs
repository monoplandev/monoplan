//! `monoplan cache` — local cache inspection and reset. Local-only; never opens a WS.

use std::io::IsTerminal;

use clap::{Parser, Subcommand};
use dialoguer::Confirm;
use serde::Serialize;

use crate::config::Profile;

use super::items::print_json;

#[derive(Parser, Debug)]
#[command(subcommand_required = true, arg_required_else_help = true)]
pub struct CacheArgs {
    #[command(subcommand)]
    pub cmd: CacheCmd,
}

#[derive(Subcommand, Debug)]
pub enum CacheCmd {
    /// Show cache location and size.
    Status {
        #[arg(long)]
        json: bool,
    },
    /// Delete the local doc cache and reset the sync cursor. The next
    /// `monoplan sync` will rehydrate from the server.
    Clear {
        /// Skip the confirmation prompt when there are unsynced ops.
        #[arg(long)]
        force: bool,
    },
}

pub async fn run(args: CacheArgs) -> anyhow::Result<()> {
    match args.cmd {
        CacheCmd::Status { json } => status(json).await,
        CacheCmd::Clear { force } => clear(force).await,
    }
}

#[derive(Serialize)]
struct CacheStatusJson<'a> {
    profile_dir: &'a str,
    doc_path: &'a str,
    doc_bytes: Option<u64>,
}

async fn status(json: bool) -> anyhow::Result<()> {
    let profile = Profile::require_active()?;
    let doc_path = profile.doc_path();
    let doc_bytes = std::fs::metadata(&doc_path).ok().map(|m| m.len());

    if json {
        print_json(&CacheStatusJson {
            profile_dir: profile.dir.to_string_lossy().as_ref(),
            doc_path: doc_path.to_string_lossy().as_ref(),
            doc_bytes,
        })?;
    } else {
        println!("Profile dir: {}", profile.dir.display());
        println!("Doc file:    {}", doc_path.display());
        println!(
            "Doc size:    {}",
            doc_bytes.map(format_bytes).unwrap_or_else(|| "—".into())
        );
    }
    Ok(())
}

async fn clear(force: bool) -> anyhow::Result<()> {
    let profile = Profile::require_active()?;

    // The doc cache (ops + snapshot) and sync cursor live in the sqlite
    // db; account identity lives there too, so we truncate the cache
    // tables rather than unlink the file — clearing the cache must not
    // log the user out.
    let storage = crate::storage::open_storage(&profile)?;
    let account = storage.read_account()?;
    let doc_id = account.primary_doc_id;

    // Local operations the server has no proof of would be lost by a
    // cache wipe.
    let secrets = profile.read_secrets()?;
    let dek = crate::keystore::dek_for_doc(&secrets, doc_id)?;
    let pending = crate::storage::has_unsynced_ops(&storage, &dek, doc_id)
        .map_err(|e| anyhow::anyhow!("read pending state: {e}"))?;

    if pending && !force {
        if !std::io::stdin().is_terminal() {
            anyhow::bail!(
                "local cache has unsynced changes; run `monoplan sync` first or pass --force"
            );
        }
        let proceed = Confirm::new()
            .with_prompt("Local cache has unsynced changes that will be lost. Clear anyway?")
            .default(false)
            .interact()?;
        if !proceed {
            println!("aborted");
            return Ok(());
        }
    }

    storage.clear_cache(doc_id)?;

    println!("Cache cleared. Run `monoplan sync` to rehydrate.");
    Ok(())
}

fn format_bytes(n: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = KB * 1024;
    const GB: u64 = MB * 1024;
    if n >= GB {
        format!("{:.2} GiB", n as f64 / GB as f64)
    } else if n >= MB {
        format!("{:.2} MiB", n as f64 / MB as f64)
    } else if n >= KB {
        format!("{:.1} KiB", n as f64 / KB as f64)
    } else {
        format!("{n} B")
    }
}
