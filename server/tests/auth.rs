//! End-to-end auth: real server, real sqlite, real reqwest, real crypto.
//!
//! Each test spins up a fresh `:memory:` server on a random port. We
//! use weak Argon2 params (`m=8 KiB, t=1, p=1`) so the test pass stays
//! under a second — the crypto correctness is covered by `core` unit
//! tests.

use monoplan_core::{
    AEAD_NONCE_LEN, Dek, InviteSecret, derive_password_master, derive_recovery_master,
    generate_recovery_code, random_bytes,
};
use monoplan_protocol::{
    AcceptInviteRequest, AcceptInviteResponse, CreateDocRequest, CreateDocResponse,
    CreateInviteRequest, CreateInviteResponse, DeviceRenameRequest, DevicesListResponse, DocKey,
    DocsListResponse, InvitePayloadResponse, InvitesListResponse, KdfParams, LoginRequest,
    LoginResponse, MemberRole, MembersListResponse, PasswordChangeRequest, PasswordResetRequest,
    PreloginRequest, PreloginResponse, RecoverRequest, RecoverResponse, RecoveryMaterial,
    SignupRequest, SignupResponse,
};
use monoplan_server::{AppState, router};
use reqwest::header::CONTENT_TYPE;
use rusqlite::params;
use serde::Serialize;
use serde::de::DeserializeOwned;
use std::sync::OnceLock;
use uuid::Uuid;

const MSGPACK: &str = "application/msgpack";

fn weak_params() -> KdfParams {
    KdfParams {
        m_kib: 8,
        t: 1,
        p: 1,
    }
}

struct TestServer {
    base: String,
    state: AppState,
    handle: tokio::task::JoinHandle<()>,
}

impl TestServer {
    async fn start() -> Self {
        let state = AppState::open_in_memory().await.unwrap();
        let app = router(state.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let base = format!("http://{}", addr);
        let handle = tokio::spawn(async move {
            axum::serve(
                listener,
                app.into_make_service_with_connect_info::<std::net::SocketAddr>(),
            )
            .await
            .unwrap();
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

fn http() -> &'static reqwest::Client {
    static C: OnceLock<reqwest::Client> = OnceLock::new();
    C.get_or_init(reqwest::Client::new)
}

async fn post_msgpack<Req, Resp>(url: &str, body: &Req, token: Option<&str>) -> Resp
where
    Req: Serialize,
    Resp: DeserializeOwned,
{
    let bytes = rmp_serde::to_vec_named(body).unwrap();
    let mut req = http().post(url).header(CONTENT_TYPE, MSGPACK).body(bytes);
    if let Some(t) = token {
        req = req.bearer_auth(t);
    }
    let resp = req.send().await.unwrap();
    assert!(
        resp.status().is_success(),
        "expected 2xx, got {}: {}",
        resp.status(),
        resp.text().await.unwrap_or_default()
    );
    let bytes = resp.bytes().await.unwrap();
    rmp_serde::from_slice(&bytes).unwrap()
}

async fn post_msgpack_no_response<Req: Serialize>(url: &str, body: &Req, token: Option<&str>) {
    let bytes = rmp_serde::to_vec_named(body).unwrap();
    let mut req = http().post(url).header(CONTENT_TYPE, MSGPACK).body(bytes);
    if let Some(t) = token {
        req = req.bearer_auth(t);
    }
    let resp = req.send().await.unwrap();
    assert!(
        resp.status().is_success(),
        "expected 2xx, got {}: {}",
        resp.status(),
        resp.text().await.unwrap_or_default()
    );
}

async fn post_msgpack_status<Req: Serialize>(url: &str, body: &Req) -> reqwest::StatusCode {
    let bytes = rmp_serde::to_vec_named(body).unwrap();
    let resp = http()
        .post(url)
        .header(CONTENT_TYPE, MSGPACK)
        .body(bytes)
        .send()
        .await
        .unwrap();
    resp.status()
}

/// Returns `(status, set-cookie values, decoded body)`. Used by cookie
/// tests to assert both transport behaviour and response content.
async fn post_msgpack_full<Req, Resp>(
    url: &str,
    body: &Req,
    token: Option<&str>,
    cookie: Option<&str>,
) -> (reqwest::StatusCode, Vec<String>, Option<Resp>)
where
    Req: Serialize,
    Resp: DeserializeOwned,
{
    let bytes = rmp_serde::to_vec_named(body).unwrap();
    let mut req = http().post(url).header(CONTENT_TYPE, MSGPACK).body(bytes);
    if let Some(t) = token {
        req = req.bearer_auth(t);
    }
    if let Some(c) = cookie {
        req = req.header(reqwest::header::COOKIE, c);
    }
    let resp = req.send().await.unwrap();
    let status = resp.status();
    let set_cookies: Vec<String> = resp
        .headers()
        .get_all(reqwest::header::SET_COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok().map(str::to_owned))
        .collect();
    let bytes = resp.bytes().await.unwrap();
    let parsed = if bytes.is_empty() {
        None
    } else {
        Some(rmp_serde::from_slice(&bytes).unwrap())
    };
    (status, set_cookies, parsed)
}

/// Pull the value of `name=` out of a `Set-Cookie` header line.
fn cookie_value<'a>(set_cookie: &'a str, name: &str) -> Option<&'a str> {
    let prefix = format!("{name}=");
    let rest = set_cookie.strip_prefix(&prefix)?;
    Some(rest.split(';').next().unwrap_or(rest))
}

fn find_set_cookie<'a>(set_cookies: &'a [String], name: &str) -> Option<&'a str> {
    let needle = format!("{name}=");
    set_cookies
        .iter()
        .map(String::as_str)
        .find(|s| s.starts_with(&needle))
}

struct Account {
    server: TestServer,
    email: String,
    account_id: String,
    primary_doc_id: String,
    device_id: String,
    device_token: String,
    dek: Dek,
    master_salt: Vec<u8>,
    recovery_code: Option<String>,
    recovery_salt: Option<Vec<u8>>,
    kdf_params: KdfParams,
}

async fn signup(with_recovery: bool) -> Account {
    let server = TestServer::start().await;
    let email = format!("user-{}@example.com", uuid::Uuid::now_v7());
    let password = "correct horse battery staple";
    let kdf_params = weak_params();

    let master_salt: [u8; 16] = random_bytes();
    let master = derive_password_master(password.as_bytes(), &master_salt, kdf_params).unwrap();
    let kek = master.kek().unwrap();
    let auth_secret = master.auth_secret().unwrap();
    let dek = Dek::generate();
    let wrapped = kek.wrap(&dek).unwrap();

    let mut recovery_code = None;
    let mut recovery_salt: Option<Vec<u8>> = None;
    let recovery = if with_recovery {
        let code = generate_recovery_code().unwrap();
        let salt: [u8; 16] = random_bytes();
        let r_master = derive_recovery_master(code.as_str(), &salt, kdf_params).unwrap();
        let r_kek = r_master.kek().unwrap();
        let r_auth = r_master.auth_secret().unwrap();
        let r_wrapped = r_kek.wrap(&dek).unwrap();
        recovery_code = Some(code.as_str().to_string());
        recovery_salt = Some(salt.to_vec());
        Some(RecoveryMaterial {
            recovery_salt: salt.to_vec(),
            recovery_auth_secret: r_auth.as_bytes().to_vec(),
            recovery_wrapped_dek: r_wrapped.ciphertext,
            recovery_wrapped_dek_nonce: r_wrapped.nonce.to_vec(),
        })
    } else {
        None
    };

    let url = format!("{}/api/account/signup", server.base);
    let resp: SignupResponse = post_msgpack(
        &url,
        &SignupRequest {
            email: email.clone(),
            master_salt: master_salt.to_vec(),
            kdf_params,
            auth_secret: auth_secret.as_bytes().to_vec(),
            wrapped_dek: wrapped.ciphertext,
            wrapped_dek_nonce: wrapped.nonce.to_vec(),
            recovery,
            device_name: "test-device".into(),
            device_kind: "linux".into(),
        },
        None,
    )
    .await;

    Account {
        server,
        email,
        account_id: resp.account_id,
        primary_doc_id: resp.primary_doc_id,
        device_id: resp.device_id,
        device_token: resp.device_token,
        dek,
        master_salt: master_salt.to_vec(),
        recovery_code,
        recovery_salt,
        kdf_params,
    }
}

