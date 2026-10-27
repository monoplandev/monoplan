use std::sync::OnceLock;

use monoplan_cli::config::{Config, Profile, Secrets};
use monoplan_cli::keystore::{
    derive_master, parse_doc_id, unwrap_doc_key, unwrap_home, unwrap_memberships,
};
use monoplan_cli::net::Client;
use monoplan_cli::storage::Account;
use monoplan_core::{
    Dek, Doc, DocId, Kek, derive_password_master, derive_recovery_master, generate_recovery_code,
    random_bytes,
};
use monoplan_protocol::{
    CreateDocRequest, CreateDocResponse, DeviceCredential, DeviceRegistration, DocKey,
    DocsListResponse, KdfParams, LoginRequest, LoginResponse, PasswordChangeRequest,
    PasswordResetRequest, PasswordResetResponse, PreloginRequest, PreloginResponse, RecoverRequest,
    RecoverResponse, RecoveryMaterial, SignupRequest, SignupResponse,
};
use monoplan_server::sync::queries;
use monoplan_server::{AppState, router};
use reqwest::header::CONTENT_TYPE;
use uuid::Uuid;

const MSGPACK: &str = "application/msgpack";
pub const TEST_PASSWORD: &str = "correct horse battery staple";

pub struct SignedUpAccount {
    pub email: String,
    pub account_id: String,
    pub primary_doc_id: String,
    pub device_id: String,
    pub device_token: String,
    pub dek: Dek,
    pub master_salt: Vec<u8>,
    pub kdf_params: KdfParams,
    pub recovery_code: String,
    pub recovery_salt: Vec<u8>,
}

pub struct LoggedInDevice {
    pub account_id: String,
    pub primary_doc_id: String,
    pub device_id: String,
    pub device_token: String,
    /// The Home DEK (primary doc).
    pub dek: Dek,
    /// Every membership's key, server order, primary first — what
    /// `monoplan auth login` writes to `secrets.toml`.
    pub doc_keys: Vec<(DocId, Dek)>,
    pub recovery_present: bool,
}

pub struct RecoveredDevice {
    pub account_id: String,
    pub primary_doc_id: String,
    pub device_id: String,
    pub device_token: String,
    /// The Home DEK (primary doc).
    pub dek: Dek,
    /// Every membership's key, from `GET /api/docs` after the reset.
    pub doc_keys: Vec<(DocId, Dek)>,
}

pub fn weak_params() -> KdfParams {
    KdfParams {
        m_kib: 8,
        t: 1,
        p: 1,
    }
}

pub struct TestServer {
    pub base: String,
    pub state: AppState,
    handle: tokio::task::JoinHandle<()>,
}

impl TestServer {
    pub async fn start() -> Self {
        Self::start_with_state(AppState::open_in_memory().await.unwrap()).await
    }

    pub async fn start_with_state(state: AppState) -> Self {
        let app = router(state.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let base = format!("http://{}", addr);
        let handle = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        Self {
            base,
            state,
            handle,
        }
    }
}

impl Drop for TestServer {
    fn drop(&mut self) {
        self.handle.abort();
    }
}

pub fn http() -> &'static reqwest::Client {
    static C: OnceLock<reqwest::Client> = OnceLock::new();
    C.get_or_init(reqwest::Client::new)
}

/// A recovery code enrolment for `dek`: the material signup ships plus
/// the code and salt a test needs to redeem it later.
pub fn enrol_recovery(dek: &Dek, kdf_params: KdfParams) -> (RecoveryMaterial, String, Vec<u8>) {
    let code = generate_recovery_code().unwrap();
    let salt: [u8; 16] = random_bytes();
    let recovery_master = derive_recovery_master(code.as_str(), &salt, kdf_params).unwrap();
    let recovery_wrapped = recovery_master.kek().unwrap().wrap(dek).unwrap();
    (
        RecoveryMaterial {
            recovery_salt: salt.to_vec(),
            recovery_auth_secret: recovery_master.auth_secret().unwrap().as_bytes().to_vec(),
            recovery_wrapped_dek: recovery_wrapped.ciphertext,
            recovery_wrapped_dek_nonce: recovery_wrapped.nonce.to_vec(),
        },
        code.as_str().to_string(),
        salt.to_vec(),
    )
}

