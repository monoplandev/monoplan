//! WebSocket sync endpoint: handshake → open doc → push / pull / ack
//! loop with peer broadcast on push.
//!
//! Auth happens on the upgrade via the same bearer extractor used by
//! HTTP routes — token validation runs *before* the upgrade response
//! goes out, so an unauthorized client never gets a socket.
//!
//! The connection is `(account, device)`-bound; docs are bound per
//! frame. After the version handshake the client sends `OpenDoc` for
//! the doc it wants to sync, the server checks access and subscribes
//! the connection to that doc's broadcasts, and every payload frame
//! after that names the doc it is about. One doc per connection
//! today (an account has exactly one); the per-frame `doc_id` and the
//! open step are the seams multi-doc multiplexing slots into
//! (`spec/sharing-plan.md`).
use std::time::Instant;

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Extension, Query, State};
use axum::http::{Extensions, HeaderMap};
use axum::response::Response;
use monoplan_protocol::{
    ClientFrame, EncryptedBlob, Hello, HelloAck, HelloRejected, PROTOCOL_VERSION, PushBlob,
    ServerFrame,
};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use tracing::Instrument;
use uuid::Uuid;

use crate::auth::DeviceAuth;
use crate::auth::client_ip::{client_ip, peer_ip};
use crate::auth::cookie;
use crate::auth::queries::{
    self as queries_auth, find_device_by_token_hash, touch_device_last_seen,
};
use crate::auth::tokens::{decode_token, sha256};
use crate::error::ApiError;
use crate::http::request_id::RequestId;
use crate::state::AppState;
use crate::sync::sessions::Subscription;
use crate::sync::snapshot::{Decision, PrevSnapshot, ReleaseResult};

use super::queries;

/// Server's reported version on `HelloAck`. Bumped manually for now;
/// when there's a build-info crate we'll plumb `CARGO_PKG_VERSION`.
const SERVER_VERSION: &str = env!("CARGO_PKG_VERSION");

/// `?token=...` is a non-browser fallback (e.g. test clients that can't
/// set headers on WS upgrades). Browsers use the `monoplan_device` cookie,
/// which the user-agent attaches to the upgrade request automatically;
/// CLI uses the `Authorization` header.
#[derive(Deserialize)]
pub struct WsAuthQuery {
    token: Option<String>,
}

/// A doc bound to this connection by `OpenDoc`.
struct OpenDoc {
    doc_id: Uuid,
    sub: Subscription,
    snapshot_lease: Option<u64>,
}

struct WSSession {
    auth: DeviceAuth,
    /// The doc this connection has opened, if any. Payload frames are
    /// refused until one is open, and must name this doc.
    open: Option<OpenDoc>,
}

impl WSSession {
    /// The open doc a payload frame for `doc_id` may act on.
    fn open_doc(&mut self, doc_id: Uuid) -> Result<&mut OpenDoc, SessionError> {
        match self.open.as_mut() {
            Some(open) if open.doc_id == doc_id => Ok(open),
            Some(open) => Err(SessionError::DocNotOpen {
                doc_id,
                open: Some(open.doc_id),
            }),
            None => Err(SessionError::DocNotOpen { doc_id, open: None }),
        }
    }
}

/// May this device open `doc_id`? An active `doc_members` row for the
/// device's account is the one and only access decision; keep every
/// such check behind this function.
async fn can_open(state: &AppState, auth: &DeviceAuth, doc_id: Uuid) -> anyhow::Result<bool> {
    queries_auth::is_member(&state.db, doc_id, auth.account_id).await
}

