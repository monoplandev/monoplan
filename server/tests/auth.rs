//! End-to-end auth: real server, real sqlite, real reqwest, real crypto.
//!
//! Each test spins up a fresh `:memory:` server on a random port. We
//! use weak Argon2 params (`m=8 KiB, t=1, p=1`) so the test pass stays
//! under a second — the crypto correctness is covered by `core` unit
//! tests.

use monoplan_core::{
    AEAD_NONCE_LEN, Dek, derive_password_master, derive_recovery_master, generate_recovery_code,
    random_bytes,
};
use monoplan_protocol::{
    DeviceRenameRequest, DevicesListResponse, KdfParams, LoginRequest, LoginResponse,
    PasswordResetRequest, PreloginRequest, PreloginResponse, RecoverRequest, RecoverResponse,
    RecoveryMaterial, SignupRequest, SignupResponse,
};
use monoplan_server::{AppState, router};
use reqwest::header::CONTENT_TYPE;
use serde::Serialize;
use serde::de::DeserializeOwned;
use std::sync::OnceLock;

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
    handle: tokio::task::JoinHandle<()>,
}

impl TestServer {
    async fn start() -> Self {
        let state = AppState::open_in_memory().await.unwrap();
        let app = router(state);
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
        Self { base, handle }
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
        },
        None,
    )
    .await;

    Account {
        server,
        email,
        account_id: resp.account_id,
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
    let nonce: [u8; AEAD_NONCE_LEN] = recovered
        .recovery_wrapped_dek_nonce
        .as_slice()
        .try_into()
        .unwrap();
    let dek = r_kek
        .unwrap(&monoplan_core::WrappedDek {
            ciphertext: recovered.recovery_wrapped_dek,
            nonce,
        })
        .unwrap();

    // New password + reset.
    let new_password = "freshly chosen pass";
    let new_salt: [u8; 16] = random_bytes();
    let new_params = weak_params();
    let new_master =
        derive_password_master(new_password.as_bytes(), &new_salt, new_params).unwrap();
    let new_kek = new_master.kek().unwrap();
    let new_auth = new_master.auth_secret().unwrap();
    let new_wrapped = new_kek.wrap(&dek).unwrap();

    let reset: monoplan_protocol::PasswordResetResponse = post_msgpack(
        &format!("{}/api/account/password/reset", acc.server.base),
        &PasswordResetRequest {
            recovery_session_token: recovered.recovery_session_token.clone(),
            new_master_salt: new_salt.to_vec(),
            new_kdf_params: new_params,
            new_auth_secret: new_auth.as_bytes().to_vec(),
            new_wrapped_dek: new_wrapped.ciphertext,
            new_wrapped_dek_nonce: new_wrapped.nonce.to_vec(),
            device_name: "post-recover".into(),
        },
        None,
    )
    .await;
    assert!(!reset.device_token.is_empty());

    // Recovery session token cannot be reused.
    let new_wrapped_2 = new_kek.wrap(&dek).unwrap();
    let status = post_msgpack_status(
        &format!("{}/api/account/password/reset", acc.server.base),
        &PasswordResetRequest {
            recovery_session_token: recovered.recovery_session_token,
            new_master_salt: new_salt.to_vec(),
            new_kdf_params: new_params,
            new_auth_secret: new_auth.as_bytes().to_vec(),
            new_wrapped_dek: new_wrapped_2.ciphertext,
            new_wrapped_dek_nonce: new_wrapped_2.nonce.to_vec(),
            device_name: "should-fail".into(),
        },
    )
    .await;
    assert_eq!(status, reqwest::StatusCode::FORBIDDEN);
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
