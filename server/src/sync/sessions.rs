//! In-memory subscriber registry for op broadcast.
//!
//! Each connected sync session subscribes against the doc it's synchronising
//! (today: always the device's account's primary doc; tomorrow with sharing:
//! any doc the device is a member of). Pushes from one device fan out to
//! peer sessions subscribed to the same doc. The fan-out delivers
//! pre-encoded msgpack bytes so we encode once per push, not once per peer.
//!
//! **Backpressure policy.** Each subscriber gets a bounded mpsc
//! (`CHANNEL_CAPACITY`). On a full or closed channel the broadcast
//! drops the message *and* removes the subscriber. The session task
//! observes its rx going `None` and exits; the client reconnects and
//! pulls from `last_acked_seq`. The op stream is the durable
//! channel — broadcast is just an optimization to skip the round-trip
//! when peers are live, so dropping a frame is safe.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use monoplan_protocol::{ServerFrame, StoredBlob};
use tokio::sync::mpsc;
use uuid::Uuid;

/// Per-subscriber buffer. 256 frames × ~256 KiB worst case = ~64 MiB
/// upper bound on a single hung subscriber. Sized for two-device
/// usage; tune up when we bump device count or down when we add real
/// memory accounting.
const CHANNEL_CAPACITY: usize = 256;

#[derive(Clone, Default)]
pub struct SyncSessions {
    inner: Arc<Mutex<Inner>>,
}

#[derive(Default)]
struct Inner {
    docs: HashMap<Uuid, Vec<Subscriber>>,
}

struct Subscriber {
    sub_id: u64,
    device_id: Uuid,
    account_id: Uuid,
    tx: mpsc::Sender<Outbound>,
}

/// One message on a subscriber's channel: pre-encoded frame bytes plus
/// what the session loop must do after forwarding them.
pub enum Outbound {
    /// An `OpsBroadcast`: send and carry on.
    Frame(Vec<u8>),
    /// A `DocClosed`: send, then unbind the doc from the connection
    /// (drop the subscription, release any snapshot lease). The
    /// session loop must not serve another frame for the doc.
    Close(Vec<u8>),
    /// Unbind the doc as for `Close`, but send nothing: the device on
    /// this connection is the one that deleted the doc and tears it
    /// down itself on the HTTP reply. Dropping its subscriber silently
    /// is not an option: a closed channel reads as lag and ends the
    /// whole session.
    Unbind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ConnectedSubscriber {
    pub sub_id: u64,
    pub device_id: Uuid,
}

/// RAII subscription handle. Dropping deregisters the subscriber.
pub struct Subscription {
    sessions: SyncSessions,
    doc_id: Uuid,
    sub_id: u64,
    pub rx: mpsc::Receiver<Outbound>,
}

impl Subscription {
    /// Per-connection subscriber id. Pass to `broadcast` so a push from
    /// this session is excluded only from *its own* WS, not from peer
    /// tabs that happen to share the same `device_id`.
    pub fn sub_id(&self) -> u64 {
        self.sub_id
    }
}

impl Drop for Subscription {
    fn drop(&mut self) {
        self.sessions.unsubscribe(self.doc_id, self.sub_id);
    }
}

impl SyncSessions {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn subscribe(&self, doc_id: Uuid, device_id: Uuid, account_id: Uuid) -> Subscription {
        let (tx, rx) = mpsc::channel(CHANNEL_CAPACITY);
        let sub_id = next_sub_id();
        let mut inner = self.inner.lock().unwrap();
        inner.docs.entry(doc_id).or_default().push(Subscriber {
            sub_id,
            device_id,
            account_id,
            tx,
        });
        Subscription {
            sessions: self.clone(),
            doc_id,
            sub_id,
            rx,
        }
    }

    fn unsubscribe(&self, doc_id: Uuid, sub_id: u64) {
        let mut inner = self.inner.lock().unwrap();
        if let Some(subs) = inner.docs.get_mut(&doc_id) {
            subs.retain(|s| s.sub_id != sub_id);
            if subs.is_empty() {
                inner.docs.remove(&doc_id);
            }
        }
    }

    /// Fan out an op set to every subscriber on `doc_id` except
    /// `exclude_sub`. Excluding by `sub_id` rather than `device_id`
    /// matters for multi-tab on the same device — both tabs share the
    /// device cookie, so a `device_id` filter would silence tab-to-tab
    /// broadcast on the same device. Each WS connection still gets a
    /// fresh `sub_id` so the originating tab is excluded but its peer
    /// tabs receive the frame.
    ///
    /// Returns the number of subscribers that received it (slow /
    /// closed receivers are dropped from the registry as a side
    /// effect).
    pub fn broadcast(&self, doc_id: Uuid, exclude_sub: u64, ops: Vec<StoredBlob>) -> usize {
        if ops.is_empty() {
            return 0;
        }
        let bytes = match rmp_serde::to_vec_named(&ServerFrame::OpsBroadcast { doc_id, ops }) {
            Ok(b) => b,
            Err(e) => {
                tracing::error!(error = %e, "failed to encode OpsBroadcast");
                return 0;
            }
        };
        let mut delivered = 0;
        let mut inner = self.inner.lock().unwrap();
        if let Some(subs) = inner.docs.get_mut(&doc_id) {
            subs.retain(|s| {
                if s.sub_id == exclude_sub {
                    return true;
                }
                match s.tx.try_send(Outbound::Frame(bytes.clone())) {
                    Ok(()) => {
                        delivered += 1;
                        true
                    }
                    // Channel full or closed: drop the subscriber. The
                    // peer's session loop sees `rx` go `None` and the
                    // client reconnects + pulls.
                    Err(_) => false,
                }
            });
        }
        delivered
    }

