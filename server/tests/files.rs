//! Attachment routes end to end (`spec/attachments.md` §Server): real
//! server, real sqlite, the directory blob store over a temp dir.
//!
//! Bodies are random bytes: the server stores them as opaque sealed
//! blobs and never opens one.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicI64, Ordering};

use monoplan_core::{
    Dek, derive_password_master, derive_recovery_master, generate_recovery_code, random_bytes,
};
use monoplan_protocol::{
    CopyFileRequest, CreateDocRequest, CreateDocResponse, DeleteAccountRequest, ErrorBody,
    FilesListResponse, KdfParams, RecoveryMaterial, SignupRequest, SignupResponse,
};
use monoplan_server::files::{
    BlobStore, DirStore, MAX_FILE_BODY_BYTES, TOMBSTONE_GRACE_MILLIS, purge_expired,
};
use monoplan_server::state::Clock;
use monoplan_server::{AppState, router};
use reqwest::StatusCode;
use reqwest::header::CONTENT_TYPE;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use uuid::Uuid;

const PASSWORD: &[u8] = b"correct horse battery staple";

fn weak_params() -> KdfParams {
    KdfParams {
        m_kib: 8,
        t: 1,
        p: 1,
    }
}

fn http() -> &'static reqwest::Client {
    static C: OnceLock<reqwest::Client> = OnceLock::new();
    C.get_or_init(reqwest::Client::new)
}

/// A clock the purge tests move by hand.
#[derive(Debug)]
struct TestClock(AtomicI64);

impl Clock for TestClock {
    fn now_millis(&self) -> i64 {
        self.0.load(Ordering::SeqCst)
    }
}

struct TestServer {
    base: String,
    addr: std::net::SocketAddr,
    state: AppState,
    clock: Arc<TestClock>,
    dir: tempfile::TempDir,
    handle: tokio::task::JoinHandle<()>,
}

impl TestServer {
    async fn start() -> Self {
        Self::start_with_limit(None).await
    }

    async fn start_with_limit(max_account_file_bytes: Option<u64>) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let clock = Arc::new(TestClock(AtomicI64::new(1_000_000)));
        let mut state = AppState::open_in_memory()
            .await
            .unwrap()
            .with_blob_store(BlobStore::Dir(DirStore::open(dir.path()).unwrap()));
        state.clock = clock.clone();
        if let Some(max) = max_account_file_bytes {
            state = state.with_max_account_file_bytes(max);
        }
        let app = router(state.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let handle = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        Self {
            base: format!("http://{addr}"),
            addr,
            state,
            clock,
            dir,
            handle,
        }
    }

    fn stored_path(&self, doc_id: Uuid, file_id: &str) -> PathBuf {
        self.dir.path().join(doc_id.to_string()).join(file_id)
    }

    fn file_url(&self, doc_id: Uuid, file_id: &str) -> String {
        format!("{}/api/docs/{doc_id}/files/{file_id}", self.base)
    }
}

impl Drop for TestServer {
    fn drop(&mut self) {
        self.handle.abort();
    }
}

struct Account {
    token: String,
    account_id: Uuid,
    home: Uuid,
    master_salt: Vec<u8>,
}

async fn signup(server: &TestServer) -> Account {
    let kdf_params = weak_params();
    let master_salt: [u8; 16] = random_bytes();
    let master = derive_password_master(PASSWORD, &master_salt, kdf_params).unwrap();
    let dek = Dek::generate();
    let wrapped = master.kek().unwrap().wrap(&dek).unwrap();

    let code = generate_recovery_code().unwrap();
    let recovery_salt: [u8; 16] = random_bytes();
    let r_master = derive_recovery_master(code.as_str(), &recovery_salt, kdf_params).unwrap();
    let r_wrapped = r_master.kek().unwrap().wrap(&dek).unwrap();

    let body = serde_json::to_vec(&SignupRequest {
        email: format!("user-{}@example.com", Uuid::now_v7()),
        master_salt: master_salt.to_vec(),
        kdf_params,
        auth_secret: master.auth_secret().unwrap().as_bytes().to_vec(),
        wrapped_dek: wrapped.ciphertext,
        wrapped_dek_nonce: wrapped.nonce.to_vec(),
        recovery: RecoveryMaterial {
            recovery_salt: recovery_salt.to_vec(),
            recovery_auth_secret: r_master.auth_secret().unwrap().as_bytes().to_vec(),
            recovery_wrapped_dek: r_wrapped.ciphertext,
            recovery_wrapped_dek_nonce: r_wrapped.nonce.to_vec(),
        },
        device_name: "files-test".into(),
        device_kind: "linux".into(),
        primary_doc_id: None,
    })
    .unwrap();
    let resp = http()
        .post(format!("{}/api/account/signup", server.base))
        .header(CONTENT_TYPE, "application/json")
        .body(body)
        .send()
        .await
        .unwrap();
    assert!(resp.status().is_success(), "signup: {}", resp.status());
    let resp: SignupResponse = resp.json_body().await;
    Account {
        token: resp.device_token,
        account_id: Uuid::parse_str(&resp.account_id).unwrap(),
        home: Uuid::parse_str(&resp.primary_doc_id).unwrap(),
        master_salt: master_salt.to_vec(),
    }
}

