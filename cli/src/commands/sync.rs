//! `monoplan sync` — ask the daemon to push now and say whether the
//! server is reachable. The daemon pulls continuously while connected,
//! so "sync" is a push plus a liveness report; an unreachable server is
//! a hard error because the user asked for the network explicitly.

use crate::client::{self, Spawn};
use crate::config::Profile;
use crate::ipc::{Reply, Request};

pub async fn run(spawn: Spawn) -> anyhow::Result<()> {
    let profile = Profile::require_active()?;
    match client::call(&profile, Request::Sync, spawn).await? {
        Reply::Synced { sync } if sync.online => {
            println!("Synced.");
            Ok(())
        }
        Reply::Synced { .. } => anyhow::bail!("could not reach server"),
        Reply::Err { message } => anyhow::bail!(message),
        other => anyhow::bail!("unexpected daemon reply: {other:?}"),
    }
}
