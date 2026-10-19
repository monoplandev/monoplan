//! WAL/VV-separation integration tests against real sqlite files
//! (`spec/vv-wal-separation.md`). These cover the storage half of the
//! spec's test list: bounded offline WALs (1), exact restart from
//! snapshot + WAL (2), cursor/in-flight survival across snapshot folds
//! and reopens (8/9's observable contract — mid-transaction crash
//! atomicity itself is sqlite's guarantee, exercised here as
//! "reopening at any operation boundary yields a consistent state"),
//! and duplicate remote delivery (6, storage half).

use monoplan_core::{
    BootMeta, Dek, Doc, DocId, InFlightPush, LocalSeq, LocalStorage, PushId, RemoteWalRow,
    ServerSeq, SyncEngine, boot_doc,
};
use monoplan_protocol::EncryptedBlob;
use monoplan_storage_sqlite::SqliteStorage;
use uuid::Uuid;

fn doc_id() -> DocId {
    DocId(Uuid::from_u128(0xA1D0))
}

fn blob(byte: u8) -> EncryptedBlob {
    EncryptedBlob {
        nonce: vec![byte; 24],
        ciphertext: vec![byte; 32],
    }
}

fn engine_over(storage: SqliteStorage, dek: Dek, doc: Doc, meta: &BootMeta) -> SyncEngine {
    let mut eng = SyncEngine::new(
        doc,
        doc_id(),
        dek,
        meta.last_acked_server_seq.0,
        Box::new(storage),
    );
    eng.seed_boot(meta);
    eng
}

/// Spec tests 1 + 2: thousands of offline mutations with the CLI's
/// threshold policy keep the WAL bounded, and a restart from snapshot +
/// WAL reconstructs the exact doc state.
#[test]
fn offline_mutations_stay_bounded_and_restart_exactly() {
    const MUTATIONS: usize = 1_000;
    const MAX_ROWS: u64 = 100;
    const MAX_BYTES: u64 = 4 * 1024 * 1024;

    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("wal.sqlite");
    let dek = Dek::generate();

    let fingerprint = {
        let storage = SqliteStorage::open(&path).unwrap();
        let mut eng = engine_over(
            storage.clone(),
            dek.clone(),
            Doc::new().unwrap(),
            &BootMeta::default(),
        );
        for i in 0..MUTATIONS {
            eng.doc_mut()
                .add_item("inbox", &format!("offline item {i}"))
                .unwrap();
            // The CLI's per-command rhythm: capture, then evaluate the
            // fold threshold.
            eng.capture_local_ops().unwrap();
            eng.snapshot_if_wal_exceeds(MAX_ROWS, MAX_BYTES).unwrap();
            assert!(
                eng.wal_rows() < MAX_ROWS,
                "WAL must stay bounded (rows = {} at i = {i})",
                eng.wal_rows()
            );
        }
        // Storage agrees with the engine's in-memory stats.
        let boot = storage.boot(doc_id()).unwrap();
        assert!(
            (boot.replay.len() as u64) < MAX_ROWS,
            "persisted WAL rows = {}",
            boot.replay.len()
        );
        assert!(
            boot.snapshot.is_some(),
            "periodic folding produced a snapshot"
        );
        eng.doc().fingerprint()
    };

    // "Restart": fresh connection over the same file, full boot replay.
    let storage = SqliteStorage::open(&path).unwrap();
    let (doc, meta) = boot_doc(&storage, &dek, doc_id(), None).unwrap();
    assert_eq!(doc.fingerprint(), fingerprint, "exact state after restart");
    assert!(
        !doc.has_uncaptured_ops(),
        "boot replay covers the capture cursor"
    );
    assert_eq!(doc.items_in_list("inbox", false).len(), MUTATIONS);
    // Nothing synced in this scenario, so everything still derives as
    // unsent from the (empty) server_known_vv.
    assert!(meta.server_known_vv.is_empty());
}

/// Spec test 9's observable contract: after a snapshot fold commits,
/// rows past the cutoff plus every cursor and the in-flight record
/// survive a reopen; rows at or below the cutoff are gone.
#[test]
fn snapshot_fold_preserves_tail_cursors_and_in_flight_across_reopen() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("fold.sqlite");

    let push = InFlightPush {
        push_id: PushId::generate(),
        payload: blob(9),
        from_vv: b"from".to_vec(),
        to_vv: b"to".to_vec(),
    };
    {
        let storage = SqliteStorage::open(&path).unwrap();
        storage.append_local_wal(doc_id(), blob(1)).unwrap();
        storage.append_local_wal(doc_id(), blob(2)).unwrap();
        storage
            .write_server_known_vv(doc_id(), b"vv-bytes")
            .unwrap();
        storage.write_acked_seq(doc_id(), ServerSeq(7)).unwrap();
        storage.put_in_flight_push(doc_id(), push.clone()).unwrap();
        // Fold through row 1 only — row 2 is the surviving tail.
        storage
            .write_snapshot(doc_id(), LocalSeq(1), blob(0xFF))
            .unwrap();
    } // drop = "crash after the fold committed"

    let storage = SqliteStorage::open(&path).unwrap();
    let boot = storage.boot(doc_id()).unwrap();
    let snap = boot.snapshot.expect("snapshot survived");
    assert_eq!(snap.up_to_local_seq, LocalSeq(2), "high-water, not cutoff");
    assert_eq!(boot.replay.len(), 1, "tail row survived the fold");
    assert_eq!(boot.replay[0].local_seq, LocalSeq(2));
    assert_eq!(boot.last_local_seq, LocalSeq(2));
    assert_eq!(boot.server_known_vv, b"vv-bytes");
    assert_eq!(boot.last_acked_server_seq, ServerSeq(7));
    let stored = boot.in_flight_push.expect("in-flight record survived");
    assert_eq!(stored.push_id, push.push_id);
    assert_eq!(stored.payload, push.payload);
    assert_eq!(stored.from_vv, push.from_vv);
    assert_eq!(stored.to_vv, push.to_vv);

    // Post-prune appends stay monotonic past the pruned max.
    let next = storage.append_local_wal(doc_id(), blob(3)).unwrap();
    assert_eq!(next, LocalSeq(3));
}

