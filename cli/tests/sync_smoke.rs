//! End-to-end CLI sync smoke: real monoplan-server, real Loro, real WS.
//!
//! Bypasses the interactive auth UI by signing up via direct HTTP and
//! materializing the on-disk profile that the CLI would have written.
//! Then drives `Session` directly to verify the sync lifecycle:
//!
//!   open → mutate → flush → re-open → next pull observes nothing new.
//!
//! Confirms ops actually landed on the server by hitting the same
//! sqlite the server uses (via `monoplan-server`'s public queries
//! module).

use std::time::Duration;

use monoplan_cli::commands::export::write_export;
use monoplan_cli::config::{Config, Profile, Secrets};
use monoplan_cli::keystore::dek_to_hex;
use monoplan_cli::storage::Account;
use monoplan_cli::sync::Session;
use monoplan_core::{Dek, Doc, LIST_INBOX, NotesDeltaOp};
use monoplan_server::sync::queries;
use uuid::Uuid;

mod support;

use support::{
    TestServer, materialize_profile, materialize_signup_profile, register_device, reopen_profile,
    signup_via_http,
};

#[tokio::test]
async fn session_pushes_and_acks_then_reopen_is_clean() {
    let server = TestServer::start().await;
    let dek = Dek::generate();
    let signup = signup_via_http(&server, &dek, "smoke-test").await;

    let tmp = tempfile::tempdir().unwrap();
    let profile = materialize_signup_profile(
        tmp.path(),
        &server.base,
        &signup,
        &dek,
        "smoke-test@example.com",
        true,
    )
    .await;

    // First open: connect, handshake, pull (empty). The seeded doc is
    // already persisted locally; only the user's new mutation should
    // ship on flush.
    let session = Session::open_with_profile(profile, true).await.unwrap();
    assert!(session.is_online(), "expected to connect to local server");
    let item_id = session.doc().add_item(LIST_INBOX, "hello world").unwrap();
    session.flush().await.unwrap();

    let primary_doc_id = Uuid::parse_str(&signup.primary_doc_id).unwrap();
    let batch = wait_for_ops(&server, primary_doc_id, 1).await;
    assert_eq!(batch.ops.len(), 1, "only the add-item mutation should push");
    let highest_assigned = batch.ops.iter().map(|o| o.seq).max().unwrap();

    // Device's frontier should have advanced to the highest assigned id.
    let device_uuid = Uuid::parse_str(&signup.device_id).unwrap();
    let acked = queries::get_last_acked_seq(&server.state.db, device_uuid)
        .await
        .unwrap();
    assert_eq!(acked, highest_assigned);

    // Re-open. last_acked_seq is persisted, so the pull is empty,
    // and `pending_export` should be `None`.
    let profile2 = reopen_profile(tmp.path());
    let session2 = Session::open_with_profile(profile2, true).await.unwrap();
    assert!(session2.is_online());
    assert!(
        session2.doc().get_item(&item_id).is_some(),
        "item survived round-trip"
    );
    assert!(
        !session2.doc().has_uncaptured_ops(),
        "no new local mutations"
    );
    session2.flush().await.unwrap();

    // No new op blobs should have been pushed.
    let after = queries::fetch_ops_batch(&server.state.db, primary_doc_id, 0)
        .await
        .unwrap();
    assert_eq!(after.ops.len(), 1, "second flush must not re-push");
}

