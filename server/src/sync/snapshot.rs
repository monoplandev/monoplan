use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;
use std::{collections::HashMap, sync::Mutex, time::Duration};

use monoplan_protocol::{EncryptedBlob, SnapshotBase};
use uuid::Uuid;

// Default threshold: 500 *update blobs* (each is one encrypted PushUpdates push;
// see `spec/sync-protocol.md` §"Terminology") since the last snapshot
// makes a doc eligible. Sized for a single-user doc: keeps bootstrap
// catch-up (update replay after snapshot import) short and lets compaction
// advance without churning full-doc snapshots. Operators override via
// `snapshot_threshold_blobs` in the server config (or
// `MONOPLAN_SNAPSHOT_THRESHOLD_BLOBS`); JS e2e tests drop it to a handful
// so the snapshot path runs in seconds.
// Timeout after 5 minutes
pub const SNAPSHOT_THRESHOLD_BLOBS: u64 = 500;
const SNAPSHOT_TIMEOUT: Duration = Duration::from_secs(5 * 60);

struct ActiveLease {
    lease_id: u64,
    expires_at: Instant,
}

enum LeaseState {
    Idle,
    Active(ActiveLease),
}

#[derive(Clone)]
pub struct SnapshotCoordinator {
    timeout: Duration, // Maximum time to allow device to return a snapshot
    threshold_blobs: u64,
    leases: Arc<Mutex<HashMap<Uuid, LeaseState>>>,
    next_lease_id: Arc<AtomicU64>,
}

/// The latest stored snapshot, as the trigger sees it.
pub struct PrevSnapshot {
    pub up_to_seq: u64,
    pub compaction_floor_seq: u64,
    /// Sealed producer VV at `up_to_seq`; becomes the next delta's base.
    pub up_to_vv: EncryptedBlob,
}

pub enum Decision {
    Issue {
        lease_id: u64,
        /// State frontier the snapshot will be encoded at (= producer's
        /// `last_acked_seq`, = `server_last_seq` since producer
        /// must be caught up).
        up_to_seq: u64,
        /// Seq at/below which update blobs become eligible for GC once
        /// this snapshot lands (= `max(horizon, prev compaction_floor_seq)`).
        /// Server-only concept — does not affect what Loro encodes.
        compaction_floor_seq: u64,
        /// `Some`: ask for a delta from the latest snapshot. `None`:
        /// no snapshot exists yet, ask for a full one (the chain head).
        base: Option<SnapshotBase>,
    },
    Skip,
}

pub enum ReleaseResult {
    Accepted,
    Stale, // covers both "wrong lease_id" and "no active lease"
}

impl SnapshotCoordinator {
    pub fn new() -> Self {
        Self::with_config(SNAPSHOT_THRESHOLD_BLOBS, SNAPSHOT_TIMEOUT)
    }

    pub fn with_threshold_blobs(threshold_blobs: u64) -> Self {
        Self::with_config(threshold_blobs, SNAPSHOT_TIMEOUT)
    }

