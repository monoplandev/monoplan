use clap::Parser;
use dialoguer::{Input, Password};
use monoplan_core::{Dek, Doc, DocId, Kek, WrappedDek};
use monoplan_protocol::{
    DeviceRegistration, LoginRequest, LoginResponse, PreloginRequest, PreloginResponse,
};

use crate::config::{Config, Profile, Secrets};
use crate::keystore::derive_master;
use crate::net::Client;
use crate::storage::Account;
use crate::sync::Session;

use super::{default_device_name, default_server};

#[derive(Parser, Debug)]
pub struct Args {
    #[arg(long, default_value_t = default_server())]
    pub server: String,
    #[arg(long)]
    pub email: Option<String>,
    #[arg(long)]
    pub device_name: Option<String>,
}

pub async fn run(args: Args) -> anyhow::Result<()> {
    let email = match args.email {
        Some(e) => e,
        None => Input::new().with_prompt("Email").interact_text()?,
    };
    let password: String = Password::new().with_prompt("Password").interact()?;
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

    println!("Deriving keys…");
    let master_salt = pre.master_salt.clone();
    let kdf_params = pre.kdf_params;
    let master = tokio::task::spawn_blocking({
        let pw = password;
        move || derive_master(&pw, &master_salt, kdf_params)
    })
    .await??;
    let kek = master.kek()?;
    let auth_secret = master.auth_secret()?;

    let resp: LoginResponse = client
        .post(
            "/api/account/login",
            &LoginRequest {
                email: email.clone(),
                auth_secret: auth_secret.as_bytes().to_vec(),
                register_device: Some(DeviceRegistration { name: device_name }),
            },
        )
        .await?;

    // One wrap per membership, each under the KEK just derived. The
    // primary doc must be among them — it is the doc this CLI runs.
    let doc_keys: Vec<(DocId, Dek)> = resp
        .memberships
        .iter()
        .map(|m| {
            Ok((
                parse_doc_id(&m.doc_id)?,
                unwrap_doc_key(&kek, &m.wrapped_dek, &m.wrapped_dek_nonce)?,
            ))
        })
        .collect::<anyhow::Result<_>>()?;

    let device = resp.device.ok_or_else(|| {
        anyhow::anyhow!("server did not return a device credential despite register_device")
    })?;

    let profile = Profile::create()?;
    let doc_id = parse_doc_id(&resp.primary_doc_id)?;
    let dek = doc_keys
        .iter()
        .find(|(id, _)| *id == doc_id)
        .map(|(_, dek)| dek.clone())
        .ok_or_else(|| anyhow::anyhow!("login response has no membership for the primary doc"))?;
    // Empty doc baseline; the initial sync below pulls from seq 0,
    // applies device-1's seed + history, and we converge. secrets.toml
    // is written last — it's the "logged in" marker.
    let storage = crate::storage::open_storage(&profile)?;
    crate::storage::seed_snapshot(&storage, &dek, doc_id, &Doc::empty())?;
    storage.write_account(&Account {
        account_id: resp.account_id.clone(),
        email,
        device_id: device.device_id,
        primary_doc_id: doc_id,
    })?;
    profile.write_config(&Config {
        server_url: args.server,
    })?;
    profile.write_secrets(&Secrets::new(
        device.device_token,
        doc_keys.iter().map(|(id, dek)| (*id, dek)),
    ))?;

    println!("Syncing…");
    let session = Session::open(true).await?;
    session.flush().await?;

    println!("Logged in to account {}", resp.account_id);
    if !resp.recovery_present {
        println!("(no recovery code is enrolled — losing your password loses your data)");
    }
    Ok(())
}

pub(crate) fn parse_doc_id(hex: &str) -> anyhow::Result<DocId> {
    uuid::Uuid::parse_str(hex)
        .map(DocId)
        .map_err(|e| anyhow::anyhow!("server returned malformed doc id {hex:?}: {e}"))
}

/// Open one server-held wrap with `kek` (password or recovery KEK).
pub(crate) fn unwrap_doc_key(kek: &Kek, wrapped_dek: &[u8], nonce: &[u8]) -> anyhow::Result<Dek> {
    let nonce: [u8; monoplan_core::AEAD_NONCE_LEN] = nonce
        .try_into()
        .map_err(|_| anyhow::anyhow!("server returned wrapped_dek_nonce of wrong length"))?;
    Ok(kek.unwrap(&WrappedDek {
        ciphertext: wrapped_dek.to_vec(),
        nonce,
    })?)
}
