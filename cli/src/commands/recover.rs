use clap::Parser;
use dialoguer::Input;
use monoplan_core::{Doc, derive_recovery_master, parse_recovery_code, random_bytes};
use monoplan_protocol::{
    DocKey, DocsListResponse, KdfParams, PasswordResetRequest, PasswordResetResponse,
    PreloginRequest, PreloginResponse, RecoverRequest, RecoverResponse,
};

use crate::client::{self, Spawn};
use crate::config::{Config, Profile, Secrets};
use crate::keystore::{derive_master, parse_doc_id, unwrap_doc_key, unwrap_memberships};
use crate::net::Client;
use crate::storage::Account;

use super::{default_device_name, default_server, device_kind, prompt_new_password};

#[derive(Parser, Debug)]
pub struct Args {
    #[arg(long, default_value_t = default_server())]
    pub server: String,
    #[arg(long)]
    pub email: Option<String>,
    #[arg(long)]
    pub device_name: Option<String>,
}

pub async fn run(args: Args, spawn: Spawn) -> anyhow::Result<()> {
    let email = match args.email {
        Some(e) => e,
        None => Input::new().with_prompt("Email").interact_text()?,
    };
    let phrase: String = Input::new()
        .with_prompt("Recovery code (12 words)")
        .interact_text()?;
    let recovery_code = parse_recovery_code(&phrase)?;
    let new_password = prompt_new_password("New password")?;
    let device_name = args.device_name.unwrap_or_else(|| {
        Input::new()
            .with_prompt("Device name")
            .default(default_device_name())
            .interact_text()
            .unwrap_or_else(|_| default_device_name())
    });

    let client = Client::new(args.server.clone());
    let pre: PreloginResponse = client
        .post(
            "/api/account/prelogin",
            &PreloginRequest {
                email: email.clone(),
            },
        )
        .await?;
    let recovery_salt = pre
        .recovery_salt
        .ok_or_else(|| anyhow::anyhow!("this account does not have a recovery code enrolled"))?;
    let kdf_params = pre.kdf_params;

    println!("Verifying recovery code…");
    let r_master = {
        let phrase = recovery_code.as_str().to_string();
        let salt = recovery_salt.clone();
        tokio::task::spawn_blocking(move || derive_recovery_master(&phrase, &salt, kdf_params))
            .await??
    };
    let r_kek = r_master.kek()?;
    let r_auth = r_master.auth_secret()?;

    let recovered: RecoverResponse = client
        .post(
            "/api/account/recover",
            &RecoverRequest {
                email: email.clone(),
                recovery_auth_secret: r_auth.as_bytes().to_vec(),
            },
        )
        .await?;

    // The code opens one wrap: the primary membership's copy of the
    // Home DEK. Every other doc's key follows from it after the reset.
    let doc_id = parse_doc_id(&recovered.primary_doc_id)?;
    let dek = recovered
        .doc_keys
        .iter()
        .find(|k| k.doc_id == recovered.primary_doc_id)
        .map(|k| unwrap_doc_key(&r_kek, &k.wrapped_dek, &k.wrapped_dek_nonce))
        .ok_or_else(|| anyhow::anyhow!("recovery response has no key for the primary doc"))??;

    println!("Re-keying with new password…");
    let new_master_salt: [u8; 16] = random_bytes();
    let new_master = {
        let pw = new_password;
        let salt = new_master_salt.to_vec();
        tokio::task::spawn_blocking(move || derive_master(&pw, &salt, KdfParams::DEFAULT)).await??
    };
    let new_kek = new_master.kek()?;
    let new_auth = new_master.auth_secret()?;
    // The Home DEK re-wrapped under the new password KEK. The server
    // refuses anything but exactly the primary doc here.
    let new_doc_keys = {
        let w = new_kek.wrap(&dek)?;
        vec![DocKey {
            doc_id: recovered.primary_doc_id.clone(),
            wrapped_dek: w.ciphertext,
            wrapped_dek_nonce: w.nonce.to_vec(),
        }]
    };

    let reset: PasswordResetResponse = client
        .post(
            "/api/account/password/reset",
            &PasswordResetRequest {
                recovery_session_token: recovered.recovery_session_token,
                new_master_salt: new_master_salt.to_vec(),
                new_kdf_params: KdfParams::DEFAULT,
                new_auth_secret: new_auth.as_bytes().to_vec(),
                new_doc_keys,
                device_name,
                device_kind: device_kind(),
            },
        )
        .await?;

    if parse_doc_id(&reset.primary_doc_id)? != doc_id {
        anyhow::bail!("server changed primary_doc_id between recover and reset");
    }
    // Now a device: learn the rest of the memberships and open each
    // with the Home DEK, exactly as login does.
    let docs: DocsListResponse = client.get_authed("/api/docs", &reset.device_token).await?;
    let doc_keys = unwrap_memberships(&dek, doc_id, &docs.memberships)?;

    // A daemon for a previous login would keep the old doc open under
    // the new credentials; stop it before the profile changes.
    if let Some(existing) = Profile::active()? {
        crate::client::stop_if_running(&existing.dir).await?;
    }
    let profile = Profile::create()?;
    let storage = crate::storage::open_storage(&profile)?;
    crate::storage::seed_snapshot(&storage, &dek, doc_id, &Doc::empty())?;
    storage.write_account(&Account {
        account_id: recovered.account_id.clone(),
        email,
        device_id: reset.device_id,
        primary_doc_id: doc_id,
    })?;
    profile.write_config(&Config {
        server_url: args.server,
    })?;
    profile.write_secrets(&Secrets::new(
        reset.device_token,
        doc_keys.iter().map(|(id, dek)| (*id, dek)),
    ))?;

    println!("Syncing…");
    if let Err(e) = client::sync(&profile, spawn).await {
        eprintln!("offline — sync deferred ({e})");
    }

    println!("Recovery complete. Account {}.", recovered.account_id);
    Ok(())
}