/// `reqwest` is built without its `json` feature here; decode by hand.
trait JsonBody {
    async fn json_body<T: serde::de::DeserializeOwned>(self) -> T;
}

impl JsonBody for reqwest::Response {
    async fn json_body<T: serde::de::DeserializeOwned>(self) -> T {
        serde_json::from_slice(&self.bytes().await.unwrap()).unwrap()
    }
}

async fn create_doc(server: &TestServer, acc: &Account) -> Uuid {
    let resp = http()
        .post(format!("{}/api/docs", server.base))
        .bearer_auth(&acc.token)
        .header(CONTENT_TYPE, "application/json")
        .body(
            serde_json::to_vec(&CreateDocRequest {
                wrapped_dek: vec![7u8; 48],
                wrapped_dek_nonce: vec![6u8; 24],
            })
            .unwrap(),
        )
        .send()
        .await
        .unwrap();
    assert!(resp.status().is_success());
    let resp: CreateDocResponse = resp.json_body().await;
    Uuid::parse_str(&resp.doc_id).unwrap()
}

/// An active `member` row, the shape an accepted invite leaves behind.
async fn seed_member(server: &TestServer, doc_id: Uuid, account_id: Uuid) {
    server
        .state
        .db
        .call(move |c| {
            c.execute(
                "INSERT INTO doc_members (
                    doc_id, account_id, role, wrapped_dek, wrapped_dek_nonce, added_at
                 ) VALUES (?, ?, 'member', ?, ?, 2)",
                rusqlite::params![
                    doc_id.as_bytes().as_slice(),
                    account_id.as_bytes().as_slice(),
                    vec![9u8; 48],
                    vec![8u8; 24],
                ],
            )?;
            Ok(())
        })
        .await
        .unwrap();
}

fn new_file_id() -> String {
    hex::encode(random_bytes::<16>())
}

fn body_of(len: usize, seed: u8) -> Vec<u8> {
    (0..len).map(|i| (i as u8).wrapping_add(seed)).collect()
}

async fn put(server: &TestServer, token: &str, doc: Uuid, id: &str, body: Vec<u8>) -> StatusCode {
    http()
        .put(server.file_url(doc, id))
        .bearer_auth(token)
        .body(body)
        .send()
        .await
        .unwrap()
        .status()
}

async fn get(server: &TestServer, token: &str, doc: Uuid, id: &str) -> (StatusCode, Vec<u8>) {
    let resp = http()
        .get(server.file_url(doc, id))
        .bearer_auth(token)
        .send()
        .await
        .unwrap();
    let status = resp.status();
    (status, resp.bytes().await.unwrap().to_vec())
}

async fn delete(server: &TestServer, token: &str, doc: Uuid, id: &str) -> StatusCode {
    http()
        .delete(server.file_url(doc, id))
        .bearer_auth(token)
        .send()
        .await
        .unwrap()
        .status()
}

async fn restore(server: &TestServer, token: &str, doc: Uuid, id: &str) -> StatusCode {
    http()
        .post(format!("{}/restore", server.file_url(doc, id)))
        .bearer_auth(token)
        .send()
        .await
        .unwrap()
        .status()
}

async fn copy(server: &TestServer, token: &str, doc: Uuid, id: &str, to: Uuid) -> StatusCode {
    http()
        .post(format!("{}/copy", server.file_url(doc, id)))
        .bearer_auth(token)
        .header(CONTENT_TYPE, "application/json")
        .body(
            serde_json::to_vec(&CopyFileRequest {
                to_doc_id: to.to_string(),
            })
            .unwrap(),
        )
        .send()
        .await
        .unwrap()
        .status()
}