/// The CLI is one-shot per command: an offline `add` must be captured
/// into the durable op log, survive a process restart (boot replays it),
/// and ship to the server on the next sync. Exercises the
/// capture → boot-replay → outbox-push path end-to-end.
#[tokio::test]
async fn offline_add_survives_restart_then_syncs() {
    let server = TestServer::start().await;
    let dek = Dek::generate();
    let signup = signup_via_http(&server, &dek, "offline-test").await;

    let tmp = tempfile::tempdir().unwrap();
    let profile = materialize_signup_profile(
        tmp.path(),
        &server.base,
        &signup,
        &dek,
        "offline-test@example.com",
        true,
    )
    .await;

    // Offline add — no connect; the mutation is captured to the op log.
    let session = Session::open_with_profile(profile, false).await.unwrap();
    assert!(!session.is_online());
    let item_id = session.doc().add_item(LIST_INBOX, "offline item").unwrap();
    session.flush().await.unwrap();

    // Restart (still offline): the captured op replays from storage.
    let session2 = Session::open_with_profile(reopen_profile(tmp.path()), false)
        .await
        .unwrap();
    assert!(
        session2.doc().get_item(&item_id).is_some(),
        "offline add survived a restart"
    );
    session2.flush().await.unwrap();

    // Now sync: the outbox op ships to the server.
    let session3 = Session::open_with_profile(reopen_profile(tmp.path()), true)
        .await
        .unwrap();
    assert!(session3.is_online());
    session3.flush().await.unwrap();

    let primary_doc_id = Uuid::parse_str(&signup.primary_doc_id).unwrap();
    let batch = wait_for_ops(&server, primary_doc_id, 1).await;
    assert_eq!(batch.ops.len(), 1, "the offline op reached the server");

    // Reopen once more: clean, nothing re-pushes (op acked + compacted).
    let session4 = Session::open_with_profile(reopen_profile(tmp.path()), true)
        .await
        .unwrap();
    assert!(session4.doc().get_item(&item_id).is_some());
    session4.flush().await.unwrap();
    let after = queries::fetch_ops_batch(&server.state.db, primary_doc_id, 0)
        .await
        .unwrap();
    assert_eq!(after.ops.len(), 1, "no duplicate re-push after ack");
}

/// `last_sync_at` means "last successful *online* sync". It must stay
/// unset across offline flushes (every command flushes, even read-only
/// ones) and land only after a real server exchange. Regression guard
/// for the bug where `flush()` stamped it unconditionally — which made
/// `monoplan status` report "Last sync: 0s ago" while fully offline.
#[tokio::test]
async fn last_sync_at_set_only_after_online_sync() {
    let server = TestServer::start().await;
    let dek = Dek::generate();
    let signup = signup_via_http(&server, &dek, "lastsync-test").await;

    let tmp = tempfile::tempdir().unwrap();
    let profile = materialize_signup_profile(
        tmp.path(),
        &server.base,
        &signup,
        &dek,
        "lastsync-test@example.com",
        true,
    )
    .await;
    let doc_id = monoplan_core::DocId(Uuid::parse_str(&signup.primary_doc_id).unwrap());

    // Offline flush, even with a mutation, must not stamp last_sync_at.
    let session = Session::open_with_profile(profile, false).await.unwrap();
    assert!(!session.is_online());
    session.doc().add_item(LIST_INBOX, "offline item").unwrap();
    session.flush().await.unwrap();
    {
        let storage = monoplan_cli::storage::open_storage(&reopen_profile(tmp.path())).unwrap();
        assert!(
            storage
                .read_sync_cursor(doc_id)
                .unwrap()
                .last_sync_at
                .is_none(),
            "offline flush must not set last_sync_at"
        );
    }

    // A real online sync stamps it.
    let session2 = Session::open_with_profile(reopen_profile(tmp.path()), true)
        .await
        .unwrap();
    assert!(session2.is_online());
    session2.flush().await.unwrap();
    {
        let storage = monoplan_cli::storage::open_storage(&reopen_profile(tmp.path())).unwrap();
        assert!(
            storage
                .read_sync_cursor(doc_id)
                .unwrap()
                .last_sync_at
                .is_some(),
            "online sync must set last_sync_at"
        );
    }
}