pub async fn ws_handler(
    ws: WebSocketUpgrade,
    State(state): State<AppState>,
    Query(q): Query<WsAuthQuery>,
    headers: HeaderMap,
    Extension(request_id): Extension<RequestId>,
    extensions: Extensions,
) -> Result<Response, ApiError> {
    // Preference order: bearer header (CLI) → cookie (web) → query param
    // (test fallback). All three reach the same hash lookup.
    let bearer = headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.strip_prefix("Bearer "))
        .map(str::to_owned);
    let cookie_token = cookie::token_from_cookies(&headers).map(str::to_owned);
    let raw_hex = bearer
        .or(cookie_token)
        .or(q.token)
        .ok_or(ApiError::Unauthorized)?;
    let raw = decode_token(&raw_hex).ok_or(ApiError::Unauthorized)?;
    let hash = sha256(&raw).to_vec();
    let lookup = find_device_by_token_hash(&state.db, hash)
        .await?
        .ok_or(ApiError::Unauthorized)?;
    let ip = client_ip(&headers, peer_ip(&extensions));
    let _ = touch_device_last_seen(&state.db, lookup.device_id, ip).await;
    let auth = DeviceAuth {
        account_id: lookup.account_id,
        device_id: lookup.device_id,
        primary_doc_id: lookup.primary_doc_id,
    };
    let session_id = Uuid::now_v7().to_string();
    let upgrade_request_id = request_id.0;
    Ok(ws.on_upgrade(move |socket| async move {
        let span = tracing::info_span!(
            "ws.session",
            session_id = %session_id,
            upgrade_request_id = %upgrade_request_id,
            account_id = %auth.account_id,
            device_id = %auth.device_id,
        );
        async move {
            tracing::info!("ws session started");
            match run_session(socket, state, auth).await {
                Ok(()) => tracing::info!("ws session closed"),
                Err(SessionError::Closed) => tracing::info!("ws session closed"),
                Err(e) => tracing::warn!(error = %e, "ws session ended with error"),
            }
        }
        .instrument(span)
        .await
    }))
}