    pub fn with_config(threshold_blobs: u64, timeout: Duration) -> Self {
        Self {
            timeout,
            threshold_blobs,
            leases: Arc::new(Mutex::new(HashMap::new())),
            next_lease_id: Arc::new(AtomicU64::new(1)),
        }
    }
    /// Trigger (see `spec/sync-protocol.md` §"Snapshot orchestration"):
    ///
    /// 1. `server_last_seq − latest snapshot's up_to_seq > threshold`
    ///    — enough new content for a new snapshot to materially cut
    ///    bootstrap cost (smaller `PullUpdates` catch-up after import).
    /// 2. The triggering device is caught up
    ///    (`device_last_acked_seq == server_last_seq`) — that
    ///    value is what we hand back as `up_to_seq`. Lagging
    ///    connections are skipped as producers (still contribute to
    ///    horizon).
    ///
    /// Horizon is **not** a trigger condition — a new snapshot at an
    /// unchanged `compaction_floor_seq` still pays off for bootstrap
    /// even when no further compaction is possible.
    ///
    /// `compaction_floor_seq = max(horizon, prev_compaction_floor_seq)`.
    /// The `max` enforces monotonicity: if a new device's join drops
    /// horizon below the existing floor, we can't undo prior
    /// compaction, so the floor stays put.
    ///
    /// Shape: a *full* snapshot when the doc has none yet (the chain
    /// head), otherwise a *delta* from `prev`. There is no periodic
    /// fold back to full: the chain grows one delta per threshold
    /// crossing until trimmed snapshots (`storage.md` §Epochs) cut it.
    /// The producer is caught up to `server_last_seq >= prev.up_to_seq`,
    /// so it necessarily holds everything in `prev.up_to_vv` and can
    /// export from it.
    pub fn evaluate(
        &self,
        doc_id: Uuid,
        prev: Option<&PrevSnapshot>, // latest stored snapshot, if any
        server_last_seq: u64,        // latest seq on server for this doc
        horizon_seq: u64,            // min(last_acked) across devices
        device_last_acked_seq: u64,  // triggering device's frontier
        now: Instant,
    ) -> Decision {
        let prev_snapshot_up_to_seq = prev.map(|p| p.up_to_seq).unwrap_or(0);
        let prev_compaction_floor_seq = prev.map(|p| p.compaction_floor_seq).unwrap_or(0);
        // Not enough new content to bother.
        if server_last_seq.saturating_sub(prev_snapshot_up_to_seq) < self.threshold_blobs {
            return Decision::Skip;
        }
        // Triggering device isn't caught up — can't be producer.
        if device_last_acked_seq != server_last_seq {
            return Decision::Skip;
        }
        let mut leases = self
            .leases
            .lock()
            .expect("snapshot coordinator mutex poisoned");
        let lease_state = leases.entry(doc_id).or_insert_with(|| LeaseState::Idle);
        if let LeaseState::Active(lease) = lease_state {
            if now < lease.expires_at {
                return Decision::Skip;
            }
        }
        let lease_id = self.next_lease_id.fetch_add(1, Ordering::Relaxed);
        *lease_state = LeaseState::Active(ActiveLease {
            lease_id,
            expires_at: now + self.timeout,
        });
        let base = prev.map(|p| SnapshotBase {
            seq: p.up_to_seq,
            vv: p.up_to_vv.clone(),
        });
        Decision::Issue {
            lease_id,
            up_to_seq: device_last_acked_seq,
            compaction_floor_seq: horizon_seq.max(prev_compaction_floor_seq),
            base,
        }
    }
    pub fn release(&self, doc_id: Uuid, incoming_lease_id: u64) -> ReleaseResult {
        let mut leases = self
            .leases
            .lock()
            .expect("snapshot coordinator mutex poisoned");
        if let Some(lease_state) = leases.get_mut(&doc_id) {
            return match lease_state {
                LeaseState::Idle => ReleaseResult::Stale,
                LeaseState::Active(server_lease) => {
                    if server_lease.lease_id == incoming_lease_id {
                        *lease_state = LeaseState::Idle;
                        ReleaseResult::Accepted
                    } else {
                        ReleaseResult::Stale
                    }
                }
            };
        }
        ReleaseResult::Stale
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Signature reminder:
    //   evaluate(doc_id, prev, server_last, horizon, device_last_acked, now)
    // `prev(up_to, floor)` builds a latest-full-snapshot row; `None`
    // means no snapshot yet.

    fn fake_vv(seed: u8) -> EncryptedBlob {
        EncryptedBlob {
            nonce: vec![seed; 24],
            ciphertext: vec![seed; 8],
        }
    }

    fn prev(up_to_seq: u64, compaction_floor_seq: u64) -> Option<PrevSnapshot> {
        Some(PrevSnapshot {
            up_to_seq,
            compaction_floor_seq,
            up_to_vv: fake_vv(up_to_seq as u8),
        })
    }

    #[test]
    fn issues_when_threshold_exceeded_horizon_advances_and_producer_caught_up() {
        let coord = SnapshotCoordinator::new();
        let doc_id = Uuid::now_v7();
        let now = Instant::now();

        // Within default threshold (500) — skip.
        let decision = coord.evaluate(
            doc_id,
            prev(11_800, 11_800).as_ref(),
            12_000,
            12_000,
            12_000,
            now,
        );
        assert!(matches!(decision, Decision::Skip));

        // Producer not caught up — skip.
        let decision = coord.evaluate(doc_id, None, 12_000, 11_999, 11_999, now);
        assert!(matches!(decision, Decision::Skip));

        // All preconditions met — issue.
        let decision = coord.evaluate(doc_id, None, 12_000, 12_000, 12_000, now);
        assert!(matches!(
            decision,
            Decision::Issue {
                up_to_seq: 12_000,
                compaction_floor_seq: 12_000,
                ..
            }
        ));
        let Decision::Issue { lease_id, .. } = decision else {
            panic!("should issue")
        };

        // In-flight — skip second request.
        let decision = coord.evaluate(doc_id, None, 22_000, 22_000, 22_000, now);
        assert!(matches!(decision, Decision::Skip));

        let result = coord.release(doc_id, lease_id);
        assert!(matches!(result, ReleaseResult::Accepted));

        // After release, issues again.
        let decision_2 = coord.evaluate(doc_id, None, 22_000, 22_000, 22_000, now);
        assert!(matches!(
            decision_2,
            Decision::Issue {
                up_to_seq: 22_000,
                compaction_floor_seq: 22_000,
                ..
            }
        ));

        // Expired lease — issues again.
        let decision = coord.evaluate(
            doc_id,
            None,
            12_000,
            12_000,
            12_000,
            now + coord.timeout + Duration::from_secs(300),
        );
        assert!(matches!(
            decision,
            Decision::Issue {
                up_to_seq: 12_000,
                ..
            }
        ));
    }

    #[test]
    fn issues_with_unchanged_floor_when_horizon_pinned() {
        // Lagger pins horizon at the existing compaction floor. New
        // snapshot still issued for bootstrap perf (state frontier
        // advances), but the floor stays put — no new compaction.
        let coord = SnapshotCoordinator::new();
        let doc_id = Uuid::now_v7();
        let now = Instant::now();

        let decision = coord.evaluate(
            doc_id,
            prev(10_000, 10_000).as_ref(),
            22_000,
            10_000,
            22_000,
            now,
        );
        assert!(matches!(
            decision,
            Decision::Issue {
                up_to_seq: 22_000,
                compaction_floor_seq: 10_000,
                ..
            }
        ));
    }

    #[test]
    fn compaction_floor_never_regresses() {
        // Horizon dipped below the existing compaction floor (e.g., a
        // new device joined with last_acked=0). Compaction can't
        // unwind, so the new snapshot's floor is clamped up to the
        // existing floor.
        let coord = SnapshotCoordinator::new();
        let doc_id = Uuid::now_v7();
        let now = Instant::now();

        let decision = coord.evaluate(
            doc_id,
            prev(10_000, 10_000).as_ref(),
            22_000,
            0,
            22_000,
            now,
        );
        assert!(matches!(
            decision,
            Decision::Issue {
                up_to_seq: 22_000,
                compaction_floor_seq: 10_000,
                ..
            }
        ));
    }

    #[test]
    fn issues_with_advancing_floor_when_horizon_moves() {
        // Horizon advances past the existing compaction floor — new
        // floor tracks horizon, advancing compaction.
        let coord = SnapshotCoordinator::new();
        let doc_id = Uuid::now_v7();
        let now = Instant::now();

        let decision = coord.evaluate(
            doc_id,
            prev(10_000, 10_000).as_ref(),
            22_000,
            15_000,
            22_000,
            now,
        );
        assert!(matches!(
            decision,
            Decision::Issue {
                up_to_seq: 22_000,
                compaction_floor_seq: 15_000,
                ..
            }
        ));
    }

    #[test]
    fn full_when_no_snapshot_then_always_delta_from_latest() {
        let coord = SnapshotCoordinator::new();
        let doc_id = Uuid::now_v7();
        let now = Instant::now();

        // No snapshot yet: full.
        let d = coord.evaluate(doc_id, None, 1_000, 1_000, 1_000, now);
        let Decision::Issue { lease_id, base, .. } = d else {
            panic!("should issue")
        };
        assert!(base.is_none());
        coord.release(doc_id, lease_id);

        // Any later request: delta from the latest row, carrying its VV.
        for (prev_up_to, server_last) in [(1_000u64, 2_000u64), (2_000, 3_000), (9_000, 10_000)] {
            let p = prev(prev_up_to, prev_up_to);
            let d = coord.evaluate(
                doc_id,
                p.as_ref(),
                server_last,
                server_last,
                server_last,
                now,
            );
            let Decision::Issue { lease_id, base, .. } = d else {
                panic!("should issue")
            };
            assert_eq!(
                base,
                Some(SnapshotBase {
                    seq: prev_up_to,
                    vv: fake_vv(prev_up_to as u8),
                })
            );
            coord.release(doc_id, lease_id);
        }
    }

    #[test]
    fn stale_completions_ignored() {
        let coord = SnapshotCoordinator::new();
        let doc_id = Uuid::now_v7();
        let now = Instant::now();

        let result = coord.release(doc_id, 100);
        assert!(
            matches!(result, ReleaseResult::Stale),
            "Stale result if coordinator does not track any snapshot"
        );

        coord.evaluate(doc_id, None, 12_000, 12_000, 12_000, now);
        let result_2 = coord.release(doc_id, 100);
        assert!(
            matches!(result_2, ReleaseResult::Stale),
            "Stale result if coordinator snapshot mismatch with client"
        );
    }
}
