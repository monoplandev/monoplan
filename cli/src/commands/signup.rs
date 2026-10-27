use clap::Parser;
use dialoguer::Input;
use monoplan_core::{Dek, DocId, generate_recovery_code, random_bytes};
use monoplan_protocol::{KdfParams, RecoveryMaterial, SignupRequest, SignupResponse};

use crate::client::{self, Spawn};
use crate::config::{Config, Profile, Secrets};
use crate::keystore::derive_master;
use crate::net::Client;
use crate::storage::{Account, Identity};

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
    let password = prompt_new_password("Password")?;
    let device_name = args.device_name.unwrap_or_else(|| {
        Input::new()
            .with_prompt("Device name")
            .default(default_device_name())
            .interact_text()
            .unwrap_or_else(|_| default_device_name())
    });

    println!("Deriving keys (this takes a moment)…");
    let profile = Profile::create()?;
    let outcome = signup(
        &profile,
        Credentials {
            server: args.server,
            email: email.clone(),
            password,
            device_name,
            kdf_params: KdfParams::DEFAULT,
        },
    )
    .await?;

    println!("Account created: {email} ({})", outcome.account_id);
    if outcome.adopted {
        println!("Your local workspace is now this account's Personal workspace.");
    }
    let code = outcome.recovery_code.as_str();
    println!();
    println!("RECOVERY CODE — write this down NOW. It will not be shown again.");
    println!();
    println!("    {code}");
    println!();
    use std::io::{BufRead, Write};
    let stdin = std::io::stdin();
    let mut stdout = std::io::stdout();
    loop {
        print!("Type the recovery code back to confirm: ");
        stdout.flush()?;
        let mut line = String::new();
        stdin.lock().read_line(&mut line)?;
        let trimmed = line.trim();
        match monoplan_core::parse_recovery_code(trimmed) {
            Ok(parsed) if parsed.as_str() == code => break,
            Ok(_) => println!("does not match the generated code"),
            Err(_) => println!("not a valid 12-word phrase"),
        }
    }
    println!("Recovery code confirmed.");

    // Ship the doc now (the adopted history, or the fresh seed) rather
    // than on the next command; best-effort, the daemon retries.
    println!("Syncing…");
    if let Err(e) = client::sync(&profile, spawn).await {
        eprintln!("offline — sync deferred ({e})");
    }
    Ok(())
}

/// What a signup needs: prompted for by `run`; a sheet could type it in.
pub struct Credentials {
    pub server: String,
    pub email: String,
    pub password: String,
    pub device_name: String,
    /// `KdfParams::DEFAULT` outside tests.
    pub kdf_params: KdfParams,
}

pub struct Outcome {
    pub account_id: String,
    /// Shown once by the caller; never persisted.
    pub recovery_code: String,
    /// The profile held a local-only workspace and the account now
    /// owns it: same doc id, same key, history intact.
    pub adopted: bool,
}