#[derive(Debug, thiserror::Error)]
enum SessionError {
    #[error("connection closed")]
    Closed,
    #[error("ws error: {0}")]
    Ws(#[from] axum::Error),
    #[error("expected binary frame, got text")]
    UnexpectedTextFrame,
    #[error("decode: {0}")]
    Decode(#[from] rmp_serde::decode::Error),
    #[error("encode: {0}")]
    Encode(#[from] rmp_serde::encode::Error),
    #[error(transparent)]
    Sql(#[from] anyhow::Error),
    #[error("handshake failed: {0}")]
    Handshake(String),
    #[error("open doc {doc_id} refused: {reason}")]
    OpenRefused { doc_id: Uuid, reason: String },
    #[error("payload frame for doc {doc_id} but open doc is {open:?}")]
    DocNotOpen { doc_id: Uuid, open: Option<Uuid> },
    #[error("broadcast channel dropped — slow client")]
    BroadcastLagged,
}

async fn run_session(
    mut socket: WebSocket,
    state: AppState,
    auth: DeviceAuth,
) -> Result<(), SessionError> {
    let handshake_span = tracing::info_span!("ws.handshake");
    let _hello: Hello = async {
        let hello: Hello = recv_msgpack(&mut socket).await?;
        if !hello
            .supported_protocol_versions
            .contains(&PROTOCOL_VERSION)
        {
            let _ = send_msgpack(
                &mut socket,
                &HelloRejected {
                    reason: format!(
                        "no shared protocol version (server speaks {PROTOCOL_VERSION})"
                    ),
                },
            )
            .await;
            tracing::warn!(
                offered_versions = ?hello.supported_protocol_versions,
                protocol_version = PROTOCOL_VERSION,
                "ws handshake rejected"
            );
            return Err(SessionError::Handshake(format!(
                "client offered {:?}, server speaks {}",
                hello.supported_protocol_versions, PROTOCOL_VERSION
            )));
        }
        send_msgpack(
            &mut socket,
            &HelloAck {
                server_version: SERVER_VERSION.into(),
                protocol_version: PROTOCOL_VERSION,
            },
        )
        .await?;
        tracing::info!(protocol_version = PROTOCOL_VERSION, "ws handshake accepted");
        Ok(hello)
    }
    .instrument(handshake_span)
    .await?;

    let mut ws_session = WSSession { auth, open: None };

    let result = loop {
        tokio::select! {
            client_frame = recv_msgpack::<ClientFrame>(&mut socket) => {
                match client_frame {
                    Ok(frame) => handle_frame(&mut socket, &state, &mut ws_session, frame).await?,
                    Err(SessionError::Closed) => break Ok(()),
                    Err(e) => break Err(e),
                }
            }
            // No broadcasts flow until a doc is open: the branch
            // parks on a never-ready future rather than polling a
            // subscription that doesn't exist yet.
            broadcast = async {
                match ws_session.open.as_mut() {
                    Some(open) => open.sub.rx.recv().await,
                    None => std::future::pending().await,
                }
            } => {
                let bytes = broadcast.ok_or(SessionError::BroadcastLagged)?;
                let broadcast_span = tracing::info_span!(
                    "ws.broadcast_send",
                    message_bytes = bytes.len()
                );
                socket.send(Message::Binary(bytes.into())).await?;
                tracing::debug!(parent: &broadcast_span, "ws broadcast delivered");
            }
        }
    };
    // Dropping the open doc's subscription deregisters the subscriber;
    // an unfinished snapshot lease goes back to the coordinator.
    if let Some(open) = ws_session.open.take()
        && let Some(lease_id) = open.snapshot_lease
    {
        state.snapshot_coordinator.release(open.doc_id, lease_id);
    }
    result
}

async fn handle_frame(
    socket: &mut WebSocket,
    state: &AppState,
    ws_session: &mut WSSession,
    frame: ClientFrame,
) -> Result<(), SessionError> {
    match frame {
        ClientFrame::OpenDoc { doc_id } => open_doc(socket, state, ws_session, doc_id).await,
        ClientFrame::PushOps { doc_id, ops } => {
            push_ops(socket, state, ws_session, doc_id, ops).await
        }
        ClientFrame::PullOps { doc_id, since_seq } => {
            ws_session.open_doc(doc_id)?;
            pull_ops(socket, state, doc_id, since_seq).await
        }
        ClientFrame::Ack {
            doc_id,
            last_acked_seq,
        } => {
            ws_session.open_doc(doc_id)?;
            let ack_span = tracing::info_span!("ws.ack", last_acked_seq = last_acked_seq);
            queries::advance_last_acked_seq(
                &state.db,
                ws_session.auth.device_id,
                doc_id,
                last_acked_seq,
            )
            .await?;
            evaluate_snapshot(socket, state, ws_session, doc_id, last_acked_seq).await?;
            tracing::debug!(parent: &ack_span, "ws ack applied");
            Ok(())
        }
        ClientFrame::PullSnapshot { doc_id } => {
            ws_session.open_doc(doc_id)?;
            pull_snapshot(socket, state, doc_id).await
        }
        ClientFrame::PushSnapshot {
            doc_id,
            up_to_seq,
            compaction_floor_seq,
            from_seq,
            up_to_vv,
            blob,
        } => {
            push_snapshot(
                state,
                ws_session,
                doc_id,
                up_to_seq,
                compaction_floor_seq,
                from_seq,
                up_to_vv,
                blob,
            )
            .await
        }
    }
}

/// Bind `doc_id` to the connection: access check, broadcast
/// subscription, `DocOpened`. A refusal sends `DocRejected` and then
/// ends the session — there is nothing else a client can usefully do
/// on this connection, and a loud close beats a silent ignore.
async fn open_doc(
    socket: &mut WebSocket,
    state: &AppState,
    ws_session: &mut WSSession,
    doc_id: Uuid,
) -> Result<(), SessionError> {
    let span = tracing::info_span!("ws.open_doc", doc_id = %doc_id);
    async {
        let refusal = if !can_open(state, &ws_session.auth, doc_id).await? {
            Some("not a member of this doc")
        } else if ws_session.open.as_ref().is_some_and(|o| o.doc_id != doc_id) {
            // One doc per connection until multiplexing lands; make
            // the limit explicit rather than silently swapping docs.
            Some("connection already has a different doc open")
        } else {
            None
        };
        if let Some(reason) = refusal {
            tracing::warn!(reason, "ws open_doc refused");
            let _ = send_msgpack(
                socket,
                &ServerFrame::DocRejected {
                    doc_id,
                    reason: reason.into(),
                },
            )
            .await;
            return Err(SessionError::OpenRefused {
                doc_id,
                reason: reason.into(),
            });
        }
        let epoch = queries::doc_epoch(&state.db, doc_id).await?;
        if ws_session.open.is_none() {
            // Subscribe *before* replying: once the client sees
            // `DocOpened` it may rely on broadcasts for everything
            // after its pull, so there must be no window in which a
            // peer's push could slip past unregistered.
            let sub = state
                .sync_sessions
                .subscribe(doc_id, ws_session.auth.device_id);
            ws_session.open = Some(OpenDoc {
                doc_id,
                sub,
                snapshot_lease: None,
            });
        }
        send_msgpack(socket, &ServerFrame::DocOpened { doc_id, epoch }).await?;
        tracing::info!(epoch, "ws doc opened");
        Ok(())
    }
    .instrument(span)
    .await
}

/// Read snapshot/horizon state, compute the decision. Does no sends —
/// caller is responsible for emitting `SnapshotRequest` when the
/// decision is `Issue`. Separating compute from send lets the push
/// path interleave `OpsAck` before `SnapshotRequest` (clients use
/// `OpsAck` to advance `last_contiguous_seq` before producing the
/// snapshot — otherwise the snapshot stamps a stale frontier).
async fn evaluate_snapshot_decision(
    state: &AppState,
    ws_session: &WSSession,
    doc_id: Uuid,
    device_last_acked_seq: u64,
) -> Result<Decision, SessionError> {
    let prev = queries::latest_snapshot_meta(&state.db, doc_id)
        .await?
        .map(|m| PrevSnapshot {
            up_to_seq: m.up_to_seq,
            compaction_floor_seq: m.compaction_floor_seq,
            up_to_vv: m.up_to_vv,
        });
    let server_last_seq = queries::latest_doc_seq(&state.db, doc_id).await?;
    let horizon = queries::doc_horizon(
        &state.db,
        doc_id,
        ws_session.auth.device_id,
        device_last_acked_seq,
    )
    .await?;
    Ok(state.snapshot_coordinator.evaluate(
        doc_id,
        prev.as_ref(),
        server_last_seq,
        horizon,
        device_last_acked_seq,
        Instant::now(),
    ))
}

async fn issue_snapshot_request(
    socket: &mut WebSocket,
    ws_session: &mut WSSession,
    doc_id: Uuid,
    decision: Decision,
) -> Result<(), SessionError> {
    if let Decision::Issue {
        lease_id,
        up_to_seq,
        compaction_floor_seq,
        base,
    } = decision
    {
        ws_session.open_doc(doc_id)?.snapshot_lease = Some(lease_id);
        let from_seq = base.as_ref().map(|b| b.seq);
        send_msgpack(
            socket,
            &ServerFrame::SnapshotRequest {
                doc_id,
                up_to_seq,
                compaction_floor_seq,
                base,
            },
        )
        .await?;
        tracing::info!(
            lease_id,
            up_to_seq,
            compaction_floor_seq,
            from_seq,
            "ws snapshot requested"
        );
    }
    Ok(())
}

async fn evaluate_snapshot(
    socket: &mut WebSocket,
    state: &AppState,
    ws_session: &mut WSSession,
    doc_id: Uuid,
    device_last_acked_seq: u64,
) -> Result<(), SessionError> {
    let decision =
        evaluate_snapshot_decision(state, ws_session, doc_id, device_last_acked_seq).await?;
    issue_snapshot_request(socket, ws_session, doc_id, decision).await
}

async fn push_ops(
    socket: &mut WebSocket,
    state: &AppState,
    ws_session: &mut WSSession,
    doc_id: Uuid,
    ops: Vec<PushBlob>,
) -> Result<(), SessionError> {
    let sub_id = ws_session.open_doc(doc_id)?.sub.sub_id();
    let push_span = tracing::info_span!("ws.push_ops", op_count = ops.len());
    if ops.is_empty() {
        return async {
            tracing::debug!("ws push_ops empty");
            send_msgpack(
                socket,
                &ServerFrame::OpsAck {
                    doc_id,
                    acks: Vec::new(),
                },
            )
            .await
        }
        .instrument(push_span)
        .await;
    }

    async {
        // Insert with (device, push_id) dedup: a crash-retry of an
        // already-inserted push acks its original seq without a second
        // row.
        let inserted =
            queries::insert_ops(&state.db, doc_id, ws_session.auth.device_id, ops).await?;

        // Post-commit fan-out, before acking the originator — but only
        // the FRESH rows: a deduplicated retry was already broadcast on
        // its original insert. Both branches are correct (the
        // originator's ack and peer broadcasts are independent), but
        // queueing first means peers don't race the originator's "I'm
        // done pushing" signal.
        let fresh_count = inserted.fresh.len();
        if fresh_count > 0 {
            state
                .sync_sessions
                .broadcast(doc_id, sub_id, inserted.fresh);
        }

        tracing::info!(
            ack_count = inserted.acks.len(),
            fresh_count = fresh_count,
            first_seq = inserted.acks.first().map(|a| a.seq),
            last_seq = inserted.acks.last().map(|a| a.seq),
            "ws push_ops persisted"
        );

        // Order matters: compute the snapshot decision (DB reads only)
        // BEFORE sending OpsAck, then send OpsAck BEFORE
        // SnapshotRequest. The client uses OpsAck to advance
        // `last_contiguous_seq` — and that's what gets stamped as the
        // snapshot's `up_to_seq` when it produces. If SnapshotRequest
        // arrives first, the client snapshots at its pre-push frontier,
        // landing a snapshot with `up_to < compaction_floor`, which
        // traps later bootstrappers in an infinite SnapshotRequired loop.
        let snapshot_decision = if let Some(latest_seq) = inserted.acks.last().map(|a| a.seq) {
            Some(evaluate_snapshot_decision(state, ws_session, doc_id, latest_seq).await?)
        } else {
            None
        };

        send_msgpack(
            socket,
            &ServerFrame::OpsAck {
                doc_id,
                acks: inserted.acks,
            },
        )
        .await?;

        if let Some(decision) = snapshot_decision {
            issue_snapshot_request(socket, ws_session, doc_id, decision).await?;
        }

        Ok(())
    }
    .instrument(push_span)
    .await
}

async fn pull_ops(
    socket: &mut WebSocket,
    state: &AppState,
    doc_id: Uuid,
    since_seq: u64,
) -> Result<(), SessionError> {
    let pull_span = tracing::info_span!("ws.pull_ops", since_seq = since_seq);
    // Bootstrap-from-snapshot precondition: if the latest snapshot's
    // `up_to_seq` is ahead of the client's cursor, the client cannot
    // resume from ops alone (compacted ops below the floor are gone,
    // and even when they aren't, replaying every op since 0 wastes
    // round trips). Hand back `SnapshotRequired` and let the client
    // drive the bootstrap exchange.
    async {
        if let Some(meta) = queries::latest_snapshot_meta(&state.db, doc_id).await? {
            // Compaction floor (= snapshot's compaction_floor_seq) is
            // what determines whether the ops are still available.
            // Devices between compaction_floor and up_to can still
            // delta-pull; only those below the floor need bootstrap.
            if since_seq < meta.compaction_floor_seq {
                tracing::info!(
                    compaction_floor_seq = meta.compaction_floor_seq,
                    snapshot_up_to_seq = meta.up_to_seq,
                    "ws pull_ops below compaction floor; sending SnapshotRequired"
                );
                send_msgpack(
                    socket,
                    &ServerFrame::SnapshotRequired {
                        doc_id,
                        up_to_seq: meta.up_to_seq,
                    },
                )
                .await?;
                return Ok(());
            }
        }

        let mut cursor = since_seq;
        let mut batch_count = 0u64;
        let mut total_ops = 0usize;
        loop {
            let batch = queries::fetch_ops_batch(&state.db, doc_id, cursor).await?;
            let batch_len = batch.ops.len();
            let next_cursor = batch.ops.last().map(|o| o.seq).unwrap_or(cursor);
            let complete = !batch.has_more;
            send_msgpack(
                socket,
                &ServerFrame::OpsBatch {
                    doc_id,
                    ops: batch.ops,
                    complete,
                },
            )
            .await?;
            batch_count += 1;
            total_ops += batch_len;
            if complete {
                tracing::info!(
                    batch_count = batch_count,
                    op_count = total_ops,
                    final_seq = next_cursor,
                    "ws pull_ops completed"
                );
                return Ok(());
            }
            cursor = next_cursor;
        }
    }
    .instrument(pull_span)
    .await
}

#[allow(clippy::too_many_arguments)]
async fn push_snapshot(
    state: &AppState,
    ws_session: &mut WSSession,
    doc_id: Uuid,
    up_to_seq: u64,
    compaction_floor_seq: u64,
    from_seq: Option<u64>,
    up_to_vv: EncryptedBlob,
    blob: EncryptedBlob,
) -> Result<(), SessionError> {
    let open = ws_session.open_doc(doc_id)?;
    let span = tracing::info_span!(
        "ws.push_snapshot",
        up_to_seq = up_to_seq,
        compaction_floor_seq = compaction_floor_seq,
        from_seq = from_seq,
        blob_bytes = blob.ciphertext.len(),
    );
    async {
        let Some(snapshot_lease) = open.snapshot_lease else {
            tracing::warn!("no snapshot lease found");
            return Ok(());
        };
        let result = state.snapshot_coordinator.release(doc_id, snapshot_lease);
        if let ReleaseResult::Stale = result {
            tracing::warn!("ignoring unsolicited or stale PushSnapshot");
            return Ok(());
        }
        open.snapshot_lease = None;
        // One lease per doc means at most one producer reaches this
        // insert at a time; the chain check inside `insert_snapshot`
        // is the backstop for a delta that no longer links to the
        // latest row (it is rejected, nothing is written, and the
        // next trigger re-evaluates from the stored state).
        match queries::insert_snapshot(
            &state.db,
            doc_id,
            up_to_seq,
            compaction_floor_seq,
            from_seq,
            up_to_vv,
            blob,
        )
        .await?
        {
            queries::InsertSnapshot::Inserted { id } => {
                tracing::info!(snapshot_row_id = id, "ws push_snapshot persisted");
            }
            queries::InsertSnapshot::ChainMismatch { latest_up_to_seq } => {
                tracing::warn!(
                    latest_up_to_seq,
                    "ignoring delta snapshot that does not chain onto the latest row"
                );
                return Ok(());
            }
        }

        // Opportunistic compaction: a snapshot just landed, so the
        // floor moved (or stayed put, in which case the delete is a
        // no-op). Spawned so the WS task keeps reading frames; the
        // compaction itself is one tx, so a second snapshot landing
        // before this runs just shifts the floor we'll read.
        let db = state.db.clone();
        let compact_span = tracing::info_span!(
            "ws.push_snapshot.compact",
            doc_id = %doc_id,
        );
        tokio::spawn(
            async move {
                match queries::compact_doc(&db, doc_id).await {
                    Ok(stats) => {
                        tracing::info!(ops_deleted = stats.ops_deleted, "compaction completed")
                    }
                    Err(e) => tracing::warn!(error = %e, "compaction failed"),
                }
            }
            .instrument(compact_span),
        );

        // Fire-and-forget per spec — no `OpsAck`-style reply. The
        // orchestrator (when wired) tracks completion via the row
        // landing in the snapshots table, not via a wire ack.
        Ok(())
    }
    .instrument(span)
    .await
}

async fn pull_snapshot(
    socket: &mut WebSocket,
    state: &AppState,
    doc_id: Uuid,
) -> Result<(), SessionError> {
    let span = tracing::info_span!("ws.pull_snapshot");
    async {
        match queries::snapshot_chain(&state.db, doc_id).await? {
            Some(chain) => {
                tracing::info!(
                    up_to_seq = chain.up_to_seq,
                    row_count = chain.rows.len(),
                    "ws pull_snapshot served"
                );
                // One row per frame, fire-and-forget like `OpsBatch`:
                // the head can be tens of MiB and the client imports
                // each row as it lands, so never pack the chain into a
                // single message. TCP flow control paces a slow reader.
                let last = chain.rows.len().saturating_sub(1);
                for (i, row) in chain.rows.into_iter().enumerate() {
                    send_msgpack(
                        socket,
                        &ServerFrame::Snapshot {
                            doc_id,
                            up_to_seq: row.up_to_seq,
                            blob: row.blob,
                            complete: i == last,
                        },
                    )
                    .await?;
                }
                Ok(())
            }
            None => {
                // A well-behaved client only sends `PullSnapshot` after
                // receiving `SnapshotRequired`, which is only emitted
                // when a snapshot exists. If we get here, either the
                // snapshot was deleted out from under us or the client
                // is misbehaving — log and drop the request rather
                // than tearing down the session.
                tracing::warn!("PullSnapshot but no snapshot exists; ignoring");
                Ok(())
            }
        }
    }
    .instrument(span)
    .await
}

async fn recv_msgpack<T: DeserializeOwned>(socket: &mut WebSocket) -> Result<T, SessionError> {
    loop {
        let msg = socket
            .recv()
            .await
            .ok_or(SessionError::Closed)?
            .map_err(SessionError::Ws)?;
        return match msg {
            Message::Binary(bytes) => Ok(rmp_serde::from_slice(&bytes)?),
            Message::Close(_) => Err(SessionError::Closed),
            // Pings are answered by axum transparently; we just keep
            // looping until a payload arrives.
            Message::Ping(_) | Message::Pong(_) => continue,
            Message::Text(_) => Err(SessionError::UnexpectedTextFrame),
        };
    }
}

async fn send_msgpack<T: Serialize>(socket: &mut WebSocket, value: &T) -> Result<(), SessionError> {
    let bytes = rmp_serde::to_vec_named(value)?;
    socket.send(Message::Binary(bytes.into())).await?;
    Ok(())
}