pub async fn signup_account(server: &TestServer, device_name: &str) -> SignedUpAccount {
    let email = format!("user-{}@example.com", Uuid::now_v7());
    let dek = Dek::generate();
    let kdf_params = weak_params();
    let master_salt: [u8; 16] = random_bytes();
    let master =
        derive_password_master(TEST_PASSWORD.as_bytes(), &master_salt, kdf_params).unwrap();
    let kek = master.kek().unwrap();
    let auth_secret = master.auth_secret().unwrap();
    let wrapped = kek.wrap(&dek).unwrap();

    let (recovery, recovery_code, recovery_salt) = enrol_recovery(&dek, kdf_params);

    let req = SignupRequest {
        email: email.clone(),
        master_salt: master_salt.to_vec(),
        kdf_params,
        auth_secret: auth_secret.as_bytes().to_vec(),
        wrapped_dek: wrapped.ciphertext,
        wrapped_dek_nonce: wrapped.nonce.to_vec(),
        recovery,
        device_name: device_name.into(),
        device_kind: "linux".into(),
        primary_doc_id: None,
    };
    let bytes = rmp_serde::to_vec_named(&req).unwrap();
    let resp = http()
        .post(format!("{}/api/account/signup", server.base))
        .header(CONTENT_TYPE, MSGPACK)
        .body(bytes)
        .send()
        .await
        .unwrap();
    assert!(
        resp.status().is_success(),
        "signup failed: {}",
        resp.status()
    );
    let signup: SignupResponse = rmp_serde::from_slice(&resp.bytes().await.unwrap()).unwrap();

    SignedUpAccount {
        email,
        account_id: signup.account_id,
        primary_doc_id: signup.primary_doc_id,
        device_id: signup.device_id,
        device_token: signup.device_token,
        dek,
        master_salt: master_salt.to_vec(),
        kdf_params,
        recovery_code,
        recovery_salt,
    }
}

pub async fn signup_via_http(server: &TestServer, dek: &Dek, device_name: &str) -> SignupResponse {
    let kdf_params = weak_params();
    let master_salt: [u8; 16] = random_bytes();
    let master =
        derive_password_master(TEST_PASSWORD.as_bytes(), &master_salt, kdf_params).unwrap();
    let kek = master.kek().unwrap();
    let auth_secret = master.auth_secret().unwrap();
    let wrapped = kek.wrap(dek).unwrap();
    let req = SignupRequest {
        email: format!("user-{}@example.com", Uuid::now_v7()),
        master_salt: master_salt.to_vec(),
        kdf_params,
        auth_secret: auth_secret.as_bytes().to_vec(),
        wrapped_dek: wrapped.ciphertext,
        wrapped_dek_nonce: wrapped.nonce.to_vec(),
        recovery: enrol_recovery(dek, kdf_params).0,
        device_name: device_name.into(),
        device_kind: "linux".into(),
        primary_doc_id: None,
    };
    let bytes = rmp_serde::to_vec_named(&req).unwrap();
    let resp = http()
        .post(format!("{}/api/account/signup", server.base))
        .header(CONTENT_TYPE, MSGPACK)
        .body(bytes)
        .send()
        .await
        .unwrap();
    assert!(
        resp.status().is_success(),
        "signup failed: {}",
        resp.status()
    );
    rmp_serde::from_slice(&resp.bytes().await.unwrap()).unwrap()
}

pub async fn login_device(
    server_url: &str,
    email: &str,
    password: &str,
    device_name: &str,
) -> anyhow::Result<LoggedInDevice> {
    let client = Client::new(server_url);
    let pre: PreloginResponse = client
        .post(
            "/api/account/prelogin",
            &PreloginRequest {
                email: email.into(),
            },
        )
        .await?;
    let master = derive_master(password, &pre.master_salt, pre.kdf_params)?;
    let kek = master.kek()?;
    let auth_secret = master.auth_secret()?;

    let resp: LoginResponse = client
        .post(
            "/api/account/login",
            &LoginRequest {
                email: email.into(),
                auth_secret: auth_secret.as_bytes().to_vec(),
                register_device: Some(DeviceRegistration {
                    name: device_name.into(),
                    kind: "linux".into(),
                }),
            },
        )
        .await?;
    let device = resp
        .device
        .ok_or_else(|| anyhow::anyhow!("login did not mint a device credential"))?;
    // Mirror `monoplan auth login`: the primary membership opens with the
    // KEK (Home DEK), every other membership opens with the Home DEK.
    let primary_doc_id = parse_doc_id(&resp.primary_doc_id)?;
    let dek = unwrap_home(&kek, primary_doc_id, &resp.memberships)?;
    let doc_keys = unwrap_memberships(&dek, primary_doc_id, &resp.memberships)?;

    Ok(LoggedInDevice {
        account_id: resp.account_id,
        primary_doc_id: resp.primary_doc_id,
        device_id: device.device_id,
        device_token: device.device_token,
        dek,
        doc_keys,
        recovery_present: resp.recovery_present,
    })
}