#[tokio::test]
async fn default_open_skips_connect() {
    let tmp = tempfile::tempdir().unwrap();
    let profile = Profile::new(tmp.path().to_path_buf());
    let fake_account = Uuid::now_v7().to_string();
    let fake_doc_uuid = Uuid::now_v7();
    let doc_id = monoplan_core::DocId(fake_doc_uuid);
    let dek = Dek::generate();
    let storage = monoplan_cli::storage::open_storage(&profile).unwrap();
    monoplan_cli::storage::seed_snapshot(&storage, &dek, doc_id, &Doc::new().unwrap()).unwrap();
    storage
        .write_account(&Account {
            account_id: fake_account.clone(),
            email: "offline@example.com".into(),
            device_id: Uuid::now_v7().to_string(),
            primary_doc_id: doc_id,
        })
        .unwrap();
    profile
        .write_config(&Config {
            server_url: "http://127.0.0.1:1".into(), // guaranteed unreachable
        })
        .unwrap();
    profile
        .write_secrets(&Secrets {
            device_token: "deadbeef".repeat(8),
            dek_hex: dek_to_hex(&dek),
        })
        .unwrap();

    // Without --sync the open call is fast — no 2s timeout penalty.
    let started = std::time::Instant::now();
    let session = Session::open_with_profile(profile, false).await.unwrap();
    assert!(started.elapsed() < Duration::from_millis(500));
    assert!(!session.is_online());
    session.flush().await.unwrap();
}

#[tokio::test]
async fn export_json_writes_semantic_account_dump() {
    let tmp = tempfile::tempdir().unwrap();
    let doc = Doc::new().unwrap();
    let errands = doc.add_list("Errands").unwrap();
    let item_id = doc.add_item(&errands, "buy milk").unwrap();
    doc.apply_notes_delta(
        &item_id,
        &[NotesDeltaOp::Insert {
            insert: "whole milk".to_string(),
        }],
    )
    .unwrap();

    let out = tmp.path().join("export.json");
    write_export(&doc.export_json(), Some(&out)).unwrap();

    let value: serde_json::Value = serde_json::from_slice(&std::fs::read(out).unwrap()).unwrap();
    assert_eq!(value["version"], 1);
    assert_eq!(value["lists"][0]["id"], LIST_INBOX);
    assert_eq!(value["lists"][0]["name"], "Inbox");
    assert_eq!(value["items"][0]["id"], item_id);
    assert_eq!(value["items"][0]["notes"], "whole milk");
}

#[tokio::test]
async fn second_device_observes_first_devices_items_via_pull() {
    let server = TestServer::start().await;
    let dek = Dek::generate();
    let signup = signup_via_http(&server, &dek, "smoke-test").await;

    // Device A: full profile, fresh doc.
    let tmp_a = tempfile::tempdir().unwrap();
    let profile_a = materialize_signup_profile(
        tmp_a.path(),
        &server.base,
        &signup,
        &dek,
        "smoke-test@example.com",
        true,
    )
    .await;

    // Device B: register a second device on the same account, share
    // the DEK (paranthesis: the real device-2 path derives the DEK
    // from password+wrap; here we cheat because we already have it).
    let device_b = register_device(&server, &signup.device_token, "device-b").await;
    let tmp_b = tempfile::tempdir().unwrap();
    let profile_b = Profile::new(tmp_b.path().to_path_buf());
    let primary_doc_uuid = Uuid::parse_str(&signup.primary_doc_id).unwrap();
    let doc_id_b = monoplan_core::DocId(primary_doc_uuid);
    let storage_b = monoplan_cli::storage::open_storage(&profile_b).unwrap();
    storage_b
        .write_account(&Account {
            account_id: signup.account_id.clone(),
            email: "smoke-test@example.com".into(),
            device_id: device_b.device_id.clone(),
            primary_doc_id: doc_id_b,
        })
        .unwrap();
    profile_b
        .write_config(&Config {
            server_url: server.base.clone(),
        })
        .unwrap();
    profile_b
        .write_secrets(&Secrets {
            device_token: device_b.device_token.clone(),
            dek_hex: dek_to_hex(&dek),
        })
        .unwrap();
    monoplan_cli::storage::seed_snapshot(&storage_b, &dek, doc_id_b, &Doc::empty()).unwrap();

    // A pushes a new item.
    let session_a = Session::open_with_profile(profile_a, true).await.unwrap();
    let item_id = session_a.doc().add_item(LIST_INBOX, "from-A").unwrap();
    session_a.flush().await.unwrap();

    // B opens a session — its pull should ingest A's seed + add_item
    // blob and surface the item.
    let session_b = Session::open_with_profile(profile_b, true).await.unwrap();
    assert!(session_b.is_online());
    let view = session_b.doc().get_item(&item_id).unwrap();
    assert_eq!(view.text, "from-A");
    assert_eq!(view.list_id, LIST_INBOX);
    session_b.flush().await.unwrap();
}

