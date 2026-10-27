//! End-to-end WS sync: real server, real sqlite, real msgpack frames.
//!
//! Update blobs in these tests are random bytes — the server treats them
//! as opaque, so we don't need the real DEK. Signup still needs valid
//! crypto material because the auth route checks shape; we use the
//! weak Argon2 params from the auth tests so the suite stays fast.

use futures_util::{SinkExt, StreamExt};
use http::header::AUTHORIZATION;
use monoplan_core::{
    Dek, derive_password_master, derive_recovery_master, generate_recovery_code, random_bytes,
};
use monoplan_protocol::{
    ClientFrame, CreateDocRequest, CreateDocResponse, DeviceCredential, DeviceRegistration,
    EncryptedBlob, Hello, HelloAck, HelloRejected, KdfParams, PROTOCOL_VERSION, PushBlob,
    RecoveryMaterial, ServerFrame, SignupRequest, SignupResponse, SnapshotBase, StoredBlob,
};
use monoplan_server::sync::{SnapshotCoordinator, queries};
use monoplan_server::{AppState, router};
use reqwest::header::CONTENT_TYPE;
use serde::Serialize;
use serde::de::DeserializeOwned;
use std::sync::OnceLock;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use uuid::Uuid;

const MSGPACK: &str = "application/msgpack";

fn weak_params() -> KdfParams {
    KdfParams {
        m_kib: 8,
        t: 1,
        p: 1,
    }
}

/// Signup enrols a recovery code unconditionally; sync tests never
/// redeem it, so only the material is kept.
fn enrol_recovery(dek: &Dek, kdf_params: KdfParams) -> RecoveryMaterial {
    let code = generate_recovery_code().unwrap();
    let salt: [u8; 16] = random_bytes();
    let r_master = derive_recovery_master(code.as_str(), &salt, kdf_params).unwrap();
    let r_wrapped = r_master.kek().unwrap().wrap(dek).unwrap();
    RecoveryMaterial {
        recovery_salt: salt.to_vec(),
        recovery_auth_secret: r_master.auth_secret().unwrap().as_bytes().to_vec(),
        recovery_wrapped_dek: r_wrapped.ciphertext,
        recovery_wrapped_dek_nonce: r_wrapped.nonce.to_vec(),
    }
}

struct TestServer {
    base: String,
    ws_base: String,
    state: AppState,
    handle: tokio::task::JoinHandle<()>,
}

impl TestServer {
    async fn start() -> Self {
        Self::start_inner(None).await
    }

    async fn start_with_snapshot_config(
        threshold_blobs: u64,
        timeout: std::time::Duration,
    ) -> Self {
        Self::start_inner(Some(SnapshotCoordinator::with_config(
            threshold_blobs,
            timeout,
        )))
        .await
    }