/// Change password, re-wrapping the Home DEK (`primary_doc_id`, `home`)
/// — the only key the password KEK wraps. The server rejects any other
/// set.
pub async fn change_password(
    server_url: &str,
    email: &str,
    device_token: &str,
    primary_doc_id: &str,
    home: &Dek,
    current_password: &str,
    new_password: &str,
) -> anyhow::Result<()> {
    let client = Client::new(server_url);
    let pre: PreloginResponse = client
        .post(
            "/api/account/prelogin",
            &PreloginRequest {
                email: email.into(),
            },
        )
        .await?;
    let current_master = derive_master(current_password, &pre.master_salt, pre.kdf_params)?;
    let current_auth = current_master.auth_secret()?;

    let new_master_salt: [u8; 16] = random_bytes();
    let new_kdf_params = weak_params();
    let new_master = derive_master(new_password, &new_master_salt, new_kdf_params)?;
    let new_kek = new_master.kek()?;
    let new_auth = new_master.auth_secret()?;
    let new_doc_keys = vec![wrap_doc_key(&new_kek, primary_doc_id, home)?];

    client
        .post_authed_no_response(
            "/api/account/password/change",
            device_token,
            &PasswordChangeRequest {
                current_auth_secret: current_auth.as_bytes().to_vec(),
                new_master_salt: new_master_salt.to_vec(),
                new_kdf_params,
                new_auth_secret: new_auth.as_bytes().to_vec(),
                new_doc_keys,
            },
        )
        .await?;
    Ok(())
}

fn wrap_doc_key(kek: &Kek, doc_id: &str, dek: &Dek) -> anyhow::Result<DocKey> {
    let w = kek.wrap(dek)?;
    Ok(DocKey {
        doc_id: doc_id.to_string(),
        wrapped_dek: w.ciphertext,
        wrapped_dek_nonce: w.nonce.to_vec(),
    })
}

/// `POST /api/docs`: a second doc the account owns, its DEK wrapped
/// under `home` the way `monoplan` clients will. Returns `(doc_id, dek)`.
pub async fn create_doc(
    server_url: &str,
    device_token: &str,
    home: &Dek,
) -> anyhow::Result<(String, Dek)> {
    let dek = Dek::generate();
    let w = home.wrap_dek(&dek)?;
    let resp: CreateDocResponse = Client::new(server_url)
        .post_authed(
            "/api/docs",
            device_token,
            &CreateDocRequest {
                wrapped_dek: w.ciphertext,
                wrapped_dek_nonce: w.nonce.to_vec(),
            },
        )
        .await?;
    Ok((resp.doc_id, dek))
}

pub async fn recover_device(
    server_url: &str,
    email: &str,
    recovery_code: &str,
    new_password: &str,
    device_name: &str,
) -> anyhow::Result<RecoveredDevice> {
    let client = Client::new(server_url);
    let pre: PreloginResponse = client
        .post(
            "/api/account/prelogin",
            &PreloginRequest {
                email: email.into(),
            },
        )
        .await?;
    let recovery_salt = pre
        .recovery_salt
        .ok_or_else(|| anyhow::anyhow!("account has no recovery code"))?;
    let recovery_master = derive_recovery_master(recovery_code, &recovery_salt, pre.kdf_params)?;
    let recovery_kek = recovery_master.kek()?;
    let recovery_auth = recovery_master.auth_secret()?;

    let recovered: RecoverResponse = client
        .post(
            "/api/account/recover",
            &RecoverRequest {
                email: email.into(),
                recovery_auth_secret: recovery_auth.as_bytes().to_vec(),
            },
        )
        .await?;
    // Mirror `monoplan auth recover`: the one recovered key is the Home DEK;
    // re-wrap it alone for the reset, then learn the other docs from
    // `GET /api/docs` and open them with the Home DEK.
    let dek = recovered
        .doc_keys
        .iter()
        .find(|k| k.doc_id == recovered.primary_doc_id)
        .map(|k| unwrap_doc_key(&recovery_kek, &k.wrapped_dek, &k.wrapped_dek_nonce))
        .ok_or_else(|| anyhow::anyhow!("recovery response lacks the primary doc key"))??;

    let new_master_salt: [u8; 16] = random_bytes();
    let new_kdf_params = weak_params();
    let new_master = derive_master(new_password, &new_master_salt, new_kdf_params)?;
    let new_kek = new_master.kek()?;
    let new_auth = new_master.auth_secret()?;
    let new_doc_keys = vec![wrap_doc_key(&new_kek, &recovered.primary_doc_id, &dek)?];

    let reset: PasswordResetResponse = client
        .post(
            "/api/account/password/reset",
            &PasswordResetRequest {
                recovery_session_token: recovered.recovery_session_token,
                new_master_salt: new_master_salt.to_vec(),
                new_kdf_params,
                new_auth_secret: new_auth.as_bytes().to_vec(),
                new_doc_keys,
                device_name: device_name.into(),
                device_kind: "linux".into(),
            },
        )
        .await?;

    let docs: DocsListResponse = client.get_authed("/api/docs", &reset.device_token).await?;
    let doc_keys = unwrap_memberships(
        &dek,
        parse_doc_id(&reset.primary_doc_id)?,
        &docs.memberships,
    )?;

    Ok(RecoveredDevice {
        account_id: recovered.account_id,
        primary_doc_id: reset.primary_doc_id,
        device_id: reset.device_id,
        device_token: reset.device_token,
        dek,
        doc_keys,
    })
}