    /// Live revocation (`spec/sharing-plan.md` §Member removal): tell
    /// every connection of `account_id` that has `doc_id` open to drop
    /// it. Each such session forwards a `DocClosed` and unbinds the doc;
    /// the subscriber is removed from the registry here regardless, so
    /// no later broadcast on the doc reaches it even if the session is
    /// slow to act. A full or closed channel is handled as in
    /// `broadcast`: the session ends and the client's reconnect finds
    /// its `OpenDoc` refused. Returns the number of sessions told.
    pub fn close_for_account(&self, doc_id: Uuid, account_id: Uuid, reason: &str) -> usize {
        let bytes = match rmp_serde::to_vec_named(&ServerFrame::DocClosed {
            doc_id,
            reason: reason.into(),
        }) {
            Ok(b) => b,
            Err(e) => {
                tracing::error!(error = %e, "failed to encode DocClosed");
                return 0;
            }
        };
        let mut told = 0;
        let mut inner = self.inner.lock().unwrap();
        if let Some(subs) = inner.docs.get_mut(&doc_id) {
            subs.retain(|s| {
                if s.account_id != account_id {
                    return true;
                }
                if s.tx.try_send(Outbound::Close(bytes.clone())).is_ok() {
                    told += 1;
                }
                false
            });
            if subs.is_empty() {
                inner.docs.remove(&doc_id);
            }
        }
        told
    }

    /// The doc is gone (`DELETE /api/docs/:id`): drop every subscriber on
    /// it and send each a `DocClosed`, except the sessions of
    /// `except_device`, the device that asked for the delete. That device
    /// tears the doc down itself once the HTTP call returns; a `DocClosed`
    /// racing that teardown would make it forget the doc twice and show a
    /// "closed by the server" notice for its own action. Its sessions get
    /// a silent `Unbind` instead, so the doc still leaves the connection.
    /// Delivery semantics as in `close_for_account`. Returns the number
    /// sent a `DocClosed`.
    pub fn close_doc(&self, doc_id: Uuid, except_device: Option<Uuid>, reason: &str) -> usize {
        let bytes = match rmp_serde::to_vec_named(&ServerFrame::DocClosed {
            doc_id,
            reason: reason.into(),
        }) {
            Ok(b) => b,
            Err(e) => {
                tracing::error!(error = %e, "failed to encode DocClosed");
                return 0;
            }
        };
        let mut told = 0;
        let mut inner = self.inner.lock().unwrap();
        if let Some(subs) = inner.docs.remove(&doc_id) {
            for s in subs {
                if Some(s.device_id) == except_device {
                    let _ = s.tx.try_send(Outbound::Unbind);
                    continue;
                }
                if s.tx.try_send(Outbound::Close(bytes.clone())).is_ok() {
                    told += 1;
                }
            }
        }
        told
    }

    /// Snapshot the current subscriber ids for a doc. Used by
    /// tests (and the future orchestrator) as input to candidate
    /// selection. Order is not stable — caller must apply its own
    /// ordering policy.
    pub fn subscriber_ids(&self, doc_id: Uuid) -> Vec<u64> {
        let inner = self.inner.lock().unwrap();
        inner
            .docs
            .get(&doc_id)
            .map(|s| s.iter().map(|sub| sub.sub_id).collect())
            .unwrap_or_default()
    }

    /// Live subscriber count for a doc. Used by integration tests
    /// to wait until a freshly-connected peer is registered before
    /// pushing into the broadcast path.
    pub fn subscriber_count(&self, doc_id: Uuid) -> usize {
        let inner = self.inner.lock().unwrap();
        inner.docs.get(&doc_id).map(|s| s.len()).unwrap_or(0)
    }

    pub fn connected_subscribers(&self, doc_id: Uuid) -> Vec<ConnectedSubscriber> {
        let inner = self.inner.lock().unwrap();
        inner
            .docs
            .get(&doc_id)
            .map(|subs| {
                subs.iter()
                    .map(|sub| ConnectedSubscriber {
                        sub_id: sub.sub_id,
                        device_id: sub.device_id,
                    })
                    .collect()
            })
            .unwrap_or_default()
    }
}

fn next_sub_id() -> u64 {
    static NEXT: AtomicU64 = AtomicU64::new(1);
    NEXT.fetch_add(1, Ordering::Relaxed)
}