    async fn start_inner(snapshot_coord: Option<SnapshotCoordinator>) -> Self {
        let mut state = AppState::open_in_memory().await.unwrap();
        if let Some(coord) = snapshot_coord {
            state.snapshot_coordinator = coord;
        }
        let app = router(state.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let base = format!("http://{}", addr);
        let ws_base = format!("ws://{}", addr);
        let handle = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        Self {
            base,
            ws_base,
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

async fn post_msgpack<Req, Resp>(url: &str, body: &Req) -> Resp
where
    Req: Serialize,
    Resp: DeserializeOwned,
{
    post_msgpack_authed(url, body, None).await
}

async fn post_msgpack_authed<Req, Resp>(url: &str, body: &Req, token: Option<&str>) -> Resp
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

struct Account {
    server: TestServer,
    device_token: String,
    device_id: Uuid,
    account_id: Uuid,
    primary_doc_id: Uuid,
}

struct ExtraDevice {
    device_token: String,
    device_id: Uuid,
}

async fn register_second_device(acc: &Account, name: &str) -> ExtraDevice {
    let resp: DeviceCredential = post_msgpack_authed(
        &format!("{}/api/devices", acc.server.base),
        &DeviceRegistration {
            name: name.into(),
            kind: "linux".into(),
        },
        Some(&acc.device_token),
    )
    .await;
    ExtraDevice {
        device_token: resp.device_token,
        device_id: Uuid::parse_str(&resp.device_id).unwrap(),
    }
}

/// Create a second doc the account owns through the real endpoint.
/// The wrap (nominally the doc's DEK under the Home DEK) is fake bytes:
/// the server stores it opaquely and these tests never unwrap.
async fn create_doc(acc: &Account) -> Uuid {
    let resp: CreateDocResponse = post_msgpack_authed(
        &format!("{}/api/docs", acc.server.base),
        &CreateDocRequest {
            wrapped_dek: vec![7u8; 48],
            wrapped_dek_nonce: vec![6u8; 24],
        },
        Some(&acc.device_token),
    )
    .await;
    Uuid::parse_str(&resp.doc_id).unwrap()
}

/// `OpenDoc` on an already-handshaken connection; returns the epoch.
async fn open_doc(ws: &mut WsStream, doc_id: Uuid) -> u64 {
    send_msgpack(ws, &ClientFrame::OpenDoc { doc_id }).await;
    match recv_msgpack::<ServerFrame>(ws).await {
        ServerFrame::DocOpened {
            doc_id: opened,
            epoch,
        } => {
            assert_eq!(opened, doc_id);
            epoch
        }
        other => panic!("expected DocOpened, got {other:?}"),
    }
}

/// Fail if any frame at all arrives within `window`.
async fn assert_no_frame(ws: &mut WsStream, window: std::time::Duration) {
    match tokio::time::timeout(window, recv_msgpack::<ServerFrame>(ws)).await {
        Ok(frame) => panic!("unexpected frame: {frame:?}"),
        Err(_) => {}
    }
}

async fn signup_account() -> Account {
    signup_into(TestServer::start().await).await
}

async fn signup_account_with_snapshot_config(
    threshold_blobs: u64,
    timeout: std::time::Duration,
) -> Account {
    signup_into(TestServer::start_with_snapshot_config(threshold_blobs, timeout).await).await
}

/// Seed a snapshot row directly (bypassing the WS handler). Panics on
/// a chain mismatch — tests that exercise rejection call
/// `queries::insert_snapshot` themselves.
async fn seed_snapshot(
    db: &monoplan_server::db::Db,
    doc_id: Uuid,
    up_to_seq: u64,
    compaction_floor_seq: u64,
    from_seq: Option<u64>,
    seed: u8,
) -> u64 {
    match queries::insert_snapshot(
        db,
        doc_id,
        up_to_seq,
        compaction_floor_seq,
        from_seq,
        fake_blob(seed ^ 0xFF),
        fake_blob(seed),
    )
    .await
    .unwrap()
    {
        queries::InsertSnapshot::Inserted { id } => id,
        other => panic!("seed snapshot rejected: {other:?}"),
    }
}

async fn signup_into(server: TestServer) -> Account {
    let email = format!("user-{}@example.com", Uuid::now_v7());
    let kdf_params = weak_params();
    let master_salt: [u8; 16] = random_bytes();
    let master =
        derive_password_master(b"correct horse battery staple", &master_salt, kdf_params).unwrap();
    let kek = master.kek().unwrap();
    let auth_secret = master.auth_secret().unwrap();
    let dek = Dek::generate();
    let wrapped = kek.wrap(&dek).unwrap();

    let resp: SignupResponse = post_msgpack(
        &format!("{}/api/account/signup", server.base),
        &SignupRequest {
            email,
            master_salt: master_salt.to_vec(),
            kdf_params,
            auth_secret: auth_secret.as_bytes().to_vec(),
            wrapped_dek: wrapped.ciphertext,
            wrapped_dek_nonce: wrapped.nonce.to_vec(),
            recovery: enrol_recovery(&dek, kdf_params),
            device_name: "sync-test".into(),
            device_kind: "linux".into(),
            primary_doc_id: None,
        },
    )
    .await;
    Account {
        device_id: Uuid::parse_str(&resp.device_id).unwrap(),
        account_id: Uuid::parse_str(&resp.account_id).unwrap(),
        primary_doc_id: Uuid::parse_str(&resp.primary_doc_id).unwrap(),
        device_token: resp.device_token,
        server,
    }
}

type WsStream =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

async fn connect_ws(server: &TestServer, token: &str) -> WsStream {
    let url = format!("{}/api/sync", server.ws_base);
    let mut req = url.into_client_request().unwrap();
    req.headers_mut()
        .insert(AUTHORIZATION, format!("Bearer {token}").parse().unwrap());
    let (ws, _) = tokio_tungstenite::connect_async(req).await.unwrap();
    ws
}

async fn send_msgpack<T: Serialize>(ws: &mut WsStream, value: &T) {
    let bytes = rmp_serde::to_vec_named(value).unwrap();
    ws.send(Message::Binary(bytes)).await.unwrap();
}

async fn recv_msgpack<T: DeserializeOwned>(ws: &mut WsStream) -> T {
    loop {
        let msg = ws.next().await.expect("stream closed").unwrap();
        match msg {
            Message::Binary(bytes) => return rmp_serde::from_slice(&bytes).unwrap(),
            Message::Ping(_) | Message::Pong(_) => continue,
            other => panic!("unexpected ws message: {:?}", other),
        }
    }
}

/// Version handshake only: `Hello` → `HelloAck`. No doc is open yet.
async fn hello(ws: &mut WsStream) -> HelloAck {
    send_msgpack(
        ws,
        &Hello {
            client: "monoplan-cli-test".into(),
            client_version: env!("CARGO_PKG_VERSION").into(),
            supported_protocol_versions: vec![PROTOCOL_VERSION],
        },
    )
    .await;
    recv_msgpack(ws).await
}

/// Full connection setup as a client does it: version handshake, then
/// `OpenDoc` for `doc_id`. Returns the doc's epoch from `DocOpened`.
/// The server subscribes the connection before replying, so once this
/// returns the connection receives every later broadcast on the doc.
async fn handshake(ws: &mut WsStream, doc_id: Uuid) -> u64 {
    hello(ws).await;
    send_msgpack(ws, &ClientFrame::OpenDoc { doc_id }).await;
    match recv_msgpack::<ServerFrame>(ws).await {
        ServerFrame::DocOpened {
            doc_id: opened,
            epoch,
        } => {
            assert_eq!(opened, doc_id);
            epoch
        }
        other => panic!("expected DocOpened, got {other:?}"),
    }
}

fn fake_blob(seed: u8) -> EncryptedBlob {
    EncryptedBlob {
        nonce: vec![seed; 24],
        ciphertext: vec![seed; 64],
    }
}

/// Wrap a fake blob with a fresh push_id, as a real client would.
fn fake_push(seed: u8) -> PushBlob {
    PushBlob {
        push_id: Uuid::new_v4(),
        blob: fake_blob(seed),
    }
}

fn fake_pushes(range: std::ops::Range<u8>) -> Vec<PushBlob> {
    range.map(fake_push).collect()
}

/// Extract the assigned seqs from an UpdatesAck frame, in input order.
fn ack_seqs(frame: ServerFrame) -> Vec<u64> {
    match frame {
        ServerFrame::UpdatesAck { acks, .. } => acks.into_iter().map(|a| a.seq).collect(),
        other => panic!("expected UpdatesAck, got {other:?}"),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn ack_does_not_move_backwards() {
    let acc = signup_account().await;
    let mut ws = connect_ws(&acc.server, &acc.device_token).await;
    handshake(&mut ws, acc.primary_doc_id).await;

    // Push one op so there's something to ack against.
    send_msgpack(
        &mut ws,
        &ClientFrame::PushUpdates {
            doc_id: acc.primary_doc_id,
            updates: vec![fake_push(1)],
        },
    )
    .await;
    let resp: ServerFrame = recv_msgpack(&mut ws).await;
    let id = ack_seqs(resp)[0];

    send_msgpack(
        &mut ws,
        &ClientFrame::Ack {
            doc_id: acc.primary_doc_id,
            last_acked_seq: id,
        },
    )
    .await;
    wait_for_acked(&acc, id).await;
    // Stale ack from a slow client mustn't drag the frontier down.
    send_msgpack(
        &mut ws,
        &ClientFrame::Ack {
            doc_id: acc.primary_doc_id,
            last_acked_seq: 0,
        },
    )
    .await;
    // Give the server time to *not* apply it.
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    let stored =
        queries::get_last_acked_seq(&acc.server.state.db, acc.device_id, acc.primary_doc_id)
            .await
            .unwrap();
    assert_eq!(stored, id);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn repeated_push_id_dedups_to_original_seq_without_new_row() {
    // Crash-retry contract (spec/vv-wal-separation.md test 10, server
    // half): re-sending the same (device, push_id) acks the ORIGINAL
    // seq, inserts no second row, and is not re-broadcast to peers.
    let acc = signup_account().await;
    let device_b = register_second_device(&acc, "device-b").await;
    let mut ws_a = connect_ws(&acc.server, &acc.device_token).await;
    handshake(&mut ws_a, acc.primary_doc_id).await;
    let mut ws_b = connect_ws(&acc.server, &device_b.device_token).await;
    handshake(&mut ws_b, acc.primary_doc_id).await;
    wait_for_subscribers(&acc, 2).await;

    let push = fake_push(3);
    send_msgpack(
        &mut ws_a,
        &ClientFrame::PushUpdates {
            doc_id: acc.primary_doc_id,
            updates: vec![push.clone()],
        },
    )
    .await;
    let first = ack_seqs(recv_msgpack::<ServerFrame>(&mut ws_a).await);
    assert_eq!(first, vec![1]);
    // B sees the original broadcast once.
    let broadcast = match recv_msgpack::<ServerFrame>(&mut ws_b).await {
        ServerFrame::UpdatesBroadcast { updates, .. } => updates,
        other => panic!("expected UpdatesBroadcast on B, got {other:?}"),
    };
    assert_eq!(broadcast.len(), 1);

    // Retry the exact same push (simulating a lost ack).
    send_msgpack(
        &mut ws_a,
        &ClientFrame::PushUpdates {
            doc_id: acc.primary_doc_id,
            updates: vec![push],
        },
    )
    .await;
    let second = ack_seqs(recv_msgpack::<ServerFrame>(&mut ws_a).await);
    assert_eq!(second, vec![1], "retry must ack the original seq");

    // No second stored row.
    let batch = queries::fetch_updates_batch(&acc.server.state.db, acc.primary_doc_id, 0)
        .await
        .unwrap();
    assert_eq!(batch.updates.len(), 1, "dedup must not insert a second row");

    // No duplicate broadcast to B.
    let nothing = tokio::time::timeout(std::time::Duration::from_millis(200), ws_b.next()).await;
    assert!(
        nothing.is_err(),
        "B unexpectedly received a re-broadcast: {nothing:?}"
    );

    // A *different* push_id from the same device still inserts fresh.
    send_msgpack(
        &mut ws_a,
        &ClientFrame::PushUpdates {
            doc_id: acc.primary_doc_id,
            updates: vec![fake_push(4)],
        },
    )
    .await;
    let third = ack_seqs(recv_msgpack::<ServerFrame>(&mut ws_a).await);
    assert_eq!(third, vec![2], "fresh push_id gets the next dense seq");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn handshake_rejected_when_no_shared_protocol_version() {
    let acc = signup_account().await;
    let mut ws = connect_ws(&acc.server, &acc.device_token).await;

    send_msgpack(
        &mut ws,
        &Hello {
            client: "monoplan-cli-test".into(),
            client_version: "0.0.0".into(),
            supported_protocol_versions: vec![999],
        },
    )
    .await;
    let rejected: HelloRejected = recv_msgpack(&mut ws).await;
    assert!(rejected.reason.contains("no shared protocol version"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn ws_upgrade_without_token_is_rejected() {
    let server = TestServer::start().await;
    let url = format!("{}/api/sync", server.ws_base);
    let req = url.into_client_request().unwrap();
    let err = tokio_tungstenite::connect_async(req).await.unwrap_err();
    let msg = err.to_string();
    assert!(
        msg.contains("401") || msg.contains("Unauthorized"),
        "expected 401 in error, got: {msg}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn push_on_a_broadcasts_to_b_not_a() {
    let acc = signup_account().await;
    let device_b = register_second_device(&acc, "device-b").await;

    let mut ws_a = connect_ws(&acc.server, &acc.device_token).await;
    handshake(&mut ws_a, acc.primary_doc_id).await;
    let mut ws_b = connect_ws(&acc.server, &device_b.device_token).await;
    handshake(&mut ws_b, acc.primary_doc_id).await;

    // Both sessions subscribe immediately after handshake; wait for
    // the registry to reflect that before pushing into broadcast.
    wait_for_subscribers(&acc, 2).await;

    let pushes = vec![fake_push(7), fake_push(8)];
    send_msgpack(
        &mut ws_a,
        &ClientFrame::PushUpdates {
            doc_id: acc.primary_doc_id,
            updates: pushes.clone(),
        },
    )
    .await;

    // A receives its own UpdatesAck; A must NOT receive a broadcast for
    // its own push. Asserting absence requires a small wait —
    // anything pending arrives well within 200ms on localhost.
    let assigned_seqs = ack_seqs(recv_msgpack::<ServerFrame>(&mut ws_a).await);
    assert_eq!(assigned_seqs.len(), 2);

    // B receives the broadcast with the same ids/blobs.
    let broadcast = match recv_msgpack::<ServerFrame>(&mut ws_b).await {
        ServerFrame::UpdatesBroadcast { updates, .. } => updates,
        other => panic!("expected UpdatesBroadcast on B, got {other:?}"),
    };
    let want: Vec<StoredBlob> = assigned_seqs
        .iter()
        .copied()
        .zip(pushes)
        .map(|(seq, p)| StoredBlob { seq, blob: p.blob })
        .collect();
    assert_eq!(broadcast, want);

    // A's stream should have nothing pending.
    let nothing = tokio::time::timeout(std::time::Duration::from_millis(200), ws_a.next()).await;
    assert!(nothing.is_err(), "A unexpectedly received: {nothing:?}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn push_on_one_tab_broadcasts_to_other_tab_same_device() {
    // Multi-tab on the same device: two WS connections share the
    // device cookie. Broadcast must exclude only the originating
    // *connection*, not every connection sharing the device id —
    // otherwise tab→tab updates silently drop.
    let acc = signup_account().await;

    let mut tab_a = connect_ws(&acc.server, &acc.device_token).await;
    handshake(&mut tab_a, acc.primary_doc_id).await;
    let mut tab_b = connect_ws(&acc.server, &acc.device_token).await;
    handshake(&mut tab_b, acc.primary_doc_id).await;
    wait_for_subscribers(&acc, 2).await;

    let pushes = vec![fake_push(11)];
    send_msgpack(
        &mut tab_a,
        &ClientFrame::PushUpdates {
            doc_id: acc.primary_doc_id,
            updates: pushes.clone(),
        },
    )
    .await;

    // Tab A — its own ack, no broadcast echo.
    let assigned_seqs = ack_seqs(recv_msgpack::<ServerFrame>(&mut tab_a).await);
    assert_eq!(assigned_seqs.len(), 1);

    // Tab B — receives the broadcast even though it shares device_id.
    let broadcast = match recv_msgpack::<ServerFrame>(&mut tab_b).await {
        ServerFrame::UpdatesBroadcast { updates, .. } => updates,
        other => panic!("expected UpdatesBroadcast on B, got {other:?}"),
    };
    let want: Vec<StoredBlob> = assigned_seqs
        .into_iter()
        .zip(pushes)
        .map(|(seq, p)| StoredBlob { seq, blob: p.blob })
        .collect();
    assert_eq!(broadcast, want);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn subscriber_unregisters_on_disconnect() {
    let acc = signup_account().await;
    let _device_b = register_second_device(&acc, "device-b").await;

    let mut ws_a = connect_ws(&acc.server, &acc.device_token).await;
    handshake(&mut ws_a, acc.primary_doc_id).await;
    wait_for_subscribers(&acc, 1).await;
    drop(ws_a);
    wait_for_subscribers(&acc, 0).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn pull_at_or_above_snapshot_floor_streams_ops_normally() {
    // A device whose cursor is already >= the snapshot floor stays
    // on the steady-state op-streaming path — no SnapshotRequired.
    let acc = signup_account().await;
    // compaction_floor = up_to here means cursor=50 is *at* the floor,
    // not below — pull_updates should still stream normally without bootstrap.
    seed_snapshot(&acc.server.state.db, acc.primary_doc_id, 50, 50, None, 0xAA).await;

    let mut ws = connect_ws(&acc.server, &acc.device_token).await;
    handshake(&mut ws, acc.primary_doc_id).await;

    send_msgpack(
        &mut ws,
        &ClientFrame::PullUpdates {
            doc_id: acc.primary_doc_id,
            since_seq: 50,
        },
    )
    .await;
    let pulled = expect_complete_batch(&mut ws).await;
    assert!(pulled.is_empty());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn no_snapshot_request_below_threshold() {
    // Threshold=5; push 3 updates (well under) and ack them. The server
    // must never emit a SnapshotRequest.
    let acc = signup_account_with_snapshot_config(5, std::time::Duration::from_secs(60)).await;
    let mut ws = connect_ws(&acc.server, &acc.device_token).await;
    handshake(&mut ws, acc.primary_doc_id).await;

    let pushes = fake_pushes(0..3);
    send_msgpack(
        &mut ws,
        &ClientFrame::PushUpdates {
            doc_id: acc.primary_doc_id,
            updates: pushes,
        },
    )
    .await;
    let assigned_seqs = ack_seqs(recv_msgpack::<ServerFrame>(&mut ws).await);
    let last_id = *assigned_seqs.last().unwrap();
    send_msgpack(
        &mut ws,
        &ClientFrame::Ack {
            doc_id: acc.primary_doc_id,
            last_acked_seq: last_id,
        },
    )
    .await;
    assert_no_snapshot_request(&mut ws, std::time::Duration::from_millis(200)).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn push_path_triggers_snapshot_request_and_persists() {
    // Single device crosses threshold on push. Server emits a
    // SnapshotRequest before the UpdatesAck; client responds with a
    // PushSnapshot; row lands in the snapshots table.
    let acc = signup_account_with_snapshot_config(5, std::time::Duration::from_secs(60)).await;
    let mut ws = connect_ws(&acc.server, &acc.device_token).await;
    handshake(&mut ws, acc.primary_doc_id).await;

    let pushes = fake_pushes(0..5);
    send_msgpack(
        &mut ws,
        &ClientFrame::PushUpdates {
            doc_id: acc.primary_doc_id,
            updates: pushes,
        },
    )
    .await;
    let (up_to_seq, compaction_floor_seq, _base) = expect_snapshot_request(&mut ws).await;
    assert_eq!(up_to_seq, 5);
    // Single device, just pushed-and-acked: horizon = 5 = up_to.
    assert_eq!(compaction_floor_seq, 5);

    let snapshot_blob = fake_blob(0xCC);
    send_msgpack(
        &mut ws,
        &ClientFrame::PushSnapshot {
            doc_id: acc.primary_doc_id,
            up_to_seq,
            compaction_floor_seq,
            from_seq: None,
            up_to_vv: fake_blob(0x11),
            blob: snapshot_blob.clone(),
        },
    )
    .await;
    let snap = wait_for_snapshot(&acc, up_to_seq).await;
    assert_eq!(snap.blob, snapshot_blob);
    assert_eq!(snap.compaction_floor_seq, compaction_floor_seq);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn ack_path_triggers_snapshot_request() {
    // Inject updates directly via the query layer so the push-path
    // doesn't fire eligibility for the connecting device. Once the
    // device acks the latest blob id, the Ack handler must drive
    // evaluate and emit a SnapshotRequest.
    let acc = signup_account_with_snapshot_config(5, std::time::Duration::from_secs(60)).await;
    let pushes = fake_pushes(0..5);
    let inserted = queries::insert_updates(
        &acc.server.state.db,
        acc.primary_doc_id,
        acc.device_id,
        pushes,
    )
    .await
    .unwrap();
    let last_id = inserted.acks.last().unwrap().seq;

    let mut ws = connect_ws(&acc.server, &acc.device_token).await;
    handshake(&mut ws, acc.primary_doc_id).await;
    send_msgpack(
        &mut ws,
        &ClientFrame::Ack {
            doc_id: acc.primary_doc_id,
            last_acked_seq: last_id,
        },
    )
    .await;
    let (up_to_seq, _compaction_floor_seq, _base) = expect_snapshot_request(&mut ws).await;
    assert_eq!(up_to_seq, last_id);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn stale_snapshot_after_lease_expiry_rejected() {
    // Short timeout. A's push triggers lease 1 (issued to A). A never
    // replies; lease expires. B then acks → fresh lease 2 (on its own
    // WS session). A's late `PushSnapshot` for lease 1 must be rejected
    // — `release` returns Stale against ws_a's stored lease id.
    let acc = signup_account_with_snapshot_config(5, std::time::Duration::from_millis(100)).await;
    let device_b = register_second_device(&acc, "device-b").await;

    let mut ws_a = connect_ws(&acc.server, &acc.device_token).await;
    handshake(&mut ws_a, acc.primary_doc_id).await;
    let pushes = fake_pushes(0..5);
    send_msgpack(
        &mut ws_a,
        &ClientFrame::PushUpdates {
            doc_id: acc.primary_doc_id,
            updates: pushes,
        },
    )
    .await;
    let (up_to_seq, compaction_floor_seq, _base) = expect_snapshot_request(&mut ws_a).await;
    assert_eq!(up_to_seq, 5);
    // B has never acked, so horizon == 0 — first snapshot has no
    // compaction floor, just bootstrap state.
    assert_eq!(compaction_floor_seq, 0);

    // A acks (a real client would, off the back of UpdatesAck) so its DB
    // row advances. Otherwise B's later ack eval still sees horizon=0.
    // Lease 1 stays active — this eval skips on lease-in-flight.
    send_msgpack(
        &mut ws_a,
        &ClientFrame::Ack {
            doc_id: acc.primary_doc_id,
            last_acked_seq: up_to_seq,
        },
    )
    .await;

    // Wait past timeout so A's lease is expired in the coordinator.
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;

    // B acks at the same frontier — coordinator sees A's lease as
    // expired, issues a fresh one to B.
    let mut ws_b = connect_ws(&acc.server, &device_b.device_token).await;
    handshake(&mut ws_b, acc.primary_doc_id).await;
    send_msgpack(
        &mut ws_b,
        &ClientFrame::Ack {
            doc_id: acc.primary_doc_id,
            last_acked_seq: up_to_seq,
        },
    )
    .await;
    let (b_up_to, b_floor, _base) = expect_snapshot_request(&mut ws_b).await;
    assert_eq!(b_up_to, up_to_seq);
    // Both devices now caught up → horizon advances to 5; B's lease's
    // compaction floor tracks horizon.
    assert_eq!(b_floor, 5);

    // A's late snapshot for lease 1 — must be dropped. ws_a's stored
    // lease id is 1; coordinator's current lease is 2 → Stale.
    send_msgpack(
        &mut ws_a,
        &ClientFrame::PushSnapshot {
            doc_id: acc.primary_doc_id,
            up_to_seq,
            compaction_floor_seq,
            from_seq: None,
            up_to_vv: fake_blob(0x11),
            blob: fake_blob(0xAA),
        },
    )
    .await;
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    let snap = queries::latest_snapshot(&acc.server.state.db, acc.primary_doc_id)
        .await
        .unwrap();
    assert!(
        snap.is_none(),
        "stale snapshot from expired lease must not persist"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn compact_doc_deletes_ops_below_floor_and_keeps_the_chain() {
    // Insert 10 updates + a snapshot pinning compaction_floor at 6 (so ids
    // 1..=6 are below the floor, 7..=10 are above). Run compaction:
    // updates below should be gone, updates above intact, snapshot rows never
    // touched.
    let acc = signup_account().await;
    let db = &acc.server.state.db;
    let pushes = fake_pushes(0..10);
    let inserted = queries::insert_updates(db, acc.primary_doc_id, acc.device_id, pushes)
        .await
        .unwrap();
    let ids: Vec<u64> = inserted.acks.iter().map(|a| a.seq).collect();
    assert_eq!(ids, (1..=10).collect::<Vec<_>>());

    let snap1 = seed_snapshot(db, acc.primary_doc_id, 10, 6, None, 0xA1).await;

    // Every snapshot today is full-history or a lossless delta on one,
    // so it is stamped with the doc's current epoch, which never
    // leaves 0 until trimming lands.
    assert_eq!(queries::doc_epoch(db, acc.primary_doc_id).await.unwrap(), 0);
    let meta = queries::latest_snapshot_meta(db, acc.primary_doc_id)
        .await
        .unwrap()
        .expect("snapshot row");
    assert_eq!(meta.epoch, 0);
    assert_eq!(meta.from_seq, None);

    let stats = queries::compact_doc(db, acc.primary_doc_id).await.unwrap();
    assert_eq!(stats.updates_deleted, 6);

    // Surviving updates are exactly 7..=10.
    let batch = queries::fetch_updates_batch(db, acc.primary_doc_id, 0)
        .await
        .unwrap();
    let surviving_ids: Vec<u64> = batch.updates.iter().map(|o| o.seq).collect();
    assert_eq!(surviving_ids, vec![7, 8, 9, 10]);

    // Idempotent — second run finds no new floor movement.
    let stats2 = queries::compact_doc(db, acc.primary_doc_id).await.unwrap();
    assert_eq!(stats2.updates_deleted, 0);

    // A delta chained on snap1 joins the chain; compaction leaves
    // both rows alone.
    let snap2 = seed_snapshot(db, acc.primary_doc_id, 10, 6, Some(10), 0xA2).await;
    assert!(snap1 < snap2);
    let meta = queries::latest_snapshot_meta(db, acc.primary_doc_id)
        .await
        .unwrap()
        .expect("snapshot row");
    assert_eq!(meta.from_seq, Some(10));
    queries::compact_doc(db, acc.primary_doc_id).await.unwrap();
    let chain = queries::snapshot_chain(db, acc.primary_doc_id)
        .await
        .unwrap()
        .expect("chain");
    let blobs: Vec<_> = chain.rows.iter().map(|r| r.blob.clone()).collect();
    assert_eq!(blobs, vec![fake_blob(0xA1), fake_blob(0xA2)]);

    // Bootstrap path still works: a fresh device pulling from 0 gets
    // SnapshotRequired (its cursor is below the compaction floor),
    // not an UpdatesBatch with holes.
    let fresh_device = register_second_device(&acc, "device-fresh").await;
    let mut ws = connect_ws(&acc.server, &fresh_device.device_token).await;
    handshake(&mut ws, acc.primary_doc_id).await;
    send_msgpack(
        &mut ws,
        &ClientFrame::PullUpdates {
            doc_id: acc.primary_doc_id,
            since_seq: 0,
        },
    )
    .await;
    match recv_msgpack::<ServerFrame>(&mut ws).await {
        ServerFrame::SnapshotRequired { up_to_seq, .. } => assert_eq!(up_to_seq, 10),
        other => panic!("expected SnapshotRequired, got {other:?}"),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn compact_doc_with_no_snapshot_is_noop() {
    let acc = signup_account().await;
    let db = &acc.server.state.db;
    queries::insert_updates(
        db,
        acc.primary_doc_id,
        acc.device_id,
        vec![fake_push(1), fake_push(2)],
    )
    .await
    .unwrap();
    let stats = queries::compact_doc(db, acc.primary_doc_id).await.unwrap();
    assert_eq!(stats.updates_deleted, 0);
    // Ops untouched.
    let batch = queries::fetch_updates_batch(db, acc.primary_doc_id, 0)
        .await
        .unwrap();
    assert_eq!(batch.updates.len(), 2);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn push_snapshot_opportunistically_compacts_ops_below_floor() {
    // End-to-end: a real PushSnapshot through the WS handler should
    // trigger the spawned compaction. With threshold=5, pushing 5 updates
    // and acking lands a snapshot at compaction_floor=5, after which ids
    // 1..=5 should disappear from the updates table.
    let acc = signup_account_with_snapshot_config(5, std::time::Duration::from_secs(60)).await;
    let mut ws = connect_ws(&acc.server, &acc.device_token).await;
    handshake(&mut ws, acc.primary_doc_id).await;

    let pushes = fake_pushes(0..5);
    send_msgpack(
        &mut ws,
        &ClientFrame::PushUpdates {
            doc_id: acc.primary_doc_id,
            updates: pushes,
        },
    )
    .await;
    let (up_to_seq, compaction_floor_seq, _base) = expect_snapshot_request(&mut ws).await;
    assert_eq!(compaction_floor_seq, 5);

    send_msgpack(
        &mut ws,
        &ClientFrame::PushSnapshot {
            doc_id: acc.primary_doc_id,
            up_to_seq,
            compaction_floor_seq,
            from_seq: None,
            up_to_vv: fake_blob(0x11),
            blob: fake_blob(0xCC),
        },
    )
    .await;
    wait_for_snapshot(&acc, up_to_seq).await;

    // Compaction is spawned, not awaited inline — poll until the updates
    // disappear (or the deadline trips).
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
    loop {
        let batch = queries::fetch_updates_batch(&acc.server.state.db, acc.primary_doc_id, 0)
            .await
            .unwrap();
        if batch.updates.is_empty() {
            break;
        }
        if std::time::Instant::now() > deadline {
            panic!(
                "updates below floor not compacted (still {} present)",
                batch.updates.len()
            );
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn delta_chain_requested_and_served_in_order() {
    // threshold=2: the first snapshot is full, every later one is a
    // delta whose request carries the previous row's sealed VV as the
    // base. A bootstrapper gets the whole chain in order.
    let acc = signup_account_with_snapshot_config(2, std::time::Duration::from_secs(60)).await;
    let mut ws = connect_ws(&acc.server, &acc.device_token).await;
    handshake(&mut ws, acc.primary_doc_id).await;

    // 1. Two updates → full snapshot requested (no base).
    send_msgpack(
        &mut ws,
        &ClientFrame::PushUpdates {
            doc_id: acc.primary_doc_id,
            updates: fake_pushes(0..2),
        },
    )
    .await;
    let (up_to, floor, base) = expect_snapshot_request(&mut ws).await;
    assert_eq!(up_to, 2);
    assert!(base.is_none(), "first snapshot must be full");
    send_msgpack(
        &mut ws,
        &ClientFrame::PushSnapshot {
            doc_id: acc.primary_doc_id,
            up_to_seq: up_to,
            compaction_floor_seq: floor,
            from_seq: None,
            up_to_vv: fake_blob(0x01),
            blob: fake_blob(0xA1),
        },
    )
    .await;
    wait_for_snapshot(&acc, 2).await;

    // 2. Two more → delta requested from the full snapshot's VV.
    send_msgpack(
        &mut ws,
        &ClientFrame::PushUpdates {
            doc_id: acc.primary_doc_id,
            updates: fake_pushes(2..4),
        },
    )
    .await;
    let (up_to, floor, base) = expect_snapshot_request(&mut ws).await;
    assert_eq!(up_to, 4);
    assert_eq!(
        base,
        Some(SnapshotBase {
            seq: 2,
            vv: fake_blob(0x01)
        })
    );
    send_msgpack(
        &mut ws,
        &ClientFrame::PushSnapshot {
            doc_id: acc.primary_doc_id,
            up_to_seq: up_to,
            compaction_floor_seq: floor,
            from_seq: Some(2),
            up_to_vv: fake_blob(0x02),
            blob: fake_blob(0xA2),
        },
    )
    .await;
    let snap = wait_for_snapshot(&acc, 4).await;
    assert_eq!(snap.from_seq, Some(2));
    assert_eq!(snap.up_to_vv, fake_blob(0x02));

    // A fresh device bootstraps from the chain, in order.
    let fresh = register_second_device(&acc, "device-fresh").await;
    let mut ws_f = connect_ws(&acc.server, &fresh.device_token).await;
    handshake(&mut ws_f, acc.primary_doc_id).await;
    send_msgpack(
        &mut ws_f,
        &ClientFrame::PullSnapshot {
            doc_id: acc.primary_doc_id,
        },
    )
    .await;
    // One row per frame, head first, `complete` on the last; each
    // frame carries its own row's frontier.
    let rows = recv_snapshot_chain(&mut ws_f).await;
    assert_eq!(
        rows,
        vec![(2, fake_blob(0xA1), false), (4, fake_blob(0xA2), true)]
    );

    // 3. Two more → delta again, from the previous delta's VV; the
    //    chain keeps growing and is served whole.
    send_msgpack(
        &mut ws,
        &ClientFrame::PushUpdates {
            doc_id: acc.primary_doc_id,
            updates: fake_pushes(4..6),
        },
    )
    .await;
    let (up_to, floor, base) = expect_snapshot_request(&mut ws).await;
    assert_eq!(up_to, 6);
    assert_eq!(
        base,
        Some(SnapshotBase {
            seq: 4,
            vv: fake_blob(0x02)
        })
    );
    send_msgpack(
        &mut ws,
        &ClientFrame::PushSnapshot {
            doc_id: acc.primary_doc_id,
            up_to_seq: up_to,
            compaction_floor_seq: floor,
            from_seq: Some(4),
            up_to_vv: fake_blob(0x03),
            blob: fake_blob(0xA3),
        },
    )
    .await;
    wait_for_snapshot(&acc, 6).await;

    send_msgpack(
        &mut ws_f,
        &ClientFrame::PullSnapshot {
            doc_id: acc.primary_doc_id,
        },
    )
    .await;
    // A's pushes were broadcast to the fresh device's socket too;
    // `recv_snapshot_chain` skips past them to the Snapshot rows.
    let rows = recv_snapshot_chain(&mut ws_f).await;
    assert_eq!(
        rows,
        vec![
            (2, fake_blob(0xA1), false),
            (4, fake_blob(0xA2), false),
            (6, fake_blob(0xA3), true),
        ]
    );
}

/// Collect a streamed snapshot chain: every `Snapshot` frame up to and
/// including the `complete` one, as `(up_to_seq, blob, complete)`.
/// Interleaved `UpdatesBroadcast` frames are skipped.
async fn recv_snapshot_chain(ws: &mut WsStream) -> Vec<(u64, EncryptedBlob, bool)> {
    let mut rows = Vec::new();
    loop {
        match recv_msgpack::<ServerFrame>(ws).await {
            ServerFrame::Snapshot {
                doc_id: _,
                up_to_seq,
                blob,
                complete,
            } => {
                rows.push((up_to_seq, blob, complete));
                if complete {
                    return rows;
                }
            }
            ServerFrame::UpdatesBroadcast { .. } => continue,
            other => panic!("expected Snapshot, got {other:?}"),
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn delta_that_does_not_chain_onto_latest_row_is_rejected() {
    let acc = signup_account().await;
    let db = &acc.server.state.db;
    // Nothing to chain onto yet.
    let r = queries::insert_snapshot(
        db,
        acc.primary_doc_id,
        5,
        0,
        Some(0),
        fake_blob(1),
        fake_blob(2),
    )
    .await
    .unwrap();
    assert!(matches!(
        r,
        queries::InsertSnapshot::ChainMismatch {
            latest_up_to_seq: None
        }
    ));

    seed_snapshot(db, acc.primary_doc_id, 10, 0, None, 0xA1).await;
    // Stale base: latest is at 10, delta claims to start after 5.
    let r = queries::insert_snapshot(
        db,
        acc.primary_doc_id,
        12,
        0,
        Some(5),
        fake_blob(1),
        fake_blob(2),
    )
    .await
    .unwrap();
    assert!(matches!(
        r,
        queries::InsertSnapshot::ChainMismatch {
            latest_up_to_seq: Some(10)
        }
    ));
    // Delta ending before its own base.
    let r = queries::insert_snapshot(
        db,
        acc.primary_doc_id,
        9,
        0,
        Some(10),
        fake_blob(1),
        fake_blob(2),
    )
    .await
    .unwrap();
    assert!(matches!(r, queries::InsertSnapshot::ChainMismatch { .. }));
    // Nothing landed; the chain is still just the full row.
    let chain = queries::snapshot_chain(db, acc.primary_doc_id)
        .await
        .unwrap()
        .expect("chain");
    assert_eq!(chain.rows.len(), 1);
    assert_eq!(chain.rows[0].blob, fake_blob(0xA1));
    assert_eq!(chain.up_to_seq, 10);
}

async fn expect_snapshot_request(ws: &mut WsStream) -> (u64, u64, Option<SnapshotBase>) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
    loop {
        let remaining = deadline.saturating_duration_since(std::time::Instant::now());
        let frame = match tokio::time::timeout(remaining, recv_msgpack::<ServerFrame>(ws)).await {
            Ok(f) => f,
            Err(_) => panic!("no SnapshotRequest within deadline"),
        };
        match frame {
            ServerFrame::SnapshotRequest {
                doc_id: _,
                up_to_seq,
                compaction_floor_seq,
                base,
            } => return (up_to_seq, compaction_floor_seq, base),
            ServerFrame::UpdatesAck { .. } | ServerFrame::UpdatesBroadcast { .. } => continue,
            other => panic!("unexpected frame while waiting for SnapshotRequest: {other:?}"),
        }
    }
}

async fn assert_no_snapshot_request(ws: &mut WsStream, window: std::time::Duration) {
    let deadline = std::time::Instant::now() + window;
    loop {
        let remaining = deadline.saturating_duration_since(std::time::Instant::now());
        if remaining.is_zero() {
            return;
        }
        match tokio::time::timeout(remaining, recv_msgpack::<ServerFrame>(ws)).await {
            Ok(ServerFrame::UpdatesAck { .. }) | Ok(ServerFrame::UpdatesBroadcast { .. }) => {
                continue;
            }
            Ok(ServerFrame::SnapshotRequest {
                doc_id: _,
                up_to_seq,
                compaction_floor_seq,
                ..
            }) => {
                panic!(
                    "unexpected SnapshotRequest up_to_seq={up_to_seq} compaction_floor_seq={compaction_floor_seq}"
                );
            }
            Ok(other) => panic!("unexpected frame: {other:?}"),
            Err(_) => return,
        }
    }
}

async fn wait_for_snapshot(acc: &Account, up_to: u64) -> queries::LatestSnapshot {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
    loop {
        if let Some(snap) = queries::latest_snapshot(&acc.server.state.db, acc.primary_doc_id)
            .await
            .unwrap()
        {
            if snap.up_to_seq == up_to {
                return snap;
            }
        }
        if std::time::Instant::now() > deadline {
            panic!("snapshot up_to={up_to} never landed");
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
}

async fn expect_complete_batch(ws: &mut WsStream) -> Vec<StoredBlob> {
    match recv_msgpack(ws).await {
        ServerFrame::UpdatesBatch {
            updates, complete, ..
        } => {
            assert!(complete, "expected single complete batch");
            updates
        }
        other => panic!("expected UpdatesBatch, got {other:?}"),
    }
}

/// Poll until the hub holds at least `target` subscribers for the
/// account's doc. The server subscribes before it replies `DocOpened`,
/// so after `handshake` this is immediate; it still earns its keep for
/// `target == 0` (waiting for a disconnect to deregister).
async fn wait_for_subscribers(acc: &Account, target: usize) {
    wait_for_subscribers_on(acc, acc.primary_doc_id, target).await
}

async fn wait_for_subscribers_on(acc: &Account, doc_id: Uuid, target: usize) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
    loop {
        let count = acc.server.state.sync_sessions.subscriber_count(doc_id);
        if (target == 0 && count == 0) || (target > 0 && count >= target) {
            return;
        }
        if std::time::Instant::now() > deadline {
            panic!("subscriber count never reached {target} (stuck at {count})");
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
}

/// Poll until the device row's `last_acked_seq` reaches the target,
/// then return whatever the row holds. Avoids racing the WS task's
/// commit point.
async fn wait_for_acked(acc: &Account, target: u64) -> u64 {
    wait_for_acked_on(acc, acc.primary_doc_id, target).await
}

async fn wait_for_acked_on(acc: &Account, doc_id: Uuid, target: u64) -> u64 {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
    loop {
        let stored = queries::get_last_acked_seq(&acc.server.state.db, acc.device_id, doc_id)
            .await
            .unwrap();
        if stored >= target {
            return stored;
        }
        if std::time::Instant::now() > deadline {
            panic!("ack never reached {target} (stuck at {stored})");
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
}

/// Drain until the server closes the socket. Tolerates a late
/// `DocRejected`/close handshake; panics on any other payload frame.
async fn expect_closed(ws: &mut WsStream) {
    let deadline = std::time::Duration::from_secs(2);
    loop {
        match tokio::time::timeout(deadline, ws.next()).await {
            Ok(None) | Ok(Some(Err(_))) | Ok(Some(Ok(Message::Close(_)))) => return,
            Ok(Some(Ok(Message::Ping(_)))) | Ok(Some(Ok(Message::Pong(_)))) => continue,
            Ok(Some(Ok(other))) => panic!("expected close, got {other:?}"),
            Err(_) => panic!("socket still open after {deadline:?}"),
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn open_doc_for_foreign_doc_is_rejected_and_closes() {
    let acc = signup_account().await;
    let mut ws = connect_ws(&acc.server, &acc.device_token).await;
    hello(&mut ws).await;

    let foreign = Uuid::now_v7();
    send_msgpack(&mut ws, &ClientFrame::OpenDoc { doc_id: foreign }).await;
    match recv_msgpack::<ServerFrame>(&mut ws).await {
        ServerFrame::DocRejected { doc_id, reason } => {
            assert_eq!(doc_id, foreign);
            assert!(reason.contains("member"), "reason: {reason}");
        }
        other => panic!("expected DocRejected, got {other:?}"),
    }
    expect_closed(&mut ws).await;
    assert_eq!(
        acc.server.state.sync_sessions.subscriber_count(foreign),
        0,
        "a refused open must not subscribe"
    );
}

/// Seed a second doc and an active `doc_members` row for `acc` directly
/// — the invite flow does not exist yet, but access is already decided
/// by membership rather than by "is the primary doc".
async fn seed_membership(acc: &Account, role: &str) -> Uuid {
    let doc_id = Uuid::now_v7();
    let account_id = acc.account_id;
    let role = role.to_string();
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
                 ) VALUES (?, ?, ?, ?, ?, 2)",
                rusqlite::params![
                    doc_id.as_bytes().as_slice(),
                    account_id.as_bytes().as_slice(),
                    role,
                    vec![9u8; 48],
                    vec![8u8; 24],
                ],
            )?;
            tx.commit()?;
            Ok(())
        })
        .await
        .unwrap();
    doc_id
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn open_doc_for_seeded_membership_succeeds_and_seqs_stay_per_doc() {
    let acc = signup_account().await;
    let second = seed_membership(&acc, "member").await;

    // Both docs on one connection: the primary gets one op first, so
    // a per-doc seq space is visible when the seeded doc starts at 1.
    let mut ws = connect_ws(&acc.server, &acc.device_token).await;
    handshake(&mut ws, acc.primary_doc_id).await;
    send_msgpack(
        &mut ws,
        &ClientFrame::PushUpdates {
            doc_id: acc.primary_doc_id,
            updates: vec![fake_push(1)],
        },
    )
    .await;
    let _ack: ServerFrame = recv_msgpack(&mut ws).await;

    let epoch = open_doc(&mut ws, second).await;
    assert_eq!(epoch, 0);
    send_msgpack(
        &mut ws,
        &ClientFrame::PushUpdates {
            doc_id: second,
            updates: vec![fake_push(2)],
        },
    )
    .await;
    match recv_msgpack::<ServerFrame>(&mut ws).await {
        ServerFrame::UpdatesAck { doc_id, .. } => assert_eq!(doc_id, second),
        other => panic!("expected UpdatesAck, got {other:?}"),
    }
    let batch = queries::fetch_updates_batch(&acc.server.state.db, second, 0)
        .await
        .unwrap();
    assert_eq!(batch.updates.len(), 1);
    assert_eq!(
        batch.updates[0].seq, 1,
        "second doc has its own seq counter"
    );

    // The horizon for the seeded doc is pinned by this account's device
    // via its membership, not via primary-doc ownership.
    let horizon = queries::doc_horizon(&acc.server.state.db, second, Uuid::nil(), 0)
        .await
        .unwrap();
    assert_eq!(horizon, 0, "device has acked nothing on the second doc");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn two_docs_open_on_one_connection_interleave_with_independent_seqs() {
    let acc = signup_account().await;
    let doc_a = acc.primary_doc_id;
    let doc_b = create_doc(&acc).await;

    let mut ws = connect_ws(&acc.server, &acc.device_token).await;
    handshake(&mut ws, doc_a).await;
    open_doc(&mut ws, doc_b).await;
    assert_eq!(acc.server.state.sync_sessions.subscriber_count(doc_a), 1);
    assert_eq!(acc.server.state.sync_sessions.subscriber_count(doc_b), 1);

    // A, B, A: every ack names its doc and each doc counts on its own.
    for (doc, seed, expected_seq) in [(doc_a, 1u8, 1u64), (doc_b, 2, 1), (doc_a, 3, 2)] {
        send_msgpack(
            &mut ws,
            &ClientFrame::PushUpdates {
                doc_id: doc,
                updates: vec![fake_push(seed)],
            },
        )
        .await;
        match recv_msgpack::<ServerFrame>(&mut ws).await {
            ServerFrame::UpdatesAck { doc_id, acks } => {
                assert_eq!(doc_id, doc);
                assert_eq!(acks.len(), 1);
                assert_eq!(acks[0].seq, expected_seq, "doc {doc}");
            }
            other => panic!("expected UpdatesAck, got {other:?}"),
        }
    }

    // Pull on the second-opened doc is served on the same connection.
    send_msgpack(
        &mut ws,
        &ClientFrame::PullUpdates {
            doc_id: doc_b,
            since_seq: 0,
        },
    )
    .await;
    match recv_msgpack::<ServerFrame>(&mut ws).await {
        ServerFrame::UpdatesBatch {
            doc_id,
            updates,
            complete,
        } => {
            assert_eq!(doc_id, doc_b);
            assert_eq!(updates.iter().map(|o| o.seq).collect::<Vec<_>>(), vec![1]);
            assert!(complete);
        }
        other => panic!("expected UpdatesBatch, got {other:?}"),
    }

    // Acks are per (device, doc).
    send_msgpack(
        &mut ws,
        &ClientFrame::Ack {
            doc_id: doc_b,
            last_acked_seq: 1,
        },
    )
    .await;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
    loop {
        let h_b = queries::doc_horizon(&acc.server.state.db, doc_b, Uuid::nil(), 0)
            .await
            .unwrap();
        if h_b == 1 {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "doc_b horizon stuck at {h_b}"
        );
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    let h_a = queries::doc_horizon(&acc.server.state.db, doc_a, Uuid::nil(), 0)
        .await
        .unwrap();
    assert_eq!(h_a, 0, "an ack on doc_b must not move doc_a's frontier");

    // Closing the connection drops both subscriptions.
    drop(ws);
    wait_for_subscribers(&acc, 0).await;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
    while acc.server.state.sync_sessions.subscriber_count(doc_b) != 0 {
        assert!(
            std::time::Instant::now() < deadline,
            "doc_b subscriber lingered"
        );
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn broadcasts_route_per_doc_on_a_multiplexed_connection() {
    // Connection 1 (device a) has A and B open. Connection 2 (device b)
    // has only A; connection 3 (device c) has only B. A push from 2
    // must reach 1 and not 3; a push from 3 must reach 1 and not 2 —
    // and 1 must see both, tagged with the right doc.
    let acc = signup_account().await;
    let doc_a = acc.primary_doc_id;
    let doc_b = create_doc(&acc).await;
    let device_b = register_second_device(&acc, "device-b").await;
    let device_c = register_second_device(&acc, "device-c").await;

    let mut ws1 = connect_ws(&acc.server, &acc.device_token).await;
    handshake(&mut ws1, doc_a).await;
    open_doc(&mut ws1, doc_b).await;
    let mut ws2 = connect_ws(&acc.server, &device_b.device_token).await;
    handshake(&mut ws2, doc_a).await;
    let mut ws3 = connect_ws(&acc.server, &device_c.device_token).await;
    handshake(&mut ws3, doc_b).await;
    assert_eq!(acc.server.state.sync_sessions.subscriber_count(doc_a), 2);
    assert_eq!(acc.server.state.sync_sessions.subscriber_count(doc_b), 2);

    send_msgpack(
        &mut ws2,
        &ClientFrame::PushUpdates {
            doc_id: doc_a,
            updates: vec![fake_push(0xA1)],
        },
    )
    .await;
    assert_eq!(ack_seqs(recv_msgpack(&mut ws2).await), vec![1]);
    match recv_msgpack::<ServerFrame>(&mut ws1).await {
        ServerFrame::UpdatesBroadcast { doc_id, updates } => {
            assert_eq!(doc_id, doc_a);
            assert_eq!(updates[0].blob, fake_blob(0xA1));
        }
        other => panic!("expected UpdatesBroadcast on doc_a, got {other:?}"),
    }
    assert_no_frame(&mut ws3, std::time::Duration::from_millis(200)).await;

    send_msgpack(
        &mut ws3,
        &ClientFrame::PushUpdates {
            doc_id: doc_b,
            updates: vec![fake_push(0xB1)],
        },
    )
    .await;
    assert_eq!(ack_seqs(recv_msgpack(&mut ws3).await), vec![1]);
    match recv_msgpack::<ServerFrame>(&mut ws1).await {
        ServerFrame::UpdatesBroadcast { doc_id, updates } => {
            assert_eq!(doc_id, doc_b);
            assert_eq!(updates[0].blob, fake_blob(0xB1));
        }
        other => panic!("expected UpdatesBroadcast on doc_b, got {other:?}"),
    }
    assert_no_frame(&mut ws2, std::time::Duration::from_millis(200)).await;

    // The multiplexed connection's own push on B is excluded from
    // itself and reaches only the B subscriber.
    send_msgpack(
        &mut ws1,
        &ClientFrame::PushUpdates {
            doc_id: doc_b,
            updates: vec![fake_push(0xB2)],
        },
    )
    .await;
    assert_eq!(ack_seqs(recv_msgpack(&mut ws1).await), vec![2]);
    match recv_msgpack::<ServerFrame>(&mut ws3).await {
        ServerFrame::UpdatesBroadcast { doc_id, updates } => {
            assert_eq!(doc_id, doc_b);
            assert_eq!(updates[0].seq, 2);
        }
        other => panic!("expected UpdatesBroadcast on doc_b, got {other:?}"),
    }
    assert_no_frame(&mut ws2, std::time::Duration::from_millis(200)).await;
    assert_no_frame(&mut ws1, std::time::Duration::from_millis(200)).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn snapshot_lease_on_any_open_doc_is_released_when_the_connection_drops() {
    // Long lease timeout, so only an explicit release can free it.
    // Connection 1 holds A and B open and earns a lease on B. When it
    // drops, device b acking on B must be issued a fresh request at
    // once — proof the lease was handed back for *every* open doc, not
    // just one of them.
    let acc = signup_account_with_snapshot_config(5, std::time::Duration::from_secs(30)).await;
    let doc_a = acc.primary_doc_id;
    let doc_b = create_doc(&acc).await;
    let device_b = register_second_device(&acc, "device-b").await;

    let mut ws1 = connect_ws(&acc.server, &acc.device_token).await;
    handshake(&mut ws1, doc_a).await;
    open_doc(&mut ws1, doc_b).await;
    send_msgpack(
        &mut ws1,
        &ClientFrame::PushUpdates {
            doc_id: doc_b,
            updates: fake_pushes(0..5),
        },
    )
    .await;
    let (up_to_seq, _, _) = expect_snapshot_request(&mut ws1).await;
    assert_eq!(up_to_seq, 5);
    send_msgpack(
        &mut ws1,
        &ClientFrame::Ack {
            doc_id: doc_b,
            last_acked_seq: 5,
        },
    )
    .await;
    assert_eq!(wait_for_acked_on(&acc, doc_b, 5).await, 5);

    drop(ws1);
    wait_for_subscribers(&acc, 0).await;

    let mut ws2 = connect_ws(&acc.server, &device_b.device_token).await;
    handshake(&mut ws2, doc_b).await;
    send_msgpack(
        &mut ws2,
        &ClientFrame::Ack {
            doc_id: doc_b,
            last_acked_seq: 5,
        },
    )
    .await;
    let (b_up_to, _, _) = expect_snapshot_request(&mut ws2).await;
    assert_eq!(b_up_to, 5);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn removed_membership_cannot_open_doc() {
    let acc = signup_account().await;
    let second = seed_membership(&acc, "member").await;
    let account_id = acc.account_id;
    acc.server
        .state
        .db
        .call(move |c| {
            c.execute(
                "UPDATE doc_members SET removed_at = 3 WHERE doc_id = ? AND account_id = ?",
                rusqlite::params![
                    second.as_bytes().as_slice(),
                    account_id.as_bytes().as_slice()
                ],
            )
        })
        .await
        .unwrap();
    let mut ws = connect_ws(&acc.server, &acc.device_token).await;
    hello(&mut ws).await;
    send_msgpack(&mut ws, &ClientFrame::OpenDoc { doc_id: second }).await;
    match recv_msgpack::<ServerFrame>(&mut ws).await {
        ServerFrame::DocRejected { doc_id, .. } => assert_eq!(doc_id, second),
        other => panic!("expected DocRejected, got {other:?}"),
    }
    expect_closed(&mut ws).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn payload_frame_before_open_doc_closes_session() {
    let acc = signup_account().await;
    let mut ws = connect_ws(&acc.server, &acc.device_token).await;
    hello(&mut ws).await;

    // Right doc, but never opened on this connection.
    send_msgpack(
        &mut ws,
        &ClientFrame::PushUpdates {
            doc_id: acc.primary_doc_id,
            updates: vec![fake_push(1)],
        },
    )
    .await;
    expect_closed(&mut ws).await;
    let batch = queries::fetch_updates_batch(&acc.server.state.db, acc.primary_doc_id, 0)
        .await
        .unwrap();
    assert!(
        batch.updates.is_empty(),
        "nothing may be inserted before OpenDoc"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn payload_frame_for_another_doc_closes_session() {
    let acc = signup_account().await;
    let mut ws = connect_ws(&acc.server, &acc.device_token).await;
    handshake(&mut ws, acc.primary_doc_id).await;
    wait_for_subscribers(&acc, 1).await;

    send_msgpack(
        &mut ws,
        &ClientFrame::PullUpdates {
            doc_id: Uuid::now_v7(),
            since_seq: 0,
        },
    )
    .await;
    expect_closed(&mut ws).await;
    wait_for_subscribers(&acc, 0).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn reopening_the_same_doc_is_idempotent() {
    let acc = signup_account().await;
    let mut ws = connect_ws(&acc.server, &acc.device_token).await;
    handshake(&mut ws, acc.primary_doc_id).await;
    send_msgpack(
        &mut ws,
        &ClientFrame::OpenDoc {
            doc_id: acc.primary_doc_id,
        },
    )
    .await;
    match recv_msgpack::<ServerFrame>(&mut ws).await {
        ServerFrame::DocOpened { doc_id, .. } => assert_eq!(doc_id, acc.primary_doc_id),
        other => panic!("expected DocOpened, got {other:?}"),
    }
    assert_eq!(
        acc.server
            .state
            .sync_sessions
            .subscriber_count(acc.primary_doc_id),
        1,
        "re-open must not double-subscribe"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn horizon_spans_device_doc_frontiers_and_drops_revoked_devices() {
    let acc = signup_account().await;
    let device_b = register_second_device(&acc, "device-b").await;
    let db = &acc.server.state.db;
    let doc = acc.primary_doc_id;

    let mut ws_a = connect_ws(&acc.server, &acc.device_token).await;
    handshake(&mut ws_a, doc).await;
    send_msgpack(
        &mut ws_a,
        &ClientFrame::PushUpdates {
            doc_id: doc,
            updates: fake_pushes(1..4),
        },
    )
    .await;
    let seqs = ack_seqs(recv_msgpack(&mut ws_a).await);
    assert_eq!(seqs, vec![1, 2, 3]);
    send_msgpack(
        &mut ws_a,
        &ClientFrame::Ack {
            doc_id: doc,
            last_acked_seq: 3,
        },
    )
    .await;
    assert_eq!(wait_for_acked(&acc, 3).await, 3);

    // B has never acked: no frontier row, counts as 0, pins the horizon.
    let horizon = queries::doc_horizon(db, doc, acc.device_id, 3)
        .await
        .unwrap();
    assert_eq!(horizon, 0);

    let mut ws_b = connect_ws(&acc.server, &device_b.device_token).await;
    handshake(&mut ws_b, doc).await;
    send_msgpack(
        &mut ws_b,
        &ClientFrame::Ack {
            doc_id: doc,
            last_acked_seq: 2,
        },
    )
    .await;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
    loop {
        let h = queries::doc_horizon(db, doc, acc.device_id, 3)
            .await
            .unwrap();
        if h == 2 {
            break;
        }
        assert!(std::time::Instant::now() < deadline, "horizon stuck at {h}");
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }

    // Revoking B cascades its frontier rows away, so only A remains.
    drop(ws_b);
    let resp = http()
        .delete(format!(
            "{}/api/devices/{}",
            acc.server.base, device_b.device_id
        ))
        .bearer_auth(&acc.device_token)
        .send()
        .await
        .unwrap();
    assert!(resp.status().is_success(), "revoke: {}", resp.status());
    let horizon = queries::doc_horizon(db, doc, acc.device_id, 3)
        .await
        .unwrap();
    assert_eq!(horizon, 3);
}

/// A second account on `acc`'s server, for cross-account tests.
async fn signup_second(acc: &Account) -> (Uuid, String) {
    let kdf_params = weak_params();
    let master_salt: [u8; 16] = random_bytes();
    let master =
        derive_password_master(b"another password entirely", &master_salt, kdf_params).unwrap();
    let kek = master.kek().unwrap();
    let dek = Dek::generate();
    let wrapped = kek.wrap(&dek).unwrap();
    let resp: SignupResponse = post_msgpack(
        &format!("{}/api/account/signup", acc.server.base),
        &SignupRequest {
            email: format!("second-{}@example.com", Uuid::now_v7()),
            master_salt: master_salt.to_vec(),
            kdf_params,
            auth_secret: master.auth_secret().unwrap().as_bytes().to_vec(),
            wrapped_dek: wrapped.ciphertext,
            wrapped_dek_nonce: wrapped.nonce.to_vec(),
            recovery: enrol_recovery(&dek, kdf_params),
            device_name: "second-device".into(),
            device_kind: "linux".into(),
            primary_doc_id: None,
        },
    )
    .await;
    (
        Uuid::parse_str(&resp.account_id).unwrap(),
        resp.device_token,
    )
}

/// Seed an active `member` row for `account_id` on `doc_id` directly
/// (the shape an accepted invite leaves behind).
async fn seed_member_row(acc: &Account, doc_id: Uuid, account_id: Uuid) {
    acc.server
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

/// Live revocation (`spec/sharing-plan.md` §Member removal): the owner
/// removes a member over HTTP while that member has the doc open. The
/// member's connection gets `DocClosed` for that doc and nothing else;
/// the rest of the connection (its Home doc) keeps working; the
/// owner's own open is untouched and its next push is not broadcast
/// to the removed member; and a later frame from the removed member
/// naming the doc ends its session like any frame for an unopened doc.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn removing_a_member_closes_the_doc_on_its_live_connection() {
    let owner = signup_account().await;
    let shared = create_doc(&owner).await;
    let (member_id, member_token) = signup_second(&owner).await;
    seed_member_row(&owner, shared, member_id).await;

    let mut ws_owner = connect_ws(&owner.server, &owner.device_token).await;
    handshake(&mut ws_owner, shared).await;
    let mut ws_member = connect_ws(&owner.server, &member_token).await;
    hello(&mut ws_member).await;
    open_doc(&mut ws_member, shared).await;
    // The member's Home is a different doc; it must survive the close.
    let member_home = {
        let state = owner.server.state.clone();
        let rows = monoplan_server::auth::queries::list_memberships(&state.db, member_id)
            .await
            .unwrap();
        rows.into_iter()
            .map(|m| m.doc_id)
            .find(|d| *d != shared)
            .expect("member has a Home doc")
    };
    open_doc(&mut ws_member, member_home).await;
    wait_for_subscribers_on(&owner, shared, 2).await;

    let status = http()
        .delete(format!(
            "{}/api/docs/{shared}/members/{member_id}",
            owner.server.base
        ))
        .bearer_auth(&owner.device_token)
        .send()
        .await
        .unwrap()
        .status();
    assert!(status.is_success(), "remove member: {status}");

    match recv_msgpack::<ServerFrame>(&mut ws_member).await {
        ServerFrame::DocClosed { doc_id, reason } => {
            assert_eq!(doc_id, shared);
            assert!(reason.contains("removed"), "reason: {reason}");
        }
        other => panic!("expected DocClosed, got {other:?}"),
    }
    assert_eq!(
        owner.server.state.sync_sessions.subscriber_count(shared),
        1,
        "only the owner's session stays subscribed"
    );

    // The owner's push is acked and reaches nobody else.
    send_msgpack(
        &mut ws_owner,
        &ClientFrame::PushUpdates {
            doc_id: shared,
            updates: vec![fake_push(1)],
        },
    )
    .await;
    assert_eq!(
        ack_seqs(recv_msgpack::<ServerFrame>(&mut ws_owner).await),
        vec![1]
    );
    assert_no_frame(&mut ws_member, std::time::Duration::from_millis(200)).await;

    // The member's other doc still syncs on the same connection.
    send_msgpack(
        &mut ws_member,
        &ClientFrame::PushUpdates {
            doc_id: member_home,
            updates: vec![fake_push(2)],
        },
    )
    .await;
    assert_eq!(
        ack_seqs(recv_msgpack::<ServerFrame>(&mut ws_member).await),
        vec![1]
    );

    // Naming the closed doc again ends the session, as for any doc
    // the connection never opened.
    send_msgpack(
        &mut ws_member,
        &ClientFrame::PullUpdates {
            doc_id: shared,
            since_seq: 0,
        },
    )
    .await;
    expect_closed(&mut ws_member).await;

    // And a reconnect is refused outright.
    let mut ws_again = connect_ws(&owner.server, &member_token).await;
    hello(&mut ws_again).await;
    send_msgpack(&mut ws_again, &ClientFrame::OpenDoc { doc_id: shared }).await;
    match recv_msgpack::<ServerFrame>(&mut ws_again).await {
        ServerFrame::DocRejected { doc_id, .. } => assert_eq!(doc_id, shared),
        other => panic!("expected DocRejected, got {other:?}"),
    }
    expect_closed(&mut ws_again).await;
}

/// Unilateral delete (`spec/sharing-plan.md` §Deletion): the owner
/// deletes a shared doc over HTTP while a member has it open. The
/// member's connection gets `DocClosed` for that doc and keeps serving
/// its Home; the owner's own connection, the one that asked, hears
/// nothing (it tears the doc down itself on the HTTP reply); nobody is
/// left subscribed; and the member's reconnect is refused.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn deleting_a_doc_closes_it_on_other_members_live_connections() {
    let owner = signup_account().await;
    let shared = create_doc(&owner).await;
    let (member_id, member_token) = signup_second(&owner).await;
    seed_member_row(&owner, shared, member_id).await;

    let mut ws_owner = connect_ws(&owner.server, &owner.device_token).await;
    handshake(&mut ws_owner, shared).await;
    let mut ws_member = connect_ws(&owner.server, &member_token).await;
    hello(&mut ws_member).await;
    open_doc(&mut ws_member, shared).await;
    let member_home = {
        let state = owner.server.state.clone();
        let rows = monoplan_server::auth::queries::list_memberships(&state.db, member_id)
            .await
            .unwrap();
        rows.into_iter()
            .map(|m| m.doc_id)
            .find(|d| *d != shared)
            .expect("member has a Home doc")
    };
    open_doc(&mut ws_member, member_home).await;
    wait_for_subscribers_on(&owner, shared, 2).await;

    let status = http()
        .delete(format!("{}/api/docs/{shared}", owner.server.base))
        .bearer_auth(&owner.device_token)
        .send()
        .await
        .unwrap()
        .status();
    assert!(status.is_success(), "delete doc: {status}");

    match recv_msgpack::<ServerFrame>(&mut ws_member).await {
        ServerFrame::DocClosed { doc_id, reason } => {
            assert_eq!(doc_id, shared);
            assert!(reason.contains("deleted"), "reason: {reason}");
        }
        other => panic!("expected DocClosed, got {other:?}"),
    }
    assert_no_frame(&mut ws_owner, std::time::Duration::from_millis(200)).await;
    assert_eq!(
        owner.server.state.sync_sessions.subscriber_count(shared),
        0,
        "nobody stays subscribed to a deleted doc"
    );

    // The member's Home still syncs on the same connection.
    send_msgpack(
        &mut ws_member,
        &ClientFrame::PushUpdates {
            doc_id: member_home,
            updates: vec![fake_push(2)],
        },
    )
    .await;
    assert_eq!(
        ack_seqs(recv_msgpack::<ServerFrame>(&mut ws_member).await),
        vec![1]
    );

    // A reconnect naming the deleted doc is refused outright.
    let mut ws_again = connect_ws(&owner.server, &member_token).await;
    hello(&mut ws_again).await;
    send_msgpack(&mut ws_again, &ClientFrame::OpenDoc { doc_id: shared }).await;
    match recv_msgpack::<ServerFrame>(&mut ws_again).await {
        ServerFrame::DocRejected { doc_id, .. } => assert_eq!(doc_id, shared),
        other => panic!("expected DocRejected, got {other:?}"),
    }
    expect_closed(&mut ws_again).await;
}