pub async fn register_device(
    server: &TestServer,
    owner_token: &str,
    name: &str,
) -> DeviceCredential {
    let bytes = rmp_serde::to_vec_named(&DeviceRegistration {
        name: name.into(),
        kind: "linux".into(),
    })
    .unwrap();
    let resp = http()
        .post(format!("{}/api/devices", server.base))
        .header(CONTENT_TYPE, MSGPACK)
        .bearer_auth(owner_token)
        .body(bytes)
        .send()
        .await
        .unwrap();
    assert!(
        resp.status().is_success(),
        "register_device failed: {}",
        resp.status()
    );
    rmp_serde::from_slice(&resp.bytes().await.unwrap()).unwrap()
}

/// Stand up the on-disk profile a real `monoplan auth signup` would have
/// written. Constructed directly under `data_dir` so each test owns
/// its profile and parallel runs don't race on `MONOPLAN_DATA_DIR`.
pub async fn materialize_profile(
    data_dir: &std::path::Path,
    server_url: &str,
    account_id: &str,
    primary_doc_id: &str,
    device_id: &str,
    device_token: &str,
    dek: &Dek,
    email: &str,
    seed_doc: bool,
) -> Profile {
    std::fs::create_dir_all(data_dir).unwrap();
    let profile = Profile::new(data_dir.to_path_buf());
    let doc = if seed_doc {
        Doc::new().unwrap()
    } else {
        Doc::empty()
    };
    let doc_uuid = Uuid::parse_str(primary_doc_id).expect("test passed malformed primary_doc_id");
    let doc_id = DocId(doc_uuid);
    let storage = monoplan_cli::storage::open_storage(&profile).unwrap();
    monoplan_cli::storage::seed_snapshot(&storage, dek, doc_id, &doc).unwrap();
    storage
        .write_account(&Account {
            account_id: account_id.into(),
            email: email.into(),
            device_id: device_id.into(),
            primary_doc_id: doc_id,
        })
        .unwrap();
    profile
        .write_config(&Config {
            server_url: server_url.into(),
        })
        .unwrap();
    profile
        .write_secrets(&Secrets::new(device_token.into(), [(doc_id, dek)]))
        .unwrap();
    profile
}

pub async fn materialize_signup_profile(
    data_dir: &std::path::Path,
    server_url: &str,
    signup: &SignupResponse,
    dek: &Dek,
    email: &str,
    seed_doc: bool,
) -> Profile {
    materialize_profile(
        data_dir,
        server_url,
        &signup.account_id,
        &signup.primary_doc_id,
        &signup.device_id,
        &signup.device_token,
        dek,
        email,
        seed_doc,
    )
    .await
}

pub fn reopen_profile(data_dir: &std::path::Path) -> Profile {
    Profile::new(data_dir.to_path_buf())
}

/// Poll the server until `doc_id` holds at least `target` update blobs:
/// the push barrier for tests that drive the real binary, since the
/// daemon it spawns pushes asynchronously. Bounded at two seconds.
pub async fn wait_for_updates(
    server: &TestServer,
    doc_id: Uuid,
    target: usize,
) -> queries::FetchedBatch {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
    loop {
        let batch = queries::fetch_updates_batch(&server.state.db, doc_id, 0)
            .await
            .unwrap();
        if batch.updates.len() >= target {
            return batch;
        }
        if std::time::Instant::now() > deadline {
            panic!(
                "updates never reached target={target} (got {})",
                batch.updates.len()
            );
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
}
