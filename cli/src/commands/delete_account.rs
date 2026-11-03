use dialoguer::{Input, Password};
use monoplan_protocol::{DeleteAccountRequest, PreloginRequest, PreloginResponse};
use reqwest::StatusCode;

use crate::config::Profile;
use crate::keystore::derive_master;
use crate::net::{Client, NetError};

/// `monoplan auth delete-account` (`spec/auth.md` §Account deletion):
/// spell out what goes, make the user type their email, re-verify the
/// password with the server, then wipe the profile as `logout` does.
pub async fn run() -> anyhow::Result<()> {
    let profile = Profile::require_active()?;
    let secrets = profile.read_secrets()?;
    let device_token = secrets.device_token()?.to_string();
    let config = profile.read_config()?;
    let storage = crate::storage::open_storage(&profile)?;
    let account = storage.read_account()?;
    let identity = account.identity()?.clone();
    drop(storage);

    println!(
        "This deletes the account {} and cannot be undone.",
        identity.email
    );
    println!();
    println!("  - Your Personal workspace and every workspace you own are erased from the");
    println!("    server for everyone, including anyone they are shared with.");
    println!("  - You leave every workspace shared with you.");
    println!("  - All your devices are signed out; the local copy here is wiped.");
    println!();
    println!("To keep a copy first: monoplan [-w <workspace>] workspace export-json --out FILE");
    println!();

    let typed: String = Input::new()
        .with_prompt(format!("Type {} to confirm", identity.email))
        .allow_empty(true)
        .interact_text()?;
    if typed.trim().to_lowercase() != identity.email.trim().to_lowercase() {
        println!("Email did not match; nothing was deleted.");
        return Ok(());
    }
    let password: String = Password::new().with_prompt("Password").interact()?;

    let client = Client::new(config.server_url.clone());
    let pre: PreloginResponse = client
        .post(
            "/api/account/prelogin",
            &PreloginRequest {
                email: identity.email.clone(),
            },
        )
        .await?;
    println!("Verifying password…");
    let master = {
        let salt = pre.master_salt.clone();
        let params = pre.kdf_params;
        tokio::task::spawn_blocking(move || derive_master(&password, &salt, params)).await??
    };
    let auth = master.auth_secret()?;

    match client
        .delete_authed_with_body(
            "/api/account",
            &device_token,
            &DeleteAccountRequest {
                current_auth_secret: auth.as_bytes().to_vec(),
            },
        )
        .await
    {
        Ok(()) => {}
        Err(NetError::Api { status, .. }) if status == StatusCode::UNAUTHORIZED => {
            anyhow::bail!("wrong password; nothing was deleted");
        }
        Err(e) => return Err(e.into()),
    }

    // The server has already forgotten this device, so this is the
    // local half of `logout`: stop the host, drop the agent, wipe.
    super::logout::logout(&profile).await?;
    println!("Account deleted. Local state wiped.");
    Ok(())
}