/// Spec test 8's observable contract: `write_snapshot` is one sqlite
/// transaction, so a failure before commit leaves the previous
/// snapshot + full WAL untouched. Simulated at the API boundary: state
/// before the fold is fully recoverable from a parallel connection.
#[test]
fn state_before_fold_is_consistent_from_a_second_connection() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("prefold.sqlite");
    let storage = SqliteStorage::open(&path).unwrap();
    storage.append_local_wal(doc_id(), blob(1)).unwrap();
    storage
        .write_snapshot(doc_id(), LocalSeq(1), blob(0xAA))
        .unwrap();
    storage.append_local_wal(doc_id(), blob(2)).unwrap();

    // A second connection (≙ the post-crash process) sees exactly the
    // old snapshot + the surviving tail.
    let reopened = SqliteStorage::open(&path).unwrap();
    let boot = reopened.boot(doc_id()).unwrap();
    assert_eq!(boot.snapshot.unwrap().payload, blob(0xAA));
    assert_eq!(boot.replay.len(), 1);
    assert_eq!(boot.replay[0].payload, blob(2));
}

/// Spec test 6, storage half: a duplicate `server_seq` append returns
/// the original row (no phantom) while still persisting the advanced
/// `server_known_vv`.
#[test]
fn duplicate_remote_append_is_idempotent_but_advances_vv() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("dup.sqlite");
    let storage = SqliteStorage::open(&path).unwrap();

    let row = RemoteWalRow {
        server_seq: ServerSeq(4),
        payload: blob(4),
    };
    let (seq1, ins1) = storage
        .append_remote_wal(doc_id(), row.clone(), b"vv1")
        .unwrap();
    assert!(ins1);
    let (seq2, ins2) = storage.append_remote_wal(doc_id(), row, b"vv2").unwrap();
    assert!(!ins2, "duplicate server_seq must not insert");
    assert_eq!(seq1, seq2);

    let boot = storage.boot(doc_id()).unwrap();
    assert_eq!(boot.replay.len(), 1);
    assert_eq!(
        boot.server_known_vv, b"vv2",
        "duplicate delivery still proves possession"
    );
}

/// `complete_push` clears only the matching record and persists the VV
/// in the same transaction.
#[test]
fn complete_push_is_scoped_to_its_push_id() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("complete.sqlite");
    let storage = SqliteStorage::open(&path).unwrap();

    let push = InFlightPush {
        push_id: PushId::generate(),
        payload: blob(5),
        from_vv: vec![],
        to_vv: vec![],
    };
    storage.put_in_flight_push(doc_id(), push.clone()).unwrap();

    // Stale/mismatched push_id: VV lands, record survives.
    storage
        .complete_push(doc_id(), PushId::generate(), b"vv1")
        .unwrap();
    let boot = storage.boot(doc_id()).unwrap();
    assert_eq!(boot.server_known_vv, b"vv1");
    assert!(boot.in_flight_push.is_some());

    // Matching push_id: record cleared.
    storage
        .complete_push(doc_id(), push.push_id, b"vv2")
        .unwrap();
    let boot = storage.boot(doc_id()).unwrap();
    assert_eq!(boot.server_known_vv, b"vv2");
    assert!(boot.in_flight_push.is_none());
}

/// spec/peer-id-plan.md: a doc booted with the leased peer id resumes
/// that peer's counter from replayed history, so sequential invocations
/// leave exactly one peer entry in the version vector instead of one
/// per invocation.
#[test]
fn leased_peer_is_stable_across_restarts() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("peer.sqlite");
    let dek = Dek::generate();
    const PEER: u64 = 42;

    // Invocation 1: fresh profile, seeded doc, one mutation, persisted.
    let counter_first = {
        let storage = SqliteStorage::open(&path).unwrap();
        let mut eng = engine_over(
            storage,
            dek.clone(),
            Doc::new_with_peer(PEER).unwrap(),
            &BootMeta::default(),
        );
        eng.doc_mut().add_item("inbox", "first run").unwrap();
        eng.capture_local_ops().unwrap();
        eng.doc().oplog_vv().get(&PEER).copied().unwrap()
    };
    assert!(counter_first > 0);

    // Invocation 2: boot from storage under the same peer, mutate again.
    let storage = SqliteStorage::open(&path).unwrap();
    let (doc, meta) = boot_doc(&storage, &dek, doc_id(), Some(PEER)).unwrap();
    assert_eq!(doc.peer_id(), PEER);
    let mut eng = engine_over(storage, dek, doc, &meta);
    eng.doc_mut().add_item("inbox", "second run").unwrap();
    eng.capture_local_ops().unwrap();
    let vv = eng.doc().oplog_vv();
    assert!(
        vv.get(&PEER).copied().unwrap() > counter_first,
        "counter resumes past replayed history, never rewinds"
    );
    assert_eq!(
        vv.iter().count(),
        1,
        "sequential invocations add no new peers"
    );
}