/// Create an account and make `profile` its first device. A profile
/// that holds a local-only workspace (`crate::bootstrap`) is adopted:
/// its DEK becomes the Home DEK (wrapped under the new password KEK
/// and the recovery code exactly as a fresh one would be) and its doc
/// id is sent as `primary_doc_id`, so nothing is copied or
/// re-encrypted and the engine's empty `server_known_vv` pushes the
/// whole local history on first connect (`spec/auth.md` §Device
/// pairing). Any other profile (none, or an earlier account's) gets a
/// freshly minted and seeded doc, as before.
///
/// The profile is claimed (`client::claim`: any host stopped, the lock
/// held) only once the server has accepted the signup, and released on
/// return, so a rejected signup leaves a running host alone.
pub async fn signup(profile: &Profile, creds: Credentials) -> anyhow::Result<Outcome> {
    let Credentials {
        server,
        email,
        password,
        device_name,
        kdf_params,
    } = creds;

    let adopt = crate::bootstrap::local_only_doc(profile)?;

    let master_salt: [u8; 16] = random_bytes();
    let master = tokio::task::spawn_blocking({
        let pw = password;
        let salt = master_salt.to_vec();
        move || derive_master(&pw, &salt, kdf_params)
    })
    .await??;

    let kek = master.kek()?;
    let auth_secret = master.auth_secret()?;
    // The primary doc's DEK is the Home DEK: the one key the password
    // KEK (and, below, the recovery KEK) ever wraps. Docs created later
    // are wrapped under it, not under the KEK (spec/encryption.md). The
    // server never sees it, so an adopted key is indistinguishable
    // from a new one on its side.
    let dek = match &adopt {
        Some((_, dek)) => dek.clone(),
        None => Dek::generate(),
    };
    let wrapped = kek.wrap(&dek)?;

    // Every account enrols a recovery code (spec/encryption.md
    // §Recovery tiers): the same Home DEK under a KEK derived from the
    // 12 words, shown once by the caller.
    let recovery_code = generate_recovery_code()?;
    let recovery_material = {
        let recovery_salt: [u8; 16] = random_bytes();
        let code_str = recovery_code.as_str().to_string();
        let salt_vec = recovery_salt.to_vec();
        let r_master = tokio::task::spawn_blocking(move || {
            monoplan_core::derive_recovery_master(&code_str, &salt_vec, kdf_params)
        })
        .await??;
        let r_kek = r_master.kek()?;
        let r_auth = r_master.auth_secret()?;
        let r_wrapped = r_kek.wrap(&dek)?;
        RecoveryMaterial {
            recovery_salt: recovery_salt.to_vec(),
            recovery_auth_secret: r_auth.as_bytes().to_vec(),
            recovery_wrapped_dek: r_wrapped.ciphertext,
            recovery_wrapped_dek_nonce: r_wrapped.nonce.to_vec(),
        }
    };

    let client = Client::new(server.clone());
    let resp: SignupResponse = client
        .post(
            "/api/account/signup",
            &SignupRequest {
                email: email.clone(),
                master_salt: master_salt.to_vec(),
                kdf_params,
                auth_secret: auth_secret.as_bytes().to_vec(),
                wrapped_dek: wrapped.ciphertext,
                wrapped_dek_nonce: wrapped.nonce.to_vec(),
                recovery: recovery_material,
                device_name,
                device_kind: device_kind(),
                primary_doc_id: adopt.as_ref().map(|(id, _)| id.0.to_string()),
            },
        )
        .await?;

    let primary_doc_uuid = uuid::Uuid::parse_str(&resp.primary_doc_id)
        .map_err(|e| anyhow::anyhow!("server returned malformed primary_doc_id: {e}"))?;
    let doc_id = DocId(primary_doc_uuid);
    if let Some((local_id, _)) = &adopt
        && *local_id != doc_id
    {
        // The server honours the id or rejects it (409); a silent
        // substitution would strand the local history under the old id.
        anyhow::bail!("server did not adopt the local doc id at signup");
    }

    // A host for a previous login (or for the local-only workspace)
    // would keep the doc open under stale credentials: stop it, and
    // hold the profile until the new secrets are written so nothing
    // boots the half-written one.
    let _claim = client::claim(&profile.dir).await?;
    let storage = crate::storage::open_storage(profile)?;
    if adopt.is_none() {
        // Device-1 (genesis) of a fresh account: seed the onboarding
        // doc as the baseline snapshot under the server's doc id. It
        // travels to future devices through snapshot bootstrap.
        let peer = crate::peer::claim(&profile.dir, &storage)?;
        let doc = crate::bootstrap::welcome_doc(peer.peer_id)?;
        crate::storage::seed_snapshot(&storage, &dek, doc_id, &doc)?;
    }
    storage.write_account(&Account {
        primary_doc_id: doc_id,
        identity: Some(Identity {
            account_id: resp.account_id.clone(),
            email,
            device_id: resp.device_id,
        }),
    })?;
    profile.write_config(&Config { server_url: server })?;
    // Written last — with the token it is the "has an account" marker.
    profile.write_secrets(&Secrets::new(Some(resp.device_token), [(doc_id, &dek)]))?;

    Ok(Outcome {
        account_id: resp.account_id,
        recovery_code: recovery_code.as_str().to_string(),
        adopted: adopt.is_some(),
    })
}