async fn list(server: &TestServer, token: &str, doc: Uuid) -> FilesListResponse {
    let resp = http()
        .get(format!("{}/api/docs/{doc}/files", server.base))
        .bearer_auth(token)
        .send()
        .await
        .unwrap();
    assert!(resp.status().is_success(), "list: {}", resp.status());
    resp.json_body().await
}

/// Send request headers and nothing else over a bare socket, and return
/// the status line plus body. For the refusals the server must make
/// from the headers alone, which an HTTP client will not let us send
/// (a missing `Content-Length`, a declared size we never transmit).
async fn raw_put(server: &TestServer, path: &str, token: &str, extra_headers: &str) -> String {
    let mut sock = tokio::net::TcpStream::connect(server.addr).await.unwrap();
    let head = format!(
        "PUT {path} HTTP/1.1\r\nHost: {}\r\nAuthorization: Bearer {token}\r\nConnection: close\r\n{extra_headers}\r\n",
        server.addr
    );
    sock.write_all(head.as_bytes()).await.unwrap();
    let mut out = Vec::new();
    sock.read_to_end(&mut out).await.unwrap();
    String::from_utf8_lossy(&out).into_owned()
}

#[tokio::test]
async fn upload_list_download_round_trip() {
    let server = TestServer::start().await;
    let acc = signup(&server).await;
    let id = new_file_id();
    // Above axum's 2 MiB default body limit, which must not apply.
    let body = body_of(3 * 1024 * 1024, 1);

    assert_eq!(
        put(&server, &acc.token, acc.home, &id, body.clone()).await,
        StatusCode::OK
    );
    assert_eq!(
        std::fs::read(server.stored_path(acc.home, &id)).unwrap(),
        body,
        "bytes land at <dir>/<doc_id>/<file_id>"
    );

    let listing = list(&server, &acc.token, acc.home).await;
    assert_eq!(listing.doc_bytes, body.len() as u64);
    assert_eq!(listing.used_bytes, Some(body.len() as u64));
    assert_eq!(listing.limit_bytes, Some(1024 * 1024 * 1024));
    assert_eq!(listing.files.len(), 1);
    assert_eq!(listing.files[0].id, id);
    assert_eq!(listing.files[0].bytes, body.len() as u64);
    assert_eq!(listing.files[0].deleted_at, None);

    let (status, got) = get(&server, &acc.token, acc.home, &id).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(got, body);
}

/// A retry of an upload that already landed is 409 and leaves the
/// stored bytes alone, whatever the retry carried.
#[tokio::test]
async fn repeated_put_is_conflict_and_never_rewrites() {
    let server = TestServer::start().await;
    let acc = signup(&server).await;
    let id = new_file_id();
    let first = body_of(100, 1);
    assert_eq!(
        put(&server, &acc.token, acc.home, &id, first.clone()).await,
        StatusCode::OK
    );
    assert_eq!(
        put(&server, &acc.token, acc.home, &id, body_of(200, 9)).await,
        StatusCode::CONFLICT
    );
    assert_eq!(get(&server, &acc.token, acc.home, &id).await.1, first);
    assert_eq!(list(&server, &acc.token, acc.home).await.doc_bytes, 100);
}