/// Focus lens convergence across two devices (`spec/focus.md`). Device A
/// curates Focus (add / reorder / auto-remove-on-Done); device B pulls and
/// must observe the identical Focus order, and both docs' fingerprints must
/// match — the fingerprint hashes the focus order, so this is the CLI↔web
/// parity guard Phase 5 calls for. Also pins the two lifecycle rules:
/// marking a focused item Done removes it from Focus, and un-doing it does
/// **not** bring it back.
#[tokio::test]
async fn focus_curation_converges_across_devices() {
    let server = TestServer::start().await;
    let dek = Dek::generate();
    let signup = signup_via_http(&server, &dek, "focus-A").await;

    // Device A: full profile, fresh seeded doc.
    let tmp_a = tempfile::tempdir().unwrap();
    let profile_a = materialize_signup_profile(
        tmp_a.path(),
        &server.base,
        &signup,
        &dek,
        "focus@example.com",
        true,
    )
    .await;

    // Device B: register a second device on the same account, share the DEK.
    let device_b = register_device(&server, &signup.device_token, "focus-B").await;
    let tmp_b = tempfile::tempdir().unwrap();
    let profile_b = materialize_profile(
        tmp_b.path(),
        &server.base,
        &signup.account_id,
        &signup.primary_doc_id,
        &device_b.device_id,
        &device_b.device_token,
        &dek,
        "focus@example.com",
        false,
    )
    .await;

    // A curates: three inbox items, all pinned to Focus, then reorder the
    // third to the front → focus order [i3, i1, i2].
    let session_a = Session::open_with_profile(profile_a, true).await.unwrap();
    let i1 = session_a.doc().add_item(LIST_INBOX, "one").unwrap();
    let i2 = session_a.doc().add_item(LIST_INBOX, "two").unwrap();
    let i3 = session_a.doc().add_item(LIST_INBOX, "three").unwrap();
    for id in [&i1, &i2, &i3] {
        session_a.doc().add_to_focus(id, usize::MAX).unwrap();
    }
    session_a.doc().move_in_focus(&i3, 0).unwrap();
    assert_eq!(
        session_a.doc().focus_refs(),
        vec![i3.clone(), i1.clone(), i2.clone()],
        "A's local focus order after reorder"
    );
    let fp_curated = session_a.doc().fingerprint();
    session_a.flush().await.unwrap();

    // B pulls on open: identical focus order, identical fingerprint.
    let session_b = Session::open_with_profile(profile_b, true).await.unwrap();
    assert!(session_b.is_online());
    assert_eq!(
        session_b.doc().focus_refs(),
        vec![i3.clone(), i1.clone(), i2.clone()],
        "B observes A's curated focus order via pull"
    );
    assert_eq!(
        session_b.doc().fingerprint(),
        fp_curated,
        "fingerprint parity (focus order is hashed)"
    );
    session_b.flush().await.unwrap();

    // A reopens and marks the focused i1 Done: it self-removes from Focus in
    // the same commit → [i3, i2]. Un-doing it back to Backlog must NOT re-add
    // it.
    let session_a2 = Session::open_with_profile(reopen_profile(tmp_a.path()), true)
        .await
        .unwrap();
    session_a2.doc().set_item_done(&i1, true).unwrap();
    assert_eq!(
        session_a2.doc().focus_refs(),
        vec![i3.clone(), i2.clone()],
        "Done auto-removes the focus ref"
    );
    session_a2.doc().set_item_done(&i1, false).unwrap();
    assert_eq!(
        session_a2.doc().focus_refs(),
        vec![i3.clone(), i2.clone()],
        "un-done does not reappear in Focus"
    );
    let fp_compacted = session_a2.doc().fingerprint();
    session_a2.flush().await.unwrap();

    // B reopens to pull the Done + un-done ops; converges to [i3, i2] with
    // matching fingerprint.
    let session_b2 = Session::open_with_profile(reopen_profile(tmp_b.path()), true)
        .await
        .unwrap();
    assert_eq!(
        session_b2.doc().focus_refs(),
        vec![i3, i2],
        "B converges to the self-compacted focus order"
    );
    assert_eq!(
        session_b2.doc().fingerprint(),
        fp_compacted,
        "fingerprint parity after Done self-clean + un-done"
    );
    session_b2.flush().await.unwrap();
}

