use std::time::Duration;

use reqwest::StatusCode;

use crate::config::Profile;
use crate::net::{Client, NetError};

/// How long sign-out waits on the server before wiping anyway. The
/// profile claim is held meanwhile, so this bounds how long an
/// unreachable server keeps the app and the CLI out.
const REVOKE_TIMEOUT: Duration = Duration::from_secs(10);

/// What became of this device's token on the server.
#[derive(Debug)]
pub enum Revocation {
    /// The server deleted the device row; the token is dead.
    Revoked,
    /// The server no longer knew the token (revoked from another
    /// device, or the account is gone). Nothing left to revoke.
    AlreadyGone,
    /// A local-only workspace: there never was a token.
    NoAccount,
    /// The server could not be asked. The local copy of the token is
    /// wiped all the same, but the device row lives on until it is
    /// revoked from another device.
    Unreached(String),
}

pub async fn run() -> anyhow::Result<()> {
    match Profile::active()? {
        Some(p) if p.has_account()? => match logout(&p).await? {
            Revocation::Unreached(why) => {
                println!("Logged out (local state wiped).");
                eprintln!(
                    "warning: could not reach the server to revoke this device ({why}). \
                     Its token is gone from this machine, but the device stays listed on \
                     the account; remove it from another device's Settings › Devices."
                );
            }
            _ => println!("Logged out (device revoked, local state wiped)."),
        },
        // A local-only workspace has nothing to log out of, and
        // nothing a sign-out should destroy.
        Some(_) => println!("Not logged in (the local workspace is kept)."),
        None => println!("Not logged in."),
    }
    Ok(())
}

/// Stop the profile's host, revoke this device on the server
/// (`POST /api/account/logout`, `spec/auth.md`), wipe the profile.
/// Shared with the apps' Sign out (`monoplan-ffi::logout`).
///
/// The revoke is best-effort, as on the web: an unreachable server
/// must not leave readable account data on a machine the user asked to
/// sign out, so the wipe happens either way and the outcome says which.
pub async fn logout(p: &Profile) -> anyhow::Result<Revocation> {
    // Held until the purge is done, so no host (the app's included)
    // boots the doc out from under it. Taken before the revoke: the
    // host's socket is closed first, so it never sees its own token
    // die and report an auth failure.
    let _claim = crate::client::claim(&p.dir).await?;
    let revocation = revoke(p).await;
    wipe(p)?;
    Ok(revocation)
}

/// The local half of sign-out, for callers whose server side is
/// already done (`auth delete-account`): stop the host, wipe.
pub async fn logout_local(p: &Profile) -> anyhow::Result<()> {
    let _claim = crate::client::claim(&p.dir).await?;
    wipe(p)
}

fn wipe(p: &Profile) -> anyhow::Result<()> {
    // The login agent would fail at every login against a purged
    // profile.
    crate::service::uninstall_quietly();
    p.purge()?;
    Ok(())
}

async fn revoke(p: &Profile) -> Revocation {
    // An unreadable profile is still wiped; it just cannot be revoked.
    let secrets = match p.read_secrets() {
        Ok(s) => s,
        Err(e) => return Revocation::Unreached(e.to_string()),
    };
    let Some(token) = secrets.device_token else {
        return Revocation::NoAccount;
    };
    let server_url = match p.read_config() {
        Ok(c) => c.server_url,
        Err(e) => return Revocation::Unreached(e.to_string()),
    };
    let client = Client::new(server_url);
    let body = serde_json::json!({});
    let call = client.post_authed_no_response("/api/account/logout", &token, &body);
    match tokio::time::timeout(REVOKE_TIMEOUT, call).await {
        Ok(Ok(())) => Revocation::Revoked,
        Ok(Err(NetError::Api { status, .. })) if status == StatusCode::UNAUTHORIZED => {
            Revocation::AlreadyGone
        }
        Ok(Err(e)) => Revocation::Unreached(e.to_string()),
        Err(_) => Revocation::Unreached("timed out".into()),
    }
}
