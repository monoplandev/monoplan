use dialoguer::Password;
use monoplan_core::random_bytes;
use monoplan_protocol::{
    DocKey, KdfParams, PasswordChangeRequest, PreloginRequest, PreloginResponse,
};

use super::prompt_new_password;
use crate::config::Profile;
use crate::keystore::{dek_for_doc, derive_master};
use crate::net::Client;

pub async fn run() -> anyhow::Result<()> {
    let profile = Profile::require_active()?;
    // The host reads secrets only at boot; keep it out until the
    // change is through, then let it (or the app) come back.
    let _claim = crate::client::claim(&profile.dir).await?;
    let config = profile.read_config()?;
    let secrets = profile.read_secrets()?;
    let storage = crate::storage::open_storage(&profile)?;
    let account = storage.read_account()?;

    let current_password: String = Password::new().with_prompt("Current password").interact()?;
    let new_password = prompt_new_password("New password")?;

    let client = Client::new(config.server_url.clone());
    let pre: PreloginResponse = client
        .post(
            "/api/account/prelogin",
            &PreloginRequest {
                email: account.email.clone(),
            },
        )
        .await?;

    println!("Verifying current password…");
    let current_master = {
        let pw = current_password;
        let salt = pre.master_salt.clone();
        let params = pre.kdf_params;
        tokio::task::spawn_blocking(move || derive_master(&pw, &salt, params)).await??
    };
    let current_auth = current_master.auth_secret()?;

    println!("Re-keying…");
    let new_master_salt: [u8; 16] = random_bytes();
    let new_master = {
        let pw = new_password;
        let salt = new_master_salt.to_vec();
        tokio::task::spawn_blocking(move || derive_master(&pw, &salt, KdfParams::DEFAULT)).await??
    };
    let new_kek = new_master.kek()?;
    let new_auth = new_master.auth_secret()?;

    // Re-wrap the Home DEK only. Every other doc's server-side wrap is
    // under the Home DEK, not the password KEK, so a doc created on
    // another device since this one last logged in is unaffected. The
    // server refuses any set other than exactly the primary doc.
    let home = dek_for_doc(&secrets, account.primary_doc_id)?;
    let new_doc_keys = {
        let w = new_kek.wrap(&home)?;
        vec![DocKey {
            doc_id: account.primary_doc_id.0.to_string(),
            wrapped_dek: w.ciphertext,
            wrapped_dek_nonce: w.nonce.to_vec(),
        }]
    };

    client
        .post_authed_no_response(
            "/api/account/password/change",
            &secrets.device_token,
            &PasswordChangeRequest {
                current_auth_secret: current_auth.as_bytes().to_vec(),
                new_master_salt: new_master_salt.to_vec(),
                new_kdf_params: KdfParams::DEFAULT,
                new_auth_secret: new_auth.as_bytes().to_vec(),
                new_doc_keys,
            },
        )
        .await?;

    // No DEK changed — only the Home DEK's server-side wrap. The local
    // secrets file already holds the correct DEKs + device token.

    println!("Password changed.");
    Ok(())
}