#[tokio::test]
async fn every_route_is_404_to_a_non_member_and_401_unauthenticated() {
    let server = TestServer::start().await;
    let owner = signup(&server).await;
    let outsider = signup(&server).await;
    let id = new_file_id();
    assert_eq!(
        put(&server, &owner.token, owner.home, &id, body_of(64, 1)).await,
        StatusCode::OK
    );

    let t = &outsider.token;
    let doc = owner.home;
    assert_eq!(
        put(&server, t, doc, &new_file_id(), body_of(64, 2)).await,
        StatusCode::NOT_FOUND
    );
    assert_eq!(get(&server, t, doc, &id).await.0, StatusCode::NOT_FOUND);
    assert_eq!(delete(&server, t, doc, &id).await, StatusCode::NOT_FOUND);
    assert_eq!(restore(&server, t, doc, &id).await, StatusCode::NOT_FOUND);
    assert_eq!(
        copy(&server, t, doc, &id, outsider.home).await,
        StatusCode::NOT_FOUND
    );
    let listing = http()
        .get(format!("{}/api/docs/{doc}/files", server.base))
        .bearer_auth(t)
        .send()
        .await
        .unwrap();
    assert_eq!(listing.status(), StatusCode::NOT_FOUND);

    // The owner's file is untouched by any of that.
    assert_eq!(get(&server, &owner.token, doc, &id).await.0, StatusCode::OK);

    let anonymous = http().get(server.file_url(doc, &id)).send().await.unwrap();
    assert_eq!(anonymous.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn put_refuses_bad_sizes_and_ids_from_the_headers() {
    let server = TestServer::start().await;
    let acc = signup(&server).await;
    let path = format!("/api/docs/{}/files/{}", acc.home, new_file_id());

    let too_large = raw_put(
        &server,
        &path,
        &acc.token,
        &format!("Content-Length: {}\r\n", MAX_FILE_BODY_BYTES + 1),
    )
    .await;
    assert!(too_large.starts_with("HTTP/1.1 413"), "{too_large}");
    assert!(too_large.contains("\"too_large\""), "{too_large}");

    let no_length = raw_put(&server, &path, &acc.token, "").await;
    assert!(no_length.starts_with("HTTP/1.1 411"), "{no_length}");

    // Shorter than a nonce and a tag: cannot be a sealed file.
    assert_eq!(
        put(
            &server,
            &acc.token,
            acc.home,
            &new_file_id(),
            body_of(39, 1)
        )
        .await,
        StatusCode::BAD_REQUEST
    );
    for bad_id in ["abc", &new_file_id().to_uppercase(), &"g".repeat(32)] {
        assert_eq!(
            put(&server, &acc.token, acc.home, bad_id, body_of(64, 1)).await,
            StatusCode::BAD_REQUEST,
            "id {bad_id:?}"
        );
    }
    assert!(list(&server, &acc.token, acc.home).await.files.is_empty());
}

#[tokio::test]
async fn quota_counts_live_rows_and_restore_respects_it() {
    let server = TestServer::start_with_limit(Some(100)).await;
    let acc = signup(&server).await;
    let (a, b) = (new_file_id(), new_file_id());

    assert_eq!(
        put(&server, &acc.token, acc.home, &a, body_of(60, 1)).await,
        StatusCode::OK
    );
    let resp = http()
        .put(server.file_url(acc.home, &b))
        .bearer_auth(&acc.token)
        .body(body_of(60, 2))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::INSUFFICIENT_STORAGE);
    let err: ErrorBody = resp.json_body().await;
    assert_eq!(err.code, "quota");
    assert!(
        !server.stored_path(acc.home, &b).exists(),
        "a refused upload stores nothing"
    );

    // A tombstoned file stops counting, so the second upload now fits.
    assert_eq!(
        delete(&server, &acc.token, acc.home, &a).await,
        StatusCode::OK
    );
    assert_eq!(
        put(&server, &acc.token, acc.home, &b, body_of(60, 2)).await,
        StatusCode::OK
    );
    // And the first cannot come back on top of it.
    assert_eq!(
        restore(&server, &acc.token, acc.home, &a).await,
        StatusCode::INSUFFICIENT_STORAGE
    );
    let listing = list(&server, &acc.token, acc.home).await;
    assert_eq!(listing.used_bytes, Some(60));
    assert_eq!(listing.limit_bytes, Some(100));

    // The allowance is the account's, pooled over its workspaces:
    // another doc of the same account has no more room than this one.
    let other = create_doc(&server, &acc).await;
    assert_eq!(
        put(&server, &acc.token, other, &new_file_id(), body_of(60, 3)).await,
        StatusCode::INSUFFICIENT_STORAGE
    );
    assert_eq!(
        put(&server, &acc.token, other, &new_file_id(), body_of(40, 3)).await,
        StatusCode::OK
    );
    let listing = list(&server, &acc.token, other).await;
    assert_eq!(listing.doc_bytes, 40);
    assert_eq!(listing.used_bytes, Some(100));

    // A different account has its own.
    let stranger = signup(&server).await;
    assert_eq!(
        put(
            &server,
            &stranger.token,
            stranger.home,
            &new_file_id(),
            body_of(60, 4)
        )
        .await,
        StatusCode::OK
    );
}

/// Files in a shared workspace count against its owner, whoever sent
/// them, and only the owner is told the pooled figures.
#[tokio::test]
async fn a_members_upload_counts_against_the_owner() {
    let server = TestServer::start_with_limit(Some(100)).await;
    let owner = signup(&server).await;
    let member = signup(&server).await;
    let shared = create_doc(&server, &owner).await;
    seed_member(&server, shared, member.account_id).await;

    assert_eq!(
        put(
            &server,
            &owner.token,
            owner.home,
            &new_file_id(),
            body_of(50, 1)
        )
        .await,
        StatusCode::OK
    );
    assert_eq!(
        put(
            &server,
            &member.token,
            shared,
            &new_file_id(),
            body_of(40, 2)
        )
        .await,
        StatusCode::OK
    );
    // 90 of the owner's 100 are used; the member's own allowance is
    // untouched and does not help here.
    assert_eq!(
        put(
            &server,
            &member.token,
            shared,
            &new_file_id(),
            body_of(40, 3)
        )
        .await,
        StatusCode::INSUFFICIENT_STORAGE
    );
    assert_eq!(
        put(
            &server,
            &member.token,
            member.home,
            &new_file_id(),
            body_of(100, 4)
        )
        .await,
        StatusCode::OK
    );

    let seen_by_owner = list(&server, &owner.token, shared).await;
    assert_eq!(seen_by_owner.doc_bytes, 40);
    assert_eq!(seen_by_owner.used_bytes, Some(90));
    assert_eq!(seen_by_owner.limit_bytes, Some(100));
    let seen_by_member = list(&server, &member.token, shared).await;
    assert_eq!(seen_by_member.doc_bytes, 40);
    assert_eq!(seen_by_member.used_bytes, None);
    assert_eq!(seen_by_member.limit_bytes, None);
}

#[tokio::test]
async fn tombstone_then_restore() {
    let server = TestServer::start().await;
    let acc = signup(&server).await;
    let id = new_file_id();
    let body = body_of(80, 4);
    assert_eq!(
        put(&server, &acc.token, acc.home, &id, body.clone()).await,
        StatusCode::OK
    );

    assert_eq!(
        delete(&server, &acc.token, acc.home, &id).await,
        StatusCode::OK
    );
    assert_eq!(
        get(&server, &acc.token, acc.home, &id).await.0,
        StatusCode::GONE
    );
    let listing = list(&server, &acc.token, acc.home).await;
    assert_eq!(listing.doc_bytes, 0);
    assert_eq!(listing.used_bytes, Some(0));
    let stamped = listing.files[0].deleted_at.expect("tombstoned");
    assert!(
        server.stored_path(acc.home, &id).exists(),
        "a tombstone destroys nothing"
    );

    // Repeating the delete later does not move the stamp, so it cannot
    // be used to extend the grace period.
    server.clock.0.fetch_add(5_000, Ordering::SeqCst);
    assert_eq!(
        delete(&server, &acc.token, acc.home, &id).await,
        StatusCode::OK
    );
    assert_eq!(
        list(&server, &acc.token, acc.home).await.files[0].deleted_at,
        Some(stamped)
    );
    // An id that is still there, tombstoned, is still a 409 to upload.
    assert_eq!(
        put(&server, &acc.token, acc.home, &id, body.clone()).await,
        StatusCode::CONFLICT
    );

    assert_eq!(
        restore(&server, &acc.token, acc.home, &id).await,
        StatusCode::OK
    );
    assert_eq!(
        get(&server, &acc.token, acc.home, &id).await,
        (StatusCode::OK, body)
    );
    // Restoring a live file is a no-op, not an error.
    assert_eq!(
        restore(&server, &acc.token, acc.home, &id).await,
        StatusCode::OK
    );

    let unknown = new_file_id();
    assert_eq!(
        delete(&server, &acc.token, acc.home, &unknown).await,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        restore(&server, &acc.token, acc.home, &unknown).await,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        get(&server, &acc.token, acc.home, &unknown).await.0,
        StatusCode::NOT_FOUND
    );
}

#[tokio::test]
async fn copy_duplicates_into_a_doc_the_caller_belongs_to() {
    let server = TestServer::start().await;
    let acc = signup(&server).await;
    let target = create_doc(&server, &acc).await;
    let id = new_file_id();
    let body = body_of(500, 5);
    assert_eq!(
        put(&server, &acc.token, acc.home, &id, body.clone()).await,
        StatusCode::OK
    );

    assert_eq!(
        copy(&server, &acc.token, acc.home, &id, target).await,
        StatusCode::OK
    );
    assert_eq!(
        get(&server, &acc.token, target, &id).await,
        (StatusCode::OK, body.clone())
    );
    assert_eq!(list(&server, &acc.token, target).await.doc_bytes, 500);
    assert_eq!(
        copy(&server, &acc.token, acc.home, &id, target).await,
        StatusCode::CONFLICT
    );

    // The copies are independent: tombstoning the source leaves the
    // target readable, and a tombstoned source cannot be copied.
    assert_eq!(
        delete(&server, &acc.token, acc.home, &id).await,
        StatusCode::OK
    );
    assert_eq!(
        get(&server, &acc.token, target, &id).await.0,
        StatusCode::OK
    );
    let third = create_doc(&server, &acc).await;
    assert_eq!(
        copy(&server, &acc.token, acc.home, &id, third).await,
        StatusCode::GONE
    );

    // Membership of the source alone is not enough.
    let other = signup(&server).await;
    seed_member(&server, target, other.account_id).await;
    assert_eq!(
        copy(&server, &other.token, target, &id, third).await,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        copy(&server, &other.token, target, &id, other.home).await,
        StatusCode::OK
    );
}

#[tokio::test]
async fn copy_counts_against_the_target_owners_quota() {
    let server = TestServer::start_with_limit(Some(100)).await;
    let acc = signup(&server).await;
    let id = new_file_id();
    assert_eq!(
        put(&server, &acc.token, acc.home, &id, body_of(60, 1)).await,
        StatusCode::OK
    );

    // Into another of the caller's own docs: the pool has to hold both
    // copies, since the source is only tombstoned afterwards.
    let own = create_doc(&server, &acc).await;
    assert_eq!(
        copy(&server, &acc.token, acc.home, &id, own).await,
        StatusCode::INSUFFICIENT_STORAGE
    );
    assert!(!server.stored_path(own, &id).exists());

    // Into someone else's doc: their allowance, not the caller's.
    let host = signup(&server).await;
    let hosted = create_doc(&server, &host).await;
    seed_member(&server, hosted, acc.account_id).await;
    assert_eq!(
        copy(&server, &acc.token, acc.home, &id, hosted).await,
        StatusCode::OK
    );
    assert_eq!(
        list(&server, &host.token, hosted).await.used_bytes,
        Some(60)
    );
    let second = new_file_id();
    assert_eq!(
        delete(&server, &acc.token, acc.home, &id).await,
        StatusCode::OK
    );
    assert_eq!(
        put(&server, &acc.token, acc.home, &second, body_of(60, 2)).await,
        StatusCode::OK
    );
    assert_eq!(
        copy(&server, &acc.token, acc.home, &second, hosted).await,
        StatusCode::INSUFFICIENT_STORAGE
    );
    assert!(!server.stored_path(hosted, &second).exists());
}

#[tokio::test]
async fn purge_removes_only_tombstones_past_the_grace_period() {
    let server = TestServer::start().await;
    let acc = signup(&server).await;
    let (old, restored, recent, live) =
        (new_file_id(), new_file_id(), new_file_id(), new_file_id());
    for (id, seed) in [(&old, 1), (&restored, 2), (&recent, 3), (&live, 4)] {
        assert_eq!(
            put(&server, &acc.token, acc.home, id, body_of(64, seed)).await,
            StatusCode::OK
        );
    }
    assert_eq!(
        delete(&server, &acc.token, acc.home, &old).await,
        StatusCode::OK
    );
    assert_eq!(
        delete(&server, &acc.token, acc.home, &restored).await,
        StatusCode::OK
    );
    assert_eq!(
        restore(&server, &acc.token, acc.home, &restored).await,
        StatusCode::OK
    );

    // One millisecond short of the window: nothing goes.
    server
        .clock
        .0
        .fetch_add(TOMBSTONE_GRACE_MILLIS - 1, Ordering::SeqCst);
    assert_eq!(purge_expired(&server.state).await.unwrap(), 0);
    assert_eq!(
        delete(&server, &acc.token, acc.home, &recent).await,
        StatusCode::OK
    );

    server.clock.0.fetch_add(1, Ordering::SeqCst);
    assert_eq!(purge_expired(&server.state).await.unwrap(), 1);

    assert!(!server.stored_path(acc.home, &old).exists());
    assert_eq!(
        get(&server, &acc.token, acc.home, &old).await.0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        restore(&server, &acc.token, acc.home, &old).await,
        StatusCode::NOT_FOUND,
        "past the window there is nothing to restore"
    );
    let mut left: Vec<String> = list(&server, &acc.token, acc.home)
        .await
        .files
        .into_iter()
        .map(|f| f.id)
        .collect();
    left.sort();
    let mut want = vec![restored.clone(), recent.clone(), live.clone()];
    want.sort();
    assert_eq!(left, want);
    for id in [&restored, &recent, &live] {
        assert!(server.stored_path(acc.home, id).exists());
    }
    assert_eq!(
        get(&server, &acc.token, acc.home, &restored).await.0,
        StatusCode::OK
    );
}

#[tokio::test]
async fn deleting_a_doc_removes_its_files_outright() {
    let server = TestServer::start().await;
    let acc = signup(&server).await;
    let doc = create_doc(&server, &acc).await;
    let (id, kept) = (new_file_id(), new_file_id());
    assert_eq!(
        put(&server, &acc.token, doc, &id, body_of(64, 1)).await,
        StatusCode::OK
    );
    assert_eq!(
        put(&server, &acc.token, acc.home, &kept, body_of(64, 2)).await,
        StatusCode::OK
    );

    let status = http()
        .delete(format!("{}/api/docs/{doc}", server.base))
        .bearer_auth(&acc.token)
        .send()
        .await
        .unwrap()
        .status();
    assert!(status.is_success(), "delete doc: {status}");

    assert!(!server.dir.path().join(doc.to_string()).exists());
    assert_eq!(file_rows(&server, doc).await, 0);
    assert_eq!(
        get(&server, &acc.token, doc, &id).await.0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        get(&server, &acc.token, acc.home, &kept).await.0,
        StatusCode::OK
    );
}

#[tokio::test]
async fn deleting_an_account_removes_the_files_of_every_doc_it_owned() {
    let server = TestServer::start().await;
    let acc = signup(&server).await;
    let doc = create_doc(&server, &acc).await;
    for d in [acc.home, doc] {
        assert_eq!(
            put(&server, &acc.token, d, &new_file_id(), body_of(64, 1)).await,
            StatusCode::OK
        );
    }

    let master = derive_password_master(PASSWORD, &acc.master_salt, weak_params()).unwrap();
    let status = http()
        .delete(format!("{}/api/account", server.base))
        .bearer_auth(&acc.token)
        .header(CONTENT_TYPE, "application/json")
        .body(
            serde_json::to_vec(&DeleteAccountRequest {
                current_auth_secret: master.auth_secret().unwrap().as_bytes().to_vec(),
            })
            .unwrap(),
        )
        .send()
        .await
        .unwrap()
        .status();
    assert!(status.is_success(), "delete account: {status}");

    for d in [acc.home, doc] {
        assert!(!server.dir.path().join(d.to_string()).exists());
        assert_eq!(file_rows(&server, d).await, 0);
    }
}

async fn file_rows(server: &TestServer, doc_id: Uuid) -> i64 {
    server
        .state
        .db
        .call(move |c| {
            c.query_row(
                "SELECT COUNT(*) FROM doc_files WHERE doc_id = ?",
                rusqlite::params![doc_id.as_bytes().as_slice()],
                |r| r.get(0),
            )
        })
        .await
        .unwrap()
}

/// A crash mid-write leaves a temp file beside the bodies; opening the
/// store removes it and nothing else.
#[tokio::test]
async fn opening_the_directory_store_removes_stray_temp_files() {
    let dir = tempfile::tempdir().unwrap();
    let doc_dir = dir.path().join(Uuid::now_v7().to_string());
    std::fs::create_dir_all(&doc_dir).unwrap();
    let stray = doc_dir.join(".tmp-0123456789abcdef");
    let body = doc_dir.join(new_file_id());
    std::fs::write(&stray, b"half").unwrap();
    std::fs::write(&body, b"whole").unwrap();

    DirStore::open(dir.path()).unwrap();

    assert!(!stray.exists());
    assert!(body.exists());
}
