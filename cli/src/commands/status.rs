//! `monoplan status` — local-only summary. Never opens a WS.

use clap::Parser;
use serde::Serialize;

use crate::config::Profile;
use crate::keystore::dek_for_doc;

use super::items::print_json;

#[derive(Parser, Debug)]
pub struct StatusArgs {
    #[arg(long)]
    pub json: bool,
}

#[derive(Serialize)]
struct StatusJson<'a> {
    /// No account behind this workspace: the identity and server
    /// fields are null and nothing ever syncs (`crate::bootstrap`).
    local_only: bool,
    account_id: Option<&'a str>,
    email: Option<&'a str>,
    server_url: Option<&'a str>,
    device_id: Option<&'a str>,
    last_sync_at: Option<i64>,
    last_acked_seq: u64,
    pending_changes: bool,
}

pub async fn run(args: StatusArgs) -> anyhow::Result<()> {
    let profile = Profile::require_active()?;
    let secrets = profile.read_secrets()?;
    // `config.toml` exists once there is an account.
    let config = if secrets.has_account() {
        Some(profile.read_config()?)
    } else {
        None
    };

    // Identity + sync cursor live in the db; "pending" = local
    // operations beyond `server_known_vv` (or an outstanding in-flight
    // push). That answer is derived from the decrypted Loro history —
    // WAL rows alone can't tell (folded rows may carry unsent ops) —
    // so this rebuilds the doc.
    let storage = crate::storage::open_storage(&profile)?;
    let account = storage.read_account()?;
    let cursor = storage.read_sync_cursor(account.primary_doc_id)?;
    let dek = dek_for_doc(&secrets, account.primary_doc_id)?;
    let pending = crate::storage::has_unsynced_ops(&storage, &dek, account.primary_doc_id)
        .map_err(|e| anyhow::anyhow!("read pending state: {e}"))?;

    let identity = account.identity.as_ref();
    if args.json {
        print_json(&StatusJson {
            local_only: identity.is_none(),
            account_id: identity.map(|i| i.account_id.as_str()),
            email: identity.map(|i| i.email.as_str()),
            server_url: config.as_ref().map(|c| c.server_url.as_str()),
            device_id: identity.map(|i| i.device_id.as_str()),
            last_sync_at: cursor.last_sync_at,
            last_acked_seq: cursor.last_acked_server_seq.0,
            pending_changes: pending,
        })?;
    } else {
        let Some(identity) = identity else {
            println!("Account: none — local-only workspace");
            println!("         `monoplan auth signup` syncs it across devices");
            println!("Doc:     {}", account.primary_doc_id.0);
            return Ok(());
        };
        println!("Account: {} ({})", identity.email, identity.account_id);
        println!("Device:  {}", identity.device_id);
        if let Some(config) = &config {
            println!("Server:  {}", config.server_url);
        }
        println!(
            "Last sync: {}",
            cursor
                .last_sync_at
                .map(format_relative_millis)
                .unwrap_or_else(|| "never".into())
        );
        println!("Last acked seq: {}", cursor.last_acked_server_seq.0);
        println!("Pending changes: {}", if pending { "yes" } else { "no" });
    }
    Ok(())
}

fn format_relative_millis(unix_millis: i64) -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0);
    let diff_ms = (now - unix_millis).max(0);
    let secs = diff_ms / 1000;
    if secs < 60 {
        format!("{secs}s ago")
    } else if secs < 3600 {
        format!("{}m ago", secs / 60)
    } else if secs < 86_400 {
        format!("{}h ago", secs / 3600)
    } else {
        format!("{}d ago", secs / 86_400)
    }
}
