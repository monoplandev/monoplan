use clap::Parser;
use dialoguer::{Input, Password};
use monoplan_core::Doc;
use monoplan_protocol::{
    DeviceRegistration, LoginRequest, LoginResponse, PreloginRequest, PreloginResponse,
};

use crate::client::{self, Spawn};
use crate::config::{Config, Profile, Secrets};
use crate::keystore::{derive_master, parse_doc_id, unwrap_home, unwrap_memberships};
use crate::net::Client;
use crate::storage::{Account, Identity};

use super::{default_device_name, default_server, device_kind};

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
    let password: String = Password::new().with_prompt("Password").interact()?;
    let device_name = args.device_name.unwrap_or_else(|| {
        Input::new()
            .with_prompt("Device name")
            .default(default_device_name())
            .interact_text()
            .unwrap_or_else(|_| default_device_name())
    });

    let profile = Profile::create()?;
    super::confirm_discard_local_workspace(&profile)?;
    println!("Deriving keys…");
    let outcome = login(
        &profile,
        Credentials {
            server: args.server,
            email,
            password,
            device_name,
        },
    )
    .await?;

    println!("Syncing…");
    if let Err(e) = client::sync(&profile, spawn).await {
        eprintln!("offline — sync deferred ({e})");
    }

    println!("Logged in to account {}", outcome.account_id);
    if !outcome.recovery_present {
        println!(
            "(this account has opted out of a recovery code — losing your password loses your data)"
        );
    }
    Ok(())
}

/// What a sign-in needs: prompted for by `run`, typed into a sheet by
/// the app (`monoplan-ffi::login`).
pub struct Credentials {
    pub server: String,
    pub email: String,
    pub password: String,
    pub device_name: String,
}

pub struct Outcome {
    pub account_id: String,
    pub recovery_present: bool,
}

/// Sign `profile` in: derive the keys, log in and register this
/// device, then write the profile. The profile is claimed
/// (`client::claim`: any host stopped, the lock held) only once the
/// server has accepted the credentials, and released on return, so a
/// wrong password leaves a running host alone and the app (or the
/// caller's sync) hosts the new profile straight after.
pub async fn login(profile: &Profile, creds: Credentials) -> anyhow::Result<Outcome> {
    let Credentials {
        server,
        email,
        password,
        device_name,
    } = creds;
    let client = Client::new(server.clone());
    let pre: PreloginResponse = client
        .post(
            "/api/account/prelogin",
            &PreloginRequest {
                email: email.clone(),
            },
        )
        .await?;

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
                register_device: Some(DeviceRegistration {
                    name: device_name,
                    kind: device_kind(),
                }),
            },
        )
        .await?;

    // The primary membership's wrap is under the KEK just derived and
    // yields the Home DEK; every other membership's wrap is under the
    // Home DEK. The primary doc is the one this CLI runs.
    let doc_id = parse_doc_id(&resp.primary_doc_id)?;
    let dek = unwrap_home(&kek, doc_id, &resp.memberships)?;
    let doc_keys = unwrap_memberships(&dek, doc_id, &resp.memberships)?;

    let device = resp.device.ok_or_else(|| {
        anyhow::anyhow!("server did not return a device credential despite register_device")
    })?;

    // A host for a previous login would keep the old doc open under
    // the new credentials: stop it, and hold the profile until the new
    // secrets are written so nothing boots the half-written one.
    let _claim = crate::client::claim(&profile.dir).await?;
    // Empty doc baseline; the initial sync pulls from seq 0, applies
    // device-1's seed + history, and we converge. secrets.toml is
    // written last — it's the "logged in" marker.
    let storage = crate::storage::open_storage(profile)?;
    crate::storage::seed_snapshot(&storage, &dek, doc_id, &Doc::empty())?;
    storage.write_account(&Account {
        primary_doc_id: doc_id,
        identity: Some(Identity {
            account_id: resp.account_id.clone(),
            email,
            device_id: device.device_id,
        }),
    })?;
    profile.write_config(&Config { server_url: server })?;
    profile.write_secrets(&Secrets::new(
        Some(device.device_token),
        doc_keys.iter().map(|(id, dek)| (*id, dek)),
    ))?;

    Ok(Outcome {
        account_id: resp.account_id,
        recovery_present: resp.recovery_present,
    })
}