#[tokio::test]
async fn when_converges_across_devices() {
    let server = TestServer::start().await;
    let dek = Dek::generate();
    let signup = signup_via_http(&server, &dek, "when-A").await;

    let tmp_a = tempfile::tempdir().unwrap();
    let profile_a = materialize_signup_profile(
        tmp_a.path(),
        &server.base,
        &signup,
        &dek,
        "when@example.com",
        true,
    )
    .await;

    let device_b = register_device(&server, &signup.device_token, "when-B").await;
    let tmp_b = tempfile::tempdir().unwrap();
    let profile_b = materialize_profile(
        tmp_b.path(),
        &server.base,
        &signup.account_id,
        &signup.primary_doc_id,
        &device_b.device_id,
        &device_b.device_token,
        &dek,
        "when@example.com",
        false,
    )
    .await;

    // A: one timed `when`, one all-day, and a deadline beside the first.
    let session_a = Session::open_with_profile(profile_a, true).await.unwrap();
    let timed = session_a.doc().add_item(LIST_INBOX, "timed").unwrap();
    let allday = session_a.doc().add_item(LIST_INBOX, "all day").unwrap();
    session_a
        .doc()
        .set_item_when(&timed, Some("2026-09-12T14:00"))
        .unwrap();
    session_a
        .doc()
        .set_item_deadline(&timed, Some("2026-10-31"))
        .unwrap();
    session_a
        .doc()
        .set_item_when(&allday, Some("2026-09-12"))
        .unwrap();
    let fp_set = session_a.doc().fingerprint();
    session_a.flush().await.unwrap();

    // B pulls on open: both values present, fingerprint parity.
    let session_b = Session::open_with_profile(profile_b, true).await.unwrap();
    assert!(session_b.is_online());
    let b_timed = session_b.doc().get_item(&timed).unwrap();
    assert_eq!(b_timed.when.as_deref(), Some("2026-09-12T14:00"));
    assert_eq!(b_timed.deadline.as_deref(), Some("2026-10-31"));
    assert_eq!(
        session_b.doc().get_item(&allday).unwrap().when.as_deref(),
        Some("2026-09-12")
    );
    assert_eq!(session_b.doc().fingerprint(), fp_set, "when is hashed");

    // B clears the timed one's `when`; the deadline must survive.
    session_b.doc().set_item_when(&timed, None).unwrap();
    let fp_cleared = session_b.doc().fingerprint();
    session_b.flush().await.unwrap();

    // A reopens and observes the clear.
    let session_a2 = Session::open_with_profile(reopen_profile(tmp_a.path()), true)
        .await
        .unwrap();
    let a_timed = session_a2.doc().get_item(&timed).unwrap();
    assert_eq!(a_timed.when, None, "A observes B's clear");
    assert_eq!(a_timed.deadline.as_deref(), Some("2026-10-31"));
    assert_eq!(session_a2.doc().fingerprint(), fp_cleared);
    session_a2.flush().await.unwrap();
}

async fn wait_for_ops(server: &TestServer, doc_id: Uuid, target: usize) -> queries::FetchedBatch {
    let deadline = std::time::Instant::now() + Duration::from_secs(2);
    loop {
        let batch = queries::fetch_ops_batch(&server.state.db, doc_id, 0)
            .await
            .unwrap();
        if batch.ops.len() >= target {
            return batch;
        }
        if std::time::Instant::now() > deadline {
            panic!(
                "ops never reached target={target} (got {})",
                batch.ops.len()
            );
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}