#[tokio::test]
async fn device_can_be_renamed() {
    let acc = signup(false).await;
    let bytes = rmp_serde::to_vec_named(&DeviceRenameRequest {
        name: "  renamed device  ".into(),
    })
    .unwrap();
    let response = http()
        .patch(format!("{}/api/devices/{}", acc.server.base, acc.device_id))
        .bearer_auth(&acc.device_token)
        .header(CONTENT_TYPE, MSGPACK)
        .body(bytes)
        .send()
        .await
        .unwrap();
    assert!(response.status().is_success());

    let response = http()
        .get(format!("{}/api/devices", acc.server.base))
        .bearer_auth(&acc.device_token)
        .send()
        .await
        .unwrap();
    assert!(response.status().is_success());
    let devices: DevicesListResponse =
        rmp_serde::from_slice(&response.bytes().await.unwrap()).unwrap();
    assert_eq!(devices.devices.len(), 1);
    assert_eq!(devices.devices[0].name, "renamed device");
    // Test server binds loopback with connect info, no proxy header.
    assert_eq!(
        devices.devices[0].last_seen_ip.as_deref(),
        Some("127.0.0.1")
    );
}

#[tokio::test]
async fn login_with_wrong_password_is_rejected() {
    let acc = signup(false).await;

    let pre: PreloginResponse = post_msgpack(
        &format!("{}/api/account/prelogin", acc.server.base),
        &PreloginRequest {
            email: acc.email.clone(),
        },
        None,
    )
    .await;
    let master =
        derive_password_master(b"wrong password", &pre.master_salt, pre.kdf_params).unwrap();
    let auth = master.auth_secret().unwrap();

    let status = post_msgpack_status(
        &format!("{}/api/account/login", acc.server.base),
        &LoginRequest {
            email: acc.email,
            auth_secret: auth.as_bytes().to_vec(),
            register_device: None,
        },
    )
    .await;
    assert_eq!(status, reqwest::StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn prelogin_unknown_email_is_404() {
    let server = TestServer::start().await;
    let status = post_msgpack_status(
        &format!("{}/api/account/prelogin", server.base),
        &PreloginRequest {
            email: "nope@nowhere".into(),
        },
    )
    .await;
    assert_eq!(status, reqwest::StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn signup_duplicate_email_is_409() {
    let acc = signup(false).await;

    // Re-issue the same SignupRequest shape with a fresh-but-irrelevant
    // crypto load.
    let kdf_params = weak_params();
    let master_salt: [u8; 16] = random_bytes();
    let master = derive_password_master(b"x", &master_salt, kdf_params).unwrap();
    let kek = master.kek().unwrap();
    let auth = master.auth_secret().unwrap();
    let dek = Dek::generate();
    let wrapped = kek.wrap(&dek).unwrap();

    let status = post_msgpack_status(
        &format!("{}/api/account/signup", acc.server.base),
        &SignupRequest {
            email: acc.email,
            master_salt: master_salt.to_vec(),
            kdf_params,
            auth_secret: auth.as_bytes().to_vec(),
            wrapped_dek: wrapped.ciphertext,
            wrapped_dek_nonce: wrapped.nonce.to_vec(),
            recovery: None,
            device_name: "dup".into(),
            device_kind: "linux".into(),
        },
    )
    .await;
    assert_eq!(status, reqwest::StatusCode::CONFLICT);
}

#[tokio::test]
async fn email_is_normalised_server_side() {
    // Server owns the canonical form (trim + lowercase): a casing /
    // whitespace variant of an existing email is the same account for
    // prelogin, and a duplicate for signup.
    let acc = signup(false).await;
    let variant = format!("  {} ", acc.email.to_uppercase());

    let status = post_msgpack_status(
        &format!("{}/api/account/prelogin", acc.server.base),
        &PreloginRequest {
            email: variant.clone(),
        },
    )
    .await;
    assert_eq!(status, reqwest::StatusCode::OK);

    let kdf_params = weak_params();
    let master_salt: [u8; 16] = random_bytes();
    let master = derive_password_master(b"x", &master_salt, kdf_params).unwrap();
    let kek = master.kek().unwrap();
    let auth = master.auth_secret().unwrap();
    let dek = Dek::generate();
    let wrapped = kek.wrap(&dek).unwrap();
    let status = post_msgpack_status(
        &format!("{}/api/account/signup", acc.server.base),
        &SignupRequest {
            email: variant,
            master_salt: master_salt.to_vec(),
            kdf_params,
            auth_secret: auth.as_bytes().to_vec(),
            wrapped_dek: wrapped.ciphertext,
            wrapped_dek_nonce: wrapped.nonce.to_vec(),
            recovery: None,
            device_name: "dup".into(),
            device_kind: "linux".into(),
        },
    )
    .await;
    assert_eq!(status, reqwest::StatusCode::CONFLICT);
}

#[tokio::test]
async fn recovery_session_token_cannot_be_reused() {
    let acc = signup(true).await;
    let recovery_code = acc.recovery_code.clone().expect("opted in");
    let recovery_salt = acc.recovery_salt.clone().expect("opted in");

    let pre: PreloginResponse = post_msgpack(
        &format!("{}/api/account/prelogin", acc.server.base),
        &PreloginRequest {
            email: acc.email.clone(),
        },
        None,
    )
    .await;
    assert_eq!(pre.recovery_salt.as_ref(), Some(&recovery_salt));

    let r_master = derive_recovery_master(&recovery_code, &recovery_salt, pre.kdf_params).unwrap();
    let r_kek = r_master.kek().unwrap();
    let r_auth = r_master.auth_secret().unwrap();

    let recovered: RecoverResponse = post_msgpack(
        &format!("{}/api/account/recover", acc.server.base),
        &RecoverRequest {
            email: acc.email.clone(),
            recovery_auth_secret: r_auth.as_bytes().to_vec(),
        },
        None,
    )
    .await;
    assert_eq!(recovered.primary_doc_id, acc.primary_doc_id);
    assert_eq!(
        recovered.doc_keys.len(),
        1,
        "exactly the primary's recovery wrap"
    );
    assert_eq!(recovered.doc_keys[0].doc_id, acc.primary_doc_id);
    let dek = unwrap_doc_key(&r_kek, &recovered.doc_keys[0]);
    assert_eq!(dek.as_bytes(), acc.dek.as_bytes());

    // New password + reset.
    let new_password = "freshly chosen pass";
    let new_salt: [u8; 16] = random_bytes();
    let new_params = weak_params();
    let new_master =
        derive_password_master(new_password.as_bytes(), &new_salt, new_params).unwrap();
    let new_kek = new_master.kek().unwrap();
    let new_auth = new_master.auth_secret().unwrap();

    // A reset that leaves a recovered membership un-rewrapped is a 400
    // and must *not* burn the session.
    let status = post_msgpack_status(
        &format!("{}/api/account/password/reset", acc.server.base),
        &PasswordResetRequest {
            recovery_session_token: recovered.recovery_session_token.clone(),
            new_master_salt: new_salt.to_vec(),
            new_kdf_params: new_params,
            new_auth_secret: new_auth.as_bytes().to_vec(),
            new_doc_keys: vec![],
            device_name: "incomplete".into(),
            device_kind: "linux".into(),
        },
    )
    .await;
    assert_eq!(status, reqwest::StatusCode::BAD_REQUEST);

    let reset: monoplan_protocol::PasswordResetResponse = post_msgpack(
        &format!("{}/api/account/password/reset", acc.server.base),
        &PasswordResetRequest {
            recovery_session_token: recovered.recovery_session_token.clone(),
            new_master_salt: new_salt.to_vec(),
            new_kdf_params: new_params,
            new_auth_secret: new_auth.as_bytes().to_vec(),
            new_doc_keys: vec![wrap_doc_key(&new_kek, &acc.primary_doc_id, &dek)],
            device_name: "post-recover".into(),
            device_kind: "linux".into(),
        },
        None,
    )
    .await;
    assert!(!reset.device_token.is_empty());

    // Recovery session token cannot be reused.
    let status = post_msgpack_status(
        &format!("{}/api/account/password/reset", acc.server.base),
        &PasswordResetRequest {
            recovery_session_token: recovered.recovery_session_token,
            new_master_salt: new_salt.to_vec(),
            new_kdf_params: new_params,
            new_auth_secret: new_auth.as_bytes().to_vec(),
            new_doc_keys: vec![wrap_doc_key(&new_kek, &acc.primary_doc_id, &dek)],
            device_name: "should-fail".into(),
            device_kind: "linux".into(),
        },
    )
    .await;
    assert_eq!(status, reqwest::StatusCode::FORBIDDEN);

    // The new password opens the same DEK via the re-wrapped membership.
    let login = login_with(&acc, new_password).await;
    assert_eq!(login.memberships.len(), 1);
    let kek = derive_password_master(new_password.as_bytes(), &new_salt, new_params)
        .unwrap()
        .kek()
        .unwrap();
    let m = &login.memberships[0];
    let reopened = kek
        .unwrap(&monoplan_core::WrappedDek {
            ciphertext: m.wrapped_dek.clone(),
            nonce: m.wrapped_dek_nonce.as_slice().try_into().unwrap(),
        })
        .unwrap();
    assert_eq!(reopened.as_bytes(), acc.dek.as_bytes());
}

fn wrap_doc_key(kek: &monoplan_core::Kek, doc_id: &str, dek: &Dek) -> DocKey {
    let w = kek.wrap(dek).unwrap();
    DocKey {
        doc_id: doc_id.to_string(),
        wrapped_dek: w.ciphertext,
        wrapped_dek_nonce: w.nonce.to_vec(),
    }
}

fn unwrap_doc_key(kek: &monoplan_core::Kek, key: &DocKey) -> Dek {
    let nonce: [u8; AEAD_NONCE_LEN] = key.wrapped_dek_nonce.as_slice().try_into().unwrap();
    kek.unwrap(&monoplan_core::WrappedDek {
        ciphertext: key.wrapped_dek.clone(),
        nonce,
    })
    .unwrap()
}

/// Prelogin + login (no device registration) with `password`.
async fn login_with(acc: &Account, password: &str) -> LoginResponse {
    let pre: PreloginResponse = post_msgpack(
        &format!("{}/api/account/prelogin", acc.server.base),
        &PreloginRequest {
            email: acc.email.clone(),
        },
        None,
    )
    .await;
    let master =
        derive_password_master(password.as_bytes(), &pre.master_salt, pre.kdf_params).unwrap();
    post_msgpack(
        &format!("{}/api/account/login", acc.server.base),
        &LoginRequest {
            email: acc.email.clone(),
            auth_secret: master.auth_secret().unwrap().as_bytes().to_vec(),
            register_device: None,
        },
        None,
    )
    .await
}

/// Give `acc` a second, directly-seeded membership (a fresh doc, role
/// `member`, DEK wrapped under the account's Home DEK — the shape an
/// accepted invite will have). Stands in for the invite flow that does
/// not exist yet; proves the membership paths never special-case
/// length 1.
async fn seed_extra_membership(acc: &Account, home: &Dek) -> (String, Dek) {
    let doc_id = Uuid::now_v7();
    let dek = Dek::generate();
    let w = home.wrap_dek(&dek).unwrap();
    let account_id = Uuid::parse_str(&acc.account_id).unwrap();
    acc.server
        .state
        .db
        .call(move |c| {
            let tx = c.transaction()?;
            tx.execute(
                "INSERT INTO docs (id, created_at) VALUES (?, 1)",
                [doc_id.as_bytes().as_slice()],
            )?;
            tx.execute(
                "INSERT INTO doc_members (
                    doc_id, account_id, role, wrapped_dek, wrapped_dek_nonce, added_at
                 ) VALUES (?, ?, 'member', ?, ?, ?)",
                params![
                    doc_id.as_bytes().as_slice(),
                    account_id.as_bytes().as_slice(),
                    w.ciphertext,
                    w.nonce.to_vec(),
                    // Later than signup's wall-clock `added_at` so the
                    // primary stays first in `ORDER BY added_at`.
                    i64::MAX / 2,
                ],
            )?;
            tx.commit()?;
            Ok(())
        })
        .await
        .unwrap();
    (doc_id.to_string(), dek)
}

/// Open a non-primary membership's wrap with the Home DEK.
fn unwrap_with_home(home: &Dek, wrapped_dek: &[u8], nonce: &[u8]) -> Dek {
    home.unwrap_dek(&monoplan_core::WrappedDek {
        ciphertext: wrapped_dek.to_vec(),
        nonce: nonce.try_into().unwrap(),
    })
    .unwrap()
}

#[tokio::test]
async fn login_returns_primary_membership_whose_wrap_opens_the_signup_dek() {
    let acc = signup(false).await;
    let password = "correct horse battery staple";
    let login = login_with(&acc, password).await;
    assert_eq!(login.primary_doc_id, acc.primary_doc_id);
    assert_eq!(login.memberships.len(), 1);
    let m = &login.memberships[0];
    assert_eq!(m.doc_id, acc.primary_doc_id);
    assert_eq!(m.role, MemberRole::Owner);
    let kek = derive_password_master(password.as_bytes(), &acc.master_salt, acc.kdf_params)
        .unwrap()
        .kek()
        .unwrap();
    let dek = kek
        .unwrap(&monoplan_core::WrappedDek {
            ciphertext: m.wrapped_dek.clone(),
            nonce: m.wrapped_dek_nonce.as_slice().try_into().unwrap(),
        })
        .unwrap();
    assert_eq!(dek.as_bytes(), acc.dek.as_bytes());
}

#[tokio::test]
async fn password_change_rewraps_only_the_primary_and_other_docs_follow_the_home_dek() {
    let acc = signup(false).await;
    let password = "correct horse battery staple";
    let master =
        derive_password_master(password.as_bytes(), &acc.master_salt, acc.kdf_params).unwrap();
    let current_auth = master.auth_secret().unwrap();
    let (second_doc_id, second_dek) = seed_extra_membership(&acc, &acc.dek).await;

    // Login now reports both memberships, primary first.
    let login = login_with(&acc, password).await;
    let ids: Vec<&str> = login
        .memberships
        .iter()
        .map(|m| m.doc_id.as_str())
        .collect();
    assert_eq!(
        ids,
        vec![acc.primary_doc_id.as_str(), second_doc_id.as_str()]
    );
    assert_eq!(login.memberships[1].role, MemberRole::Member);

    let new_password = "another long password";
    let new_salt: [u8; 16] = random_bytes();
    let new_params = weak_params();
    let new_master =
        derive_password_master(new_password.as_bytes(), &new_salt, new_params).unwrap();
    let new_kek = new_master.kek().unwrap();
    let new_auth = new_master.auth_secret().unwrap();
    let url = format!("{}/api/account/password/change", acc.server.base);
    let req = |keys: Vec<DocKey>| PasswordChangeRequest {
        current_auth_secret: current_auth.as_bytes().to_vec(),
        new_master_salt: new_salt.to_vec(),
        new_kdf_params: new_params,
        new_auth_secret: new_auth.as_bytes().to_vec(),
        new_doc_keys: keys,
    };

    // The old "re-wrap every membership" shape is now a mismatch: the
    // second doc's wrap is under the Home DEK and must not be rewritten.
    let status = post_msgpack_status_authed(
        &url,
        &req(vec![
            wrap_doc_key(&new_kek, &acc.primary_doc_id, &acc.dek),
            wrap_doc_key(&new_kek, &second_doc_id, &second_dek),
        ]),
        &acc.device_token,
    )
    .await;
    assert_eq!(status, reqwest::StatusCode::BAD_REQUEST);
    // Only the non-primary → rejected.
    let status = post_msgpack_status_authed(
        &url,
        &req(vec![wrap_doc_key(&new_kek, &second_doc_id, &second_dek)]),
        &acc.device_token,
    )
    .await;
    assert_eq!(status, reqwest::StatusCode::BAD_REQUEST);
    // Empty → rejected.
    let status = post_msgpack_status_authed(&url, &req(vec![]), &acc.device_token).await;
    assert_eq!(status, reqwest::StatusCode::BAD_REQUEST);
    // Old password still works after the rejected attempts.
    assert_eq!(login_with(&acc, password).await.memberships.len(), 2);

    // Exactly the primary → accepted.
    post_msgpack_no_response(
        &url,
        &req(vec![wrap_doc_key(&new_kek, &acc.primary_doc_id, &acc.dek)]),
        Some(&acc.device_token),
    )
    .await;
    let login = login_with(&acc, new_password).await;
    assert_eq!(login.memberships.len(), 2);
    let primary = &login.memberships[0];
    assert_eq!(primary.doc_id, acc.primary_doc_id);
    let home = unwrap_doc_key(
        &new_kek,
        &DocKey {
            doc_id: primary.doc_id.clone(),
            wrapped_dek: primary.wrapped_dek.clone(),
            wrapped_dek_nonce: primary.wrapped_dek_nonce.clone(),
        },
    );
    assert_eq!(home.as_bytes(), acc.dek.as_bytes());
    // The seeded row is byte-for-byte what we inserted, and the Home
    // DEK still opens it.
    let second = &login.memberships[1];
    assert_eq!(second.doc_id, second_doc_id);
    let reopened = unwrap_with_home(&home, &second.wrapped_dek, &second.wrapped_dek_nonce);
    assert_eq!(reopened.as_bytes(), second_dek.as_bytes());
}

async fn get_msgpack_authed<Resp: DeserializeOwned>(url: &str, token: &str) -> Resp {
    let resp = http().get(url).bearer_auth(token).send().await.unwrap();
    assert!(
        resp.status().is_success(),
        "expected 2xx, got {}: {}",
        resp.status(),
        resp.text().await.unwrap_or_default()
    );
    rmp_serde::from_slice(&resp.bytes().await.unwrap()).unwrap()
}

/// `POST /api/docs` with a fresh DEK wrapped under `home` (the
/// account's Home DEK). Returns the doc id and its DEK.
async fn create_doc(acc: &Account, home: &Dek) -> (String, Dek) {
    let dek = Dek::generate();
    let w = home.wrap_dek(&dek).unwrap();
    let resp: CreateDocResponse = post_msgpack(
        &format!("{}/api/docs", acc.server.base),
        &CreateDocRequest {
            wrapped_dek: w.ciphertext,
            wrapped_dek_nonce: w.nonce.to_vec(),
        },
        Some(&acc.device_token),
    )
    .await;
    (resp.doc_id, dek)
}

/// A second account on an *existing* test server (`signup` starts a
/// server per account). Only what the cross-account tests need.
struct SecondAccount {
    account_id: String,
    device_token: String,
    dek: Dek,
}

async fn signup_on(base: &str) -> SecondAccount {
    let kdf_params = weak_params();
    let master_salt: [u8; 16] = random_bytes();
    let master =
        derive_password_master(b"another password entirely", &master_salt, kdf_params).unwrap();
    let kek = master.kek().unwrap();
    let dek = Dek::generate();
    let wrapped = kek.wrap(&dek).unwrap();
    let resp: SignupResponse = post_msgpack(
        &format!("{}/api/account/signup", base),
        &SignupRequest {
            email: format!("second-{}@example.com", uuid::Uuid::now_v7()),
            master_salt: master_salt.to_vec(),
            kdf_params,
            auth_secret: master.auth_secret().unwrap().as_bytes().to_vec(),
            wrapped_dek: wrapped.ciphertext,
            wrapped_dek_nonce: wrapped.nonce.to_vec(),
            recovery: None,
            device_name: "second-device".into(),
            device_kind: "linux".into(),
        },
        None,
    )
    .await;
    SecondAccount {
        account_id: resp.account_id,
        device_token: resp.device_token,
        dek,
    }
}

async fn delete_status(url: &str, token: &str) -> reqwest::StatusCode {
    http()
        .delete(url)
        .bearer_auth(token)
        .send()
        .await
        .unwrap()
        .status()
}

/// `DELETE /api/docs/:id`: an owner deletes a doc they created and it
/// is gone from the list with every row under it; the primary doc is
/// refused outright (403) and a doc that is not the caller's to delete
/// reads as absent (404).
#[tokio::test]
async fn owner_deletes_created_doc_but_never_the_primary() {
    let acc = signup(false).await;
    let (doc_id, _dek) = create_doc(&acc, &acc.dek).await;
    let docs_url = format!("{}/api/docs", acc.server.base);

    // Seed a row under the doc so the cascade has something to clear.
    let did = Uuid::parse_str(&doc_id).unwrap();
    acc.server
        .state
        .db
        .call(move |c| {
            c.execute(
                "INSERT INTO doc_sequences (doc_id, next_seq, epoch) VALUES (?, 5, 0)",
                [did.as_bytes().as_slice()],
            )?;
            Ok(())
        })
        .await
        .unwrap();

    // The primary is non-deletable, before any ownership lookup.
    assert_eq!(
        delete_status(
            &format!("{}/{}", docs_url, acc.primary_doc_id),
            &acc.device_token
        )
        .await,
        reqwest::StatusCode::FORBIDDEN
    );
    // Garbage id.
    assert_eq!(
        delete_status(&format!("{}/not-a-uuid", docs_url), &acc.device_token).await,
        reqwest::StatusCode::BAD_REQUEST
    );
    // Someone else's account cannot see it, let alone delete it.
    let other = signup_on(&acc.server.base).await;
    assert_eq!(
        delete_status(&format!("{}/{}", docs_url, doc_id), &other.device_token).await,
        reqwest::StatusCode::NOT_FOUND
    );

    // The owner deletes it.
    assert_eq!(
        delete_status(&format!("{}/{}", docs_url, doc_id), &acc.device_token).await,
        reqwest::StatusCode::OK
    );
    let docs: DocsListResponse = get_msgpack_authed(&docs_url, &acc.device_token).await;
    let ids: Vec<&str> = docs.memberships.iter().map(|m| m.doc_id.as_str()).collect();
    assert_eq!(ids, vec![acc.primary_doc_id.as_str()]);
    // Every row under it is gone.
    let counts: (i64, i64, i64) = acc
        .server
        .state
        .db
        .call(move |c| {
            let d = did.as_bytes().to_vec();
            let docs: i64 =
                c.query_row("SELECT COUNT(*) FROM docs WHERE id = ?", [&d], |r| r.get(0))?;
            let members: i64 = c.query_row(
                "SELECT COUNT(*) FROM doc_members WHERE doc_id = ?",
                [&d],
                |r| r.get(0),
            )?;
            let seqs: i64 = c.query_row(
                "SELECT COUNT(*) FROM doc_sequences WHERE doc_id = ?",
                [&d],
                |r| r.get(0),
            )?;
            Ok((docs, members, seqs))
        })
        .await
        .unwrap();
    assert_eq!(counts, (0, 0, 0));
    // Deleting again: nothing to own.
    assert_eq!(
        delete_status(&format!("{}/{}", docs_url, doc_id), &acc.device_token).await,
        reqwest::StatusCode::NOT_FOUND
    );
    // The primary still opens for the account.
    assert_eq!(
        login_with(&acc, "correct horse battery staple")
            .await
            .memberships
            .len(),
        1
    );
}

/// Deletion is unilateral (sharing-plan.md §Deletion): the owner deletes
/// a doc with a second active member and an open invite, and the member
/// simply stops seeing it. A plain member cannot delete (404, learns
/// nothing). The invite row must go with the doc or the FK refuses the
/// whole delete.
#[tokio::test]
async fn owner_deletes_a_doc_out_from_under_its_other_members() {
    let acc = signup(false).await;
    let other = signup_on(&acc.server.base).await;
    let (doc_id, dek) = create_doc(&acc, &acc.dek).await;
    let did = Uuid::parse_str(&doc_id).unwrap();
    let other_id = Uuid::parse_str(&other.account_id).unwrap();
    let w = other.dek.wrap_dek(&dek).unwrap();
    acc.server
        .state
        .db
        .call(move |c| {
            c.execute(
                "INSERT INTO doc_members (
                    doc_id, account_id, role, wrapped_dek, wrapped_dek_nonce, added_at
                 ) VALUES (?, ?, 'member', ?, ?, 2)",
                params![
                    did.as_bytes().as_slice(),
                    other_id.as_bytes().as_slice(),
                    w.ciphertext,
                    w.nonce.to_vec(),
                ],
            )?;
            Ok(())
        })
        .await
        .unwrap();
    let url = format!("{}/api/docs/{}", acc.server.base, doc_id);
    // A plain member is not the owner.
    assert_eq!(
        delete_status(&url, &other.device_token).await,
        reqwest::StatusCode::NOT_FOUND
    );
    let member_docs: DocsListResponse = get_msgpack_authed(
        &format!("{}/api/docs", acc.server.base),
        &other.device_token,
    )
    .await;
    assert!(member_docs.memberships.iter().any(|m| m.doc_id == doc_id));

    issue_invite(&acc, &doc_id, &dek, None).await;
    assert!(delete_status(&url, &acc.device_token).await.is_success());

    let member_docs: DocsListResponse = get_msgpack_authed(
        &format!("{}/api/docs", acc.server.base),
        &other.device_token,
    )
    .await;
    assert!(
        !member_docs.memberships.iter().any(|m| m.doc_id == doc_id),
        "the member no longer sees the deleted doc"
    );
    assert_eq!(
        delete_status(&url, &acc.device_token).await,
        reqwest::StatusCode::NOT_FOUND,
        "gone for the owner too"
    );
}

/// The point of the Home-DEK-as-account-key model: a doc created on an
/// enrolled account survives a password change *and* a recovery reset
/// without anyone re-wrapping it, because its only wrap is under the
/// Home DEK, which neither path alters.
#[tokio::test]
async fn created_doc_survives_password_change_and_recovery_with_no_per_doc_rewrap() {
    let acc = signup(true).await;
    let password = "correct horse battery staple";
    let master =
        derive_password_master(password.as_bytes(), &acc.master_salt, acc.kdf_params).unwrap();
    let kek = master.kek().unwrap();
    let (doc_id, dek) = create_doc(&acc, &acc.dek).await;
    assert_ne!(doc_id, acc.primary_doc_id);

    // Listed with the primary first, owner role, Home DEK opens the wrap.
    let docs: DocsListResponse =
        get_msgpack_authed(&format!("{}/api/docs", acc.server.base), &acc.device_token).await;
    assert_eq!(docs.primary_doc_id, acc.primary_doc_id);
    let ids: Vec<&str> = docs.memberships.iter().map(|m| m.doc_id.as_str()).collect();
    assert_eq!(ids, vec![acc.primary_doc_id.as_str(), doc_id.as_str()]);
    assert_eq!(docs.memberships[1].role, MemberRole::Owner);
    let listed = &docs.memberships[1];
    let reopened = unwrap_with_home(&acc.dek, &listed.wrapped_dek, &listed.wrapped_dek_nonce);
    assert_eq!(reopened.as_bytes(), dek.as_bytes());
    // The KEK does not open it: it is not a KEK wrap.
    assert!(
        kek.unwrap(&monoplan_core::WrappedDek {
            ciphertext: listed.wrapped_dek.clone(),
            nonce: listed.wrapped_dek_nonce.as_slice().try_into().unwrap(),
        })
        .is_err()
    );

    // Login reports it too.
    assert_eq!(login_with(&acc, password).await.memberships.len(), 2);

    // --- Password change: primary only, created doc untouched. ---
    let new_password = "another long password";
    let new_salt: [u8; 16] = random_bytes();
    let new_master =
        derive_password_master(new_password.as_bytes(), &new_salt, acc.kdf_params).unwrap();
    let new_kek = new_master.kek().unwrap();
    let url = format!("{}/api/account/password/change", acc.server.base);
    let req = |keys: Vec<DocKey>| PasswordChangeRequest {
        current_auth_secret: master.auth_secret().unwrap().as_bytes().to_vec(),
        new_master_salt: new_salt.to_vec(),
        new_kdf_params: acc.kdf_params,
        new_auth_secret: new_master.auth_secret().unwrap().as_bytes().to_vec(),
        new_doc_keys: keys,
    };
    // Naming the created doc is a mismatch, not a courtesy.
    let status = post_msgpack_status_authed(
        &url,
        &req(vec![
            wrap_doc_key(&new_kek, &acc.primary_doc_id, &acc.dek),
            wrap_doc_key(&new_kek, &doc_id, &dek),
        ]),
        &acc.device_token,
    )
    .await;
    assert_eq!(status, reqwest::StatusCode::BAD_REQUEST);
    post_msgpack_no_response(
        &url,
        &req(vec![wrap_doc_key(&new_kek, &acc.primary_doc_id, &acc.dek)]),
        Some(&acc.device_token),
    )
    .await;
    let login = login_with(&acc, new_password).await;
    let home = {
        let m = login
            .memberships
            .iter()
            .find(|m| m.doc_id == acc.primary_doc_id)
            .unwrap();
        unwrap_doc_key(
            &new_kek,
            &DocKey {
                doc_id: m.doc_id.clone(),
                wrapped_dek: m.wrapped_dek.clone(),
                wrapped_dek_nonce: m.wrapped_dek_nonce.clone(),
            },
        )
    };
    assert_eq!(home.as_bytes(), acc.dek.as_bytes());
    let created = login
        .memberships
        .iter()
        .find(|m| m.doc_id == doc_id)
        .expect("created doc survives the password change");
    assert_eq!(
        (
            created.wrapped_dek.as_slice(),
            created.wrapped_dek_nonce.as_slice()
        ),
        (
            listed.wrapped_dek.as_slice(),
            listed.wrapped_dek_nonce.as_slice()
        ),
        "the created doc's wrap must be byte-identical: nothing re-wrapped it"
    );
    assert_eq!(
        unwrap_with_home(&home, &created.wrapped_dek, &created.wrapped_dek_nonce).as_bytes(),
        dek.as_bytes()
    );

    // --- Recovery: one wrap out, one wrap back, created doc untouched. ---
    let recovery_code = acc.recovery_code.clone().expect("opted in");
    let recovery_salt = acc.recovery_salt.clone().expect("opted in");
    let r_master = derive_recovery_master(&recovery_code, &recovery_salt, acc.kdf_params).unwrap();
    let r_kek = r_master.kek().unwrap();
    let recovered: RecoverResponse = post_msgpack(
        &format!("{}/api/account/recover", acc.server.base),
        &RecoverRequest {
            email: acc.email.clone(),
            recovery_auth_secret: r_master.auth_secret().unwrap().as_bytes().to_vec(),
        },
        None,
    )
    .await;
    assert_eq!(
        recovered.doc_keys.len(),
        1,
        "recovery releases the primary's wrap only, however many docs exist"
    );
    assert_eq!(recovered.doc_keys[0].doc_id, acc.primary_doc_id);
    let recovered_home = unwrap_doc_key(&r_kek, &recovered.doc_keys[0]);
    assert_eq!(recovered_home.as_bytes(), acc.dek.as_bytes());

    let reset_password = "third password of the day";
    let reset_salt: [u8; 16] = random_bytes();
    let reset_master =
        derive_password_master(reset_password.as_bytes(), &reset_salt, acc.kdf_params).unwrap();
    let reset_kek = reset_master.kek().unwrap();
    let reset_req = |keys: Vec<DocKey>| PasswordResetRequest {
        recovery_session_token: recovered.recovery_session_token.clone(),
        new_master_salt: reset_salt.to_vec(),
        new_kdf_params: acc.kdf_params,
        new_auth_secret: reset_master.auth_secret().unwrap().as_bytes().to_vec(),
        new_doc_keys: keys,
        device_name: "post-recover".into(),
        device_kind: "linux".into(),
    };
    // Including the created doc is a mismatch and does not burn the session.
    let status = post_msgpack_status(
        &format!("{}/api/account/password/reset", acc.server.base),
        &reset_req(vec![
            wrap_doc_key(&reset_kek, &acc.primary_doc_id, &recovered_home),
            wrap_doc_key(&reset_kek, &doc_id, &dek),
        ]),
    )
    .await;
    assert_eq!(status, reqwest::StatusCode::BAD_REQUEST);
    let reset: monoplan_protocol::PasswordResetResponse = post_msgpack(
        &format!("{}/api/account/password/reset", acc.server.base),
        &reset_req(vec![wrap_doc_key(
            &reset_kek,
            &acc.primary_doc_id,
            &recovered_home,
        )]),
        None,
    )
    .await;
    assert_eq!(reset.primary_doc_id, acc.primary_doc_id);

    // The recovering device learns about the other doc from the list
    // endpoint and opens it with the Home DEK it just recovered.
    let docs: DocsListResponse = get_msgpack_authed(
        &format!("{}/api/docs", acc.server.base),
        &reset.device_token,
    )
    .await;
    assert_eq!(
        docs.memberships.len(),
        2,
        "no membership was closed by the reset"
    );
    let created = docs
        .memberships
        .iter()
        .find(|m| m.doc_id == doc_id)
        .expect("created doc survives recovery");
    assert_eq!(
        (
            created.wrapped_dek.as_slice(),
            created.wrapped_dek_nonce.as_slice()
        ),
        (
            listed.wrapped_dek.as_slice(),
            listed.wrapped_dek_nonce.as_slice()
        ),
    );
    assert_eq!(
        unwrap_with_home(
            &recovered_home,
            &created.wrapped_dek,
            &created.wrapped_dek_nonce
        )
        .as_bytes(),
        dek.as_bytes()
    );
    // And the new password opens the Home DEK.
    let login = login_with(&acc, reset_password).await;
    let m = &login.memberships[0];
    assert_eq!(m.doc_id, acc.primary_doc_id);
    assert_eq!(
        unwrap_doc_key(
            &reset_kek,
            &DocKey {
                doc_id: m.doc_id.clone(),
                wrapped_dek: m.wrapped_dek.clone(),
                wrapped_dek_nonce: m.wrapped_dek_nonce.clone(),
            },
        )
        .as_bytes(),
        acc.dek.as_bytes()
    );
}

async fn post_msgpack_status_authed<Req: Serialize>(
    url: &str,
    body: &Req,
    token: &str,
) -> reqwest::StatusCode {
    let bytes = rmp_serde::to_vec_named(body).unwrap();
    http()
        .post(url)
        .header(CONTENT_TYPE, MSGPACK)
        .bearer_auth(token)
        .body(bytes)
        .send()
        .await
        .unwrap()
        .status()
}

#[tokio::test]
async fn signup_sets_device_cookie_with_expected_attributes() {
    let acc = signup(false).await;
    // We can't see the Set-Cookie from the existing helper-based signup
    // path, so re-issue a signup-like call directly to inspect headers.
    let server = TestServer::start().await;
    let email = format!("user-{}@example.com", uuid::Uuid::now_v7());
    let password = b"correct horse battery staple";
    let kdf_params = weak_params();
    let salt: [u8; 16] = random_bytes();
    let master = derive_password_master(password, &salt, kdf_params).unwrap();
    let kek = master.kek().unwrap();
    let auth = master.auth_secret().unwrap();
    let dek = Dek::generate();
    let wrapped = kek.wrap(&dek).unwrap();

    let (status, set_cookies, body): (_, _, Option<SignupResponse>) = post_msgpack_full(
        &format!("{}/api/account/signup", server.base),
        &SignupRequest {
            email: email.clone(),
            master_salt: salt.to_vec(),
            kdf_params,
            auth_secret: auth.as_bytes().to_vec(),
            wrapped_dek: wrapped.ciphertext,
            wrapped_dek_nonce: wrapped.nonce.to_vec(),
            recovery: None,
            device_name: "test".into(),
            device_kind: "linux".into(),
        },
        None,
        None,
    )
    .await;
    assert!(status.is_success());
    let body = body.unwrap();
    let sc = find_set_cookie(&set_cookies, "monoplan_device")
        .expect("expected monoplan_device Set-Cookie");
    assert_eq!(
        cookie_value(sc, "monoplan_device"),
        Some(body.device_token.as_str())
    );
    assert!(sc.contains("HttpOnly"), "missing HttpOnly: {sc}");
    assert!(sc.contains("Secure"), "missing Secure: {sc}");
    assert!(
        sc.contains("SameSite=Strict"),
        "missing SameSite=Strict: {sc}"
    );
    assert!(sc.contains("Path=/"), "missing Path=/: {sc}");
    // Persistent cookie, not a session cookie: mobile Safari/WebKit drops
    // session cookies on backgrounded-tab memory reclaim (see cookie.rs).
    assert!(
        sc.contains("Max-Age=34560000"),
        "expected 400-day Max-Age: {sc}"
    );
    let _ = acc; // silence unused warning; kept to mirror style of other tests
}

#[tokio::test]
async fn cookie_authenticates_logout_without_bearer() {
    let acc = signup(false).await;
    // Logout via cookie only (no Authorization header).
    let cookie_header = format!("monoplan_device={}", acc.device_token);
    let (status, set_cookies, _): (_, _, Option<()>) = post_msgpack_full(
        &format!("{}/api/account/logout", acc.server.base),
        &serde_bytes::Bytes::new(b""), // empty body; route ignores it
        None,
        Some(&cookie_header),
    )
    .await;
    // Note: ApiResult<(HeaderMap, ())> serialises to an empty body with
    // 200; the helper returns parsed = None for empty bodies.
    assert!(
        status.is_success(),
        "logout via cookie should succeed: {status}"
    );
    let cleared = find_set_cookie(&set_cookies, "monoplan_device")
        .expect("logout should emit a clearing Set-Cookie");
    assert!(
        cleared.contains("Max-Age=0"),
        "expected clear cookie: {cleared}"
    );
    assert_eq!(cookie_value(cleared, "monoplan_device"), Some(""));

    // Subsequent bearer call should now 401 — the device was revoked.
    let status = post_msgpack_status(
        &format!("{}/api/account/logout", acc.server.base),
        &serde_bytes::Bytes::new(b""),
    )
    .await;
    assert_eq!(status, reqwest::StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn login_without_register_device_does_not_set_cookie() {
    let acc = signup(false).await;
    let pre: PreloginResponse = post_msgpack(
        &format!("{}/api/account/prelogin", acc.server.base),
        &PreloginRequest {
            email: acc.email.clone(),
        },
        None,
    )
    .await;
    let master = derive_password_master(
        b"correct horse battery staple",
        &pre.master_salt,
        pre.kdf_params,
    )
    .unwrap();
    let auth = master.auth_secret().unwrap();
    let (status, set_cookies, _): (_, _, Option<LoginResponse>) = post_msgpack_full(
        &format!("{}/api/account/login", acc.server.base),
        &LoginRequest {
            email: acc.email,
            auth_secret: auth.as_bytes().to_vec(),
            register_device: None,
        },
        None,
        None,
    )
    .await;
    assert!(status.is_success());
    assert!(
        find_set_cookie(&set_cookies, "monoplan_device").is_none(),
        "no cookie should be set when no device is minted: {set_cookies:?}"
    );
}

#[tokio::test]
async fn recover_without_enrollment_is_forbidden() {
    let acc = signup(false).await;
    let status = post_msgpack_status(
        &format!("{}/api/account/recover", acc.server.base),
        &RecoverRequest {
            email: acc.email,
            recovery_auth_secret: vec![0u8; 32],
        },
    )
    .await;
    assert_eq!(status, reqwest::StatusCode::FORBIDDEN);
}

// ---------- sharing links ----------

async fn get_status(url: &str, token: Option<&str>) -> reqwest::StatusCode {
    let mut req = http().get(url);
    if let Some(t) = token {
        req = req.bearer_auth(t);
    }
    req.send().await.unwrap().status()
}

/// Issue a link for `doc_id` as `acc`: a fresh secret, the doc's DEK
/// wrapped under the invite KEK, the HKDF id posted to the server.
async fn issue_invite(
    acc: &Account,
    doc_id: &str,
    doc_dek: &Dek,
    expires_in_secs: Option<u64>,
) -> (InviteSecret, CreateInviteResponse) {
    let secret = InviteSecret::generate();
    let w = secret.wrap(doc_dek).unwrap();
    let resp: CreateInviteResponse = post_msgpack(
        &format!("{}/api/docs/{}/invites", acc.server.base, doc_id),
        &CreateInviteRequest {
            invite_id: hex::encode(secret.id().unwrap()),
            wrapped_dek: w.ciphertext,
            wrapped_dek_nonce: w.nonce.to_vec(),
            expires_in_secs,
        },
        Some(&acc.device_token),
    )
    .await;
    (secret, resp)
}

/// The invitee side: fetch the payload, open it with the secret, re-wrap
/// under `home` and accept. Returns the doc id the server joined us to
/// and the DEK we recovered from the link.
async fn accept_invite(
    base: &str,
    secret: &InviteSecret,
    token: &str,
    home: &Dek,
) -> (String, Dek) {
    let id = hex::encode(secret.id().unwrap());
    let payload: InvitePayloadResponse = post_free_get(&format!("{base}/api/invites/{id}")).await;
    let dek = secret
        .unwrap(&monoplan_core::WrappedDek {
            ciphertext: payload.wrapped_dek.clone(),
            nonce: payload.wrapped_dek_nonce.as_slice().try_into().unwrap(),
        })
        .unwrap();
    let w = home.wrap_dek(&dek).unwrap();
    let resp: AcceptInviteResponse = post_msgpack(
        &format!("{base}/api/invites/{id}/accept"),
        &AcceptInviteRequest {
            wrapped_dek: w.ciphertext,
            wrapped_dek_nonce: w.nonce.to_vec(),
        },
        Some(token),
    )
    .await;
    assert_eq!(resp.doc_id, payload.doc_id);
    (resp.doc_id, dek)
}

/// Unauthenticated GET of a msgpack body.
async fn post_free_get<Resp: DeserializeOwned>(url: &str) -> Resp {
    let resp = http().get(url).send().await.unwrap();
    assert!(
        resp.status().is_success(),
        "expected 2xx, got {}: {}",
        resp.status(),
        resp.text().await.unwrap_or_default()
    );
    rmp_serde::from_slice(&resp.bytes().await.unwrap()).unwrap()
}

/// The whole link lifecycle: the owner issues a link for a doc they
/// created, a second account fetches it anonymously, opens the wrap with
/// the fragment secret alone, joins with a Home-DEK wrap, and from then
/// on sees the doc in `GET /api/docs` with a wrap its own Home DEK
/// opens to the very same DEK. The link is spent by the accept (410 on
/// both fetch and a second accept), the inviter's list shows it
/// consumed, and both members appear in the member list.
#[tokio::test]
async fn invite_link_joins_a_second_account_once() {
    let acc = signup(false).await;
    let base = acc.server.base.clone();
    let (doc_id, doc_dek) = create_doc(&acc, &acc.dek).await;
    let (secret, created) = issue_invite(&acc, &doc_id, &doc_dek, None).await;
    let id = hex::encode(secret.id().unwrap());
    assert_eq!(created.invite_id, id);

    // The anonymous fetch names the doc and masks the inviter.
    let payload: InvitePayloadResponse = post_free_get(&format!("{base}/api/invites/{id}")).await;
    assert_eq!(payload.doc_id, doc_id);
    assert_eq!(payload.expires_at, created.expires_at);
    assert!(
        payload.inviter_hint.starts_with("u…@"),
        "{}",
        payload.inviter_hint
    );
    // The server-held wrap is useless without the secret.
    assert!(
        InviteSecret::generate()
            .unwrap(&monoplan_core::WrappedDek {
                ciphertext: payload.wrapped_dek.clone(),
                nonce: payload.wrapped_dek_nonce.as_slice().try_into().unwrap(),
            })
            .is_err()
    );

    let second = signup_on(&base).await;
    let (joined, recovered) =
        accept_invite(&base, &secret, &second.device_token, &second.dek).await;
    assert_eq!(joined, doc_id);
    assert_eq!(recovered.as_bytes(), doc_dek.as_bytes());

    // Spent: fetch and a second accept both read as gone.
    assert_eq!(
        get_status(&format!("{base}/api/invites/{id}"), None).await,
        reqwest::StatusCode::GONE
    );
    let third = signup_on(&base).await;
    let w = third.dek.wrap_dek(&doc_dek).unwrap();
    assert_eq!(
        post_msgpack_status_authed(
            &format!("{base}/api/invites/{id}/accept"),
            &AcceptInviteRequest {
                wrapped_dek: w.ciphertext,
                wrapped_dek_nonce: w.nonce.to_vec(),
            },
            &third.device_token,
        )
        .await,
        reqwest::StatusCode::GONE
    );

    // The new member's doc list carries the membership with a wrap its
    // Home DEK opens to the same DEK; a later login would do the same.
    let docs: DocsListResponse =
        get_msgpack_authed(&format!("{base}/api/docs"), &second.device_token).await;
    let m = docs
        .memberships
        .iter()
        .find(|m| m.doc_id == doc_id)
        .expect("joined doc listed");
    assert_eq!(m.role, MemberRole::Member);
    assert_eq!(
        unwrap_with_home(&second.dek, &m.wrapped_dek, &m.wrapped_dek_nonce).as_bytes(),
        doc_dek.as_bytes()
    );

    // The inviter's view: the link is listed as consumed; both members show.
    let invites: InvitesListResponse = get_msgpack_authed(
        &format!("{base}/api/docs/{doc_id}/invites"),
        &acc.device_token,
    )
    .await;
    assert_eq!(invites.invites.len(), 1);
    assert_eq!(invites.invites[0].invite_id, id);
    assert!(invites.invites[0].consumed_at.is_some());
    let members: MembersListResponse = get_msgpack_authed(
        &format!("{base}/api/docs/{doc_id}/members"),
        &second.device_token,
    )
    .await;
    let mut roles: Vec<(String, &str)> = members
        .members
        .iter()
        .map(|m| (m.account_id.clone(), m.role.as_str()))
        .collect();
    roles.sort();
    let mut expected = vec![
        (acc.account_id.clone(), MemberRole::Owner.as_str()),
        (second.account_id.clone(), MemberRole::Member.as_str()),
    ];
    expected.sort();
    assert_eq!(roles, expected);
    assert!(members.members.iter().any(|m| m.email == acc.email));

    // Membership is what `OpenDoc` checks; a non-member cannot list members.
    assert_eq!(
        get_status(
            &format!("{base}/api/docs/{doc_id}/members"),
            Some(&third.device_token)
        )
        .await,
        reqwest::StatusCode::NOT_FOUND
    );
}

/// The three refusals around issuing: the primary doc is never shareable
/// (403: the Home DEK is the account key), a non-owner cannot issue or
/// list (404, learning nothing), and a revoked link is gone for good
/// (404 on fetch, nothing to accept).
#[tokio::test]
async fn invite_issuance_is_owner_only_never_for_home_and_revocable() {
    let acc = signup(false).await;
    let base = acc.server.base.clone();
    let secret = InviteSecret::generate();
    let w = secret.wrap(&acc.dek).unwrap();
    assert_eq!(
        post_msgpack_status_authed(
            &format!("{base}/api/docs/{}/invites", acc.primary_doc_id),
            &CreateInviteRequest {
                invite_id: hex::encode(secret.id().unwrap()),
                wrapped_dek: w.ciphertext,
                wrapped_dek_nonce: w.nonce.to_vec(),
                expires_in_secs: None,
            },
            &acc.device_token,
        )
        .await,
        reqwest::StatusCode::FORBIDDEN
    );

    let (doc_id, doc_dek) = create_doc(&acc, &acc.dek).await;
    let stranger = signup_on(&base).await;
    let w = secret.wrap(&doc_dek).unwrap();
    assert_eq!(
        post_msgpack_status_authed(
            &format!("{base}/api/docs/{doc_id}/invites"),
            &CreateInviteRequest {
                invite_id: hex::encode(secret.id().unwrap()),
                wrapped_dek: w.ciphertext,
                wrapped_dek_nonce: w.nonce.to_vec(),
                expires_in_secs: None,
            },
            &stranger.device_token,
        )
        .await,
        reqwest::StatusCode::NOT_FOUND
    );
    assert_eq!(
        get_status(
            &format!("{base}/api/docs/{doc_id}/invites"),
            Some(&stranger.device_token)
        )
        .await,
        reqwest::StatusCode::NOT_FOUND
    );

    // A member who is not the owner cannot issue either.
    let (member_secret, _) = issue_invite(&acc, &doc_id, &doc_dek, None).await;
    let member = signup_on(&base).await;
    accept_invite(&base, &member_secret, &member.device_token, &member.dek).await;
    let s2 = InviteSecret::generate();
    let w2 = s2.wrap(&doc_dek).unwrap();
    assert_eq!(
        post_msgpack_status_authed(
            &format!("{base}/api/docs/{doc_id}/invites"),
            &CreateInviteRequest {
                invite_id: hex::encode(s2.id().unwrap()),
                wrapped_dek: w2.ciphertext,
                wrapped_dek_nonce: w2.nonce.to_vec(),
                expires_in_secs: None,
            },
            &member.device_token,
        )
        .await,
        reqwest::StatusCode::NOT_FOUND
    );

    // Revoke: the owner issues, deletes, and the link reads as absent.
    let (secret, _) = issue_invite(&acc, &doc_id, &doc_dek, None).await;
    let id = hex::encode(secret.id().unwrap());
    assert_eq!(
        delete_status(
            &format!("{base}/api/docs/{doc_id}/invites/{id}"),
            &stranger.device_token
        )
        .await,
        reqwest::StatusCode::NOT_FOUND
    );
    assert_eq!(
        delete_status(
            &format!("{base}/api/docs/{doc_id}/invites/{id}"),
            &acc.device_token
        )
        .await,
        reqwest::StatusCode::OK
    );
    assert_eq!(
        get_status(&format!("{base}/api/invites/{id}"), None).await,
        reqwest::StatusCode::NOT_FOUND
    );
    let w = stranger.dek.wrap_dek(&doc_dek).unwrap();
    assert_eq!(
        post_msgpack_status_authed(
            &format!("{base}/api/invites/{id}/accept"),
            &AcceptInviteRequest {
                wrapped_dek: w.ciphertext,
                wrapped_dek_nonce: w.nonce.to_vec(),
            },
            &stranger.device_token,
        )
        .await,
        reqwest::StatusCode::NOT_FOUND
    );
}

/// Expiry and the inviter's own link: a link whose `expires_at` has
/// passed is gone (410) on fetch and accept, and the owner opening their
/// own link is told they are already a member (409) without spending it.
#[tokio::test]
async fn invite_expires_and_is_not_spent_by_an_existing_member() {
    let acc = signup(false).await;
    let base = acc.server.base.clone();
    let (doc_id, doc_dek) = create_doc(&acc, &acc.dek).await;

    // Expiry is clamped to >= 60 s, so age the row directly.
    let (secret, created) = issue_invite(&acc, &doc_id, &doc_dek, Some(1)).await;
    assert!(created.expires_at > 0);
    let id = hex::encode(secret.id().unwrap());
    let id_bytes = secret.id().unwrap().to_vec();
    acc.server
        .state
        .db
        .call(move |c| {
            c.execute(
                "UPDATE doc_invites SET expires_at = 1 WHERE id = ?",
                [&id_bytes],
            )
        })
        .await
        .unwrap();
    assert_eq!(
        get_status(&format!("{base}/api/invites/{id}"), None).await,
        reqwest::StatusCode::GONE
    );
    let second = signup_on(&base).await;
    let w = second.dek.wrap_dek(&doc_dek).unwrap();
    assert_eq!(
        post_msgpack_status_authed(
            &format!("{base}/api/invites/{id}/accept"),
            &AcceptInviteRequest {
                wrapped_dek: w.ciphertext,
                wrapped_dek_nonce: w.nonce.to_vec(),
            },
            &second.device_token,
        )
        .await,
        reqwest::StatusCode::GONE
    );

    // A fresh link, accepted by its own issuer: 409 and still open.
    let (secret, _) = issue_invite(&acc, &doc_id, &doc_dek, None).await;
    let id = hex::encode(secret.id().unwrap());
    let w = acc.dek.wrap_dek(&doc_dek).unwrap();
    assert_eq!(
        post_msgpack_status_authed(
            &format!("{base}/api/invites/{id}/accept"),
            &AcceptInviteRequest {
                wrapped_dek: w.ciphertext,
                wrapped_dek_nonce: w.nonce.to_vec(),
            },
            &acc.device_token,
        )
        .await,
        reqwest::StatusCode::CONFLICT
    );
    let (joined, _) = accept_invite(&base, &secret, &second.device_token, &second.dek).await;
    assert_eq!(joined, doc_id);
}

/// `DELETE /api/docs/:doc_id/members/:account_id`: the refusal matrix
/// around removal, then one real removal. A non-member learns nothing
/// (404), a non-owner member is told no (403), the owner cannot remove
/// themself (409: leaving is a different verb) nor the last owner
/// (409, by way of the same rule), and the primary doc is refused
/// before any lookup (403). After the owner removes the invitee, the
/// member list and the invitee's own doc list drop the doc, a second
/// removal reads as 404, and a fresh link lets the invitee back in on
/// the same row.
#[tokio::test]
async fn member_removal_is_owner_only_never_self_nor_last_owner() {
    let acc = signup(false).await;
    let base = acc.server.base.clone();
    let (doc_id, doc_dek) = create_doc(&acc, &acc.dek).await;
    let (secret, _) = issue_invite(&acc, &doc_id, &doc_dek, None).await;
    let second = signup_on(&base).await;
    accept_invite(&base, &secret, &second.device_token, &second.dek).await;
    let third = signup_on(&base).await;
    let member_url = |doc: &str, account: &str| format!("{base}/api/docs/{doc}/members/{account}");

    // The primary doc is refused before any membership lookup.
    assert_eq!(
        delete_status(
            &member_url(&acc.primary_doc_id, &second.account_id),
            &acc.device_token
        )
        .await,
        reqwest::StatusCode::FORBIDDEN
    );
    // A non-member learns nothing.
    assert_eq!(
        delete_status(
            &member_url(&doc_id, &second.account_id),
            &third.device_token
        )
        .await,
        reqwest::StatusCode::NOT_FOUND
    );
    // A member who is not an owner is told no.
    assert_eq!(
        delete_status(&member_url(&doc_id, &acc.account_id), &second.device_token).await,
        reqwest::StatusCode::FORBIDDEN
    );
    // The owner cannot remove themself: that would be "leave", and
    // with one owner it would also orphan the doc.
    assert_eq!(
        delete_status(&member_url(&doc_id, &acc.account_id), &acc.device_token).await,
        reqwest::StatusCode::CONFLICT
    );
    // A garbage account id is a bad request, not a lookup.
    assert_eq!(
        delete_status(&member_url(&doc_id, "not-a-uuid"), &acc.device_token).await,
        reqwest::StatusCode::BAD_REQUEST
    );
    // Someone who was never a member is not found.
    assert_eq!(
        delete_status(&member_url(&doc_id, &third.account_id), &acc.device_token).await,
        reqwest::StatusCode::NOT_FOUND
    );

    // The real thing.
    assert!(
        delete_status(&member_url(&doc_id, &second.account_id), &acc.device_token)
            .await
            .is_success()
    );
    let members: MembersListResponse = get_msgpack_authed(
        &format!("{base}/api/docs/{doc_id}/members"),
        &acc.device_token,
    )
    .await;
    assert_eq!(
        members
            .members
            .iter()
            .map(|m| m.account_id.as_str())
            .collect::<Vec<_>>(),
        vec![acc.account_id.as_str()]
    );
    let docs: DocsListResponse =
        get_msgpack_authed(&format!("{base}/api/docs"), &second.device_token).await;
    assert!(
        docs.memberships.iter().all(|m| m.doc_id != doc_id),
        "removed member still lists the doc"
    );
    // Removed: the member list is closed to them, and removing again
    // finds nothing.
    assert_eq!(
        get_status(
            &format!("{base}/api/docs/{doc_id}/members"),
            Some(&second.device_token)
        )
        .await,
        reqwest::StatusCode::NOT_FOUND
    );
    assert_eq!(
        delete_status(&member_url(&doc_id, &second.account_id), &acc.device_token).await,
        reqwest::StatusCode::NOT_FOUND
    );

    // A fresh link reactivates the same row.
    let (again, _) = issue_invite(&acc, &doc_id, &doc_dek, None).await;
    let (rejoined, _) = accept_invite(&base, &again, &second.device_token, &second.dek).await;
    assert_eq!(rejoined, doc_id);
    let members: MembersListResponse = get_msgpack_authed(
        &format!("{base}/api/docs/{doc_id}/members"),
        &second.device_token,
    )
    .await;
    assert_eq!(members.members.len(), 2);
}
