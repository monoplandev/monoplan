//! Phase 1 of `spec/daemon-plan.md`: the resident host holds the
//! profile, stays connected, applies peer broadcasts live, and refuses
//! one-shot access while it runs.

mod support;

use std::time::Duration;

use monoplan_cli::daemon;
use monoplan_cli::sync::{Session, SyncError};
use monoplan_core::LIST_INBOX;
use monoplan_server::sync::queries;
use support::{
    TEST_PASSWORD, TestServer, login_device, materialize_profile, reopen_profile, signup_account,
};
use uuid::Uuid;

async fn wait_until(
    rx: &mut tokio::sync::watch::Receiver<daemon::DaemonStatus>,
    what: &str,
    pred: impl Fn(&daemon::DaemonStatus) -> bool,
) {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if pred(&rx.borrow()) {
                return;
            }
            rx.changed().await.expect("daemon status channel closed");
        }
    })
    .await
    .unwrap_or_else(|_| panic!("timed out waiting for {what}: {:?}", *rx.borrow()));
}

#[tokio::test]
async fn daemon_owns_profile_and_applies_peer_ops_live() {
    let server = TestServer::start().await;
    let signup = signup_account(&server, "daemon-a", false).await;

    let tmp_a = tempfile::tempdir().unwrap();
    let profile_a = materialize_profile(
        tmp_a.path(),
        &server.base,
        &signup.account_id,
        &signup.primary_doc_id,
        &signup.device_id,
        &signup.device_token,
        &signup.dek,
        &signup.email,
        true,
    )
    .await;

    // Resident host on A: comes up offline, connects, finishes its
    // initial pull.
    let mut handle = daemon::spawn(profile_a).await.unwrap();
    wait_until(&mut handle.status, "daemon online", |s| s.online).await;

    // While it runs, a one-shot on the same profile is refused.
    let err = Session::open_with_profile(reopen_profile(tmp_a.path()), false)
        .await
        .err()
        .expect("one-shot must be refused while the daemon holds the profile");
    assert!(matches!(err, SyncError::DaemonRunning), "{err}");
    assert!(daemon::spawn(reopen_profile(tmp_a.path())).await.is_err());

    // Device B (one-shot) pushes an item.
    let login = login_device(&server.base, &signup.email, TEST_PASSWORD, "daemon-b")
        .await
        .unwrap();
    let tmp_b = tempfile::tempdir().unwrap();
    let profile_b = materialize_profile(
        tmp_b.path(),
        &server.base,
        &login.account_id,
        &login.primary_doc_id,
        &login.device_id,
        &login.device_token,
        &login.dek,
        &signup.email,
        false,
    )
    .await;
    let session_b = Session::open_with_profile(profile_b, true).await.unwrap();
    assert!(session_b.is_online());
    let item_id = session_b
        .doc()
        .add_item(LIST_INBOX, "from-device-b")
        .unwrap();
    session_b.flush().await.unwrap();
    let primary_doc_id = Uuid::parse_str(&signup.primary_doc_id).unwrap();
    let batch = queries::fetch_ops_batch(&server.state.db, primary_doc_id, 0)
        .await
        .unwrap();
    let pushed_seq = batch
        .ops
        .iter()
        .map(|o| o.seq)
        .max()
        .expect("B's flush assigned a seq");

    // The daemon sees the broadcast without anyone asking it to sync.
    wait_until(&mut handle.status, "broadcast applied", |s| {
        s.last_contiguous_seq >= pushed_seq
    })
    .await;

    // Clean shutdown releases the profile; the applied op is on disk.
    handle.shutdown().await.unwrap();
    assert!(!daemon::is_running(tmp_a.path()).unwrap());
    let session_a = Session::open_with_profile(reopen_profile(tmp_a.path()), false)
        .await
        .unwrap();
    let item = session_a
        .doc()
        .get_item(&item_id)
        .expect("peer item persisted by the daemon");
    assert_eq!(item.text, "from-device-b");
}
