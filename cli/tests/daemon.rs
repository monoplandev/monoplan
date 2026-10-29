//! Phase 1 of `spec/daemon-plan.md`: the resident host holds the
//! profile, stays connected, applies peer broadcasts live, and refuses
//! one-shot access while it runs.

mod support;

use std::time::Duration;

use monoplan_cli::commands::DocCmd;
use monoplan_cli::config::Profile;
use monoplan_cli::daemon;
use monoplan_cli::ipc::{self, Reply, Request, WatchEvent, read_frame, write_frame};
use monoplan_cli::sync::{Session, SyncError};
use monoplan_core::LIST_INBOX;
use monoplan_server::sync::queries;
use support::{
    TEST_PASSWORD, TestServer, login_device, materialize_profile, reopen_profile, signup_account,
};
use tokio::net::UnixStream;
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
    let signup = signup_account(&server, "daemon-a").await;

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
    let batch = queries::fetch_updates_batch(&server.state.db, primary_doc_id, 0)
        .await
        .unwrap();
    let pushed_seq = batch
        .updates
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

/// Read frames until one satisfies `pred`, bounded. Events and replies
/// share a connection, so a test reading for one may skip the other.
async fn expect_frame(stream: &mut UnixStream, what: &str, pred: impl Fn(&Reply) -> bool) {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let reply: Reply = read_frame(stream)
                .await
                .unwrap()
                .unwrap_or_else(|| panic!("connection closed waiting for {what}"));
            if pred(&reply) {
                return;
            }
        }
    })
    .await
    .unwrap_or_else(|_| panic!("timed out waiting for {what}"));
}

async fn connect(dir: &std::path::Path) -> UnixStream {
    let mut stream = UnixStream::connect(ipc::socket_path(dir)).await.unwrap();
    write_frame(
        &mut stream,
        &Request::Hello {
            build_id: "test".into(),
        },
    )
    .await
    .unwrap();
    expect_frame(&mut stream, "hello", |r| matches!(r, Reply::Hello { .. })).await;
    stream
}

/// The projection tier of `spec/daemon-plan.md`: a watching connection
/// hears about the sync state coming up, a local command committed on
/// another connection, and a remote device's push, each as one
/// payload-free event, and can run requests of its own meanwhile.
#[tokio::test]
async fn watcher_hears_sync_local_and_remote_changes() {
    let server = TestServer::start().await;
    let signup = signup_account(&server, "watch-a").await;
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
    let mut handle = daemon::spawn(profile_a).await.unwrap();

    // Watch straight away, before the daemon is online, so the
    // transition itself is observed as an event.
    let mut watcher = connect(tmp_a.path()).await;
    write_frame(&mut watcher, &Request::Watch).await.unwrap();
    expect_frame(&mut watcher, "watching", |r| {
        matches!(r, Reply::Watching { .. })
    })
    .await;
    expect_frame(
        &mut watcher,
        "online event",
        |r| matches!(r, Reply::Event(WatchEvent::Sync(s)) if s.online),
    )
    .await;
    wait_until(&mut handle.status, "daemon online", |s| s.online).await;

    // A local command on a second connection.
    let mut cli = connect(tmp_a.path()).await;
    write_frame(
        &mut cli,
        &Request::Doc {
            workspace: None,
            cmd: DocCmd::Add {
                list: LIST_INBOX.to_string(),
                texts: vec!["from-the-cli".into()],
            },
        },
    )
    .await
    .unwrap();
    expect_frame(&mut cli, "add output", |r| {
        matches!(r, Reply::Output { .. })
    })
    .await;
    expect_frame(&mut watcher, "changed after local add", |r| {
        matches!(r, Reply::Event(WatchEvent::Changed))
    })
    .await;

    // A read on the watching connection itself gets a reply, not an
    // event, and sees the item.
    write_frame(
        &mut watcher,
        &Request::Doc {
            workspace: None,
            cmd: DocCmd::Agenda(monoplan_cli::commands::agenda::AgendaArgs {
                days: 0,
                today: None,
                json: true,
            }),
        },
    )
    .await
    .unwrap();
    expect_frame(
        &mut watcher,
        "agenda output",
        |r| matches!(r, Reply::Output { stdout, .. } if stdout.starts_with('[')),
    )
    .await;

    // A remote device pushes; the broadcast applies and is reported.
    let login = login_device(&server.base, &signup.email, TEST_PASSWORD, "watch-b")
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
    session_b
        .doc()
        .add_item(LIST_INBOX, "from-device-b")
        .unwrap();
    session_b.flush().await.unwrap();
    expect_frame(&mut watcher, "changed after remote push", |r| {
        matches!(r, Reply::Event(WatchEvent::Changed))
    })
    .await;

    handle.shutdown().await.unwrap();
}

/// The app-hosted rules of the 2026-10-27 amendment: an embedded host
/// says so in its hello, serves doc commands, and honours `Shutdown`
/// like any host (`logout`, `login`, `password` from the terminal work
/// beside the app, which re-hosts afterwards).
#[tokio::test]
async fn embedded_host_serves_commands_and_honours_shutdown() {
    let server = TestServer::start().await;
    let signup = signup_account(&server, "embedded-a").await;
    let tmp = tempfile::tempdir().unwrap();
    let profile = materialize_profile(
        tmp.path(),
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
    let handle = daemon::spawn_with(profile, true).await.unwrap();

    let mut stream = UnixStream::connect(ipc::socket_path(tmp.path()))
        .await
        .unwrap();
    write_frame(
        &mut stream,
        &Request::Hello {
            build_id: "some-other-build".into(),
        },
    )
    .await
    .unwrap();
    expect_frame(&mut stream, "embedded hello", |r| {
        matches!(r, Reply::Hello { embedded: true, .. })
    })
    .await;

    write_frame(
        &mut stream,
        &Request::Doc {
            workspace: None,
            cmd: DocCmd::Add {
                list: LIST_INBOX.to_string(),
                texts: vec!["from the cli".into()],
            },
        },
    )
    .await
    .unwrap();
    expect_frame(&mut stream, "add output", |r| {
        matches!(r, Reply::Output { .. })
    })
    .await;

    // The CLI's stop path ends it and waits for the profile to free.
    assert!(
        monoplan_cli::client::stop_if_running(tmp.path())
            .await
            .unwrap()
    );
    assert!(!daemon::is_running(tmp.path()).unwrap());
    handle.shutdown().await.unwrap();
}

/// The app's own client path (`spec/mac-window-plan.md`, phase 1): an
/// in-process `LocalClient` answers `Hello` itself, runs doc commands
/// through the same loop as the socket, hears the same watch events
/// as a socket watcher (a local command on either path, in order), and
/// reports the loop's end instead of hanging.
#[tokio::test]
async fn local_client_shares_the_loop_with_the_socket() {
    let server = TestServer::start().await;
    let signup = signup_account(&server, "local-a").await;
    let tmp = tempfile::tempdir().unwrap();
    let profile = materialize_profile(
        tmp.path(),
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
    let handle = daemon::spawn_with(profile, true).await.unwrap();
    let local = handle.client.clone();

    // Hello needs no loop and says what the socket's hello says.
    assert!(matches!(
        local
            .request(Request::Hello {
                build_id: "app".into()
            })
            .await,
        Reply::Hello { embedded: true, .. }
    ));
    assert!(!local.is_stopped());

    // Watch in process and over the socket; both hear both paths.
    let (_sync, mut events) = local.watch();
    let mut socket_watcher = connect(tmp.path()).await;
    write_frame(&mut socket_watcher, &Request::Watch)
        .await
        .unwrap();
    expect_frame(&mut socket_watcher, "watching", |r| {
        matches!(r, Reply::Watching { .. })
    })
    .await;

    let add = |text: &str| Request::Doc {
        workspace: None,
        cmd: DocCmd::Add {
            list: LIST_INBOX.to_string(),
            texts: vec![text.into()],
        },
    };
    assert!(matches!(
        local.request(add("from-the-app")).await,
        Reply::Output { .. }
    ));
    let ev = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if let Ok(WatchEvent::Changed) = events.recv().await {
                return WatchEvent::Changed;
            }
        }
    })
    .await
    .expect("local watcher hears the local command");
    assert_eq!(ev, WatchEvent::Changed);
    expect_frame(&mut socket_watcher, "changed after app add", |r| {
        matches!(r, Reply::Event(WatchEvent::Changed))
    })
    .await;

    // The other way round: a socket command is seen in process, and a
    // read in process sees what it committed.
    write_frame(&mut socket_watcher, &add("from-the-cli"))
        .await
        .unwrap();
    expect_frame(&mut socket_watcher, "add output", |r| {
        matches!(r, Reply::Output { .. })
    })
    .await;
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if let Ok(WatchEvent::Changed) = events.recv().await {
                return;
            }
        }
    })
    .await
    .expect("local watcher hears the socket command");
    let listing = local
        .request(Request::Doc {
            workspace: None,
            cmd: DocCmd::Ls(monoplan_cli::commands::items::LsArgs {
                list: LIST_INBOX.to_string(),
                done: false,
                json: true,
            }),
        })
        .await;
    match listing {
        Reply::Output { stdout, .. } => {
            assert!(stdout.contains("from-the-app"), "{stdout}");
            assert!(stdout.contains("from-the-cli"), "{stdout}");
        }
        other => panic!("unexpected: {other:?}"),
    }

    // The blocking form, from a thread that is not on the runtime.
    let off_runtime = local.clone();
    let hello = tokio::task::spawn_blocking(move || {
        off_runtime.request_blocking(Request::Hello {
            build_id: "app".into(),
        })
    })
    .await
    .unwrap();
    assert!(matches!(hello, Reply::Hello { embedded: true, .. }));

    // After the loop ends: stopped, an error reply, a closed stream.
    handle.shutdown().await.unwrap();
    assert!(local.is_stopped());
    assert!(matches!(
        local.request(add("too late")).await,
        Reply::Err { .. }
    ));
    // Events the exit queued (the sync status flipping) come first;
    // then the stream closes, which is what tells an FFI watcher
    // `stopped`.
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if let Err(tokio::sync::broadcast::error::RecvError::Closed) = events.recv().await {
                return;
            }
        }
    })
    .await
    .expect("watch stream closes once the loop has ended");
}

/// The app's takeover at launch: a headless daemon is stopped and the
/// profile handed over; another app's host is left alone; a profile
/// held by a command (`client::claim`) is reported busy, not waited on.
#[tokio::test]
async fn take_over_stops_headless_only() {
    let server = TestServer::start().await;
    let signup = signup_account(&server, "takeover-a").await;
    let tmp = tempfile::tempdir().unwrap();
    let profile = materialize_profile(
        tmp.path(),
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
    drop(profile);

    // Nothing running: free.
    assert!(monoplan_cli::client::take_over(tmp.path()).await.unwrap());

    // Headless: stopped and handed over.
    let handle = daemon::spawn(Profile::new(tmp.path().to_path_buf()))
        .await
        .unwrap();
    assert!(monoplan_cli::client::take_over(tmp.path()).await.unwrap());
    assert!(!daemon::is_running(tmp.path()).unwrap());
    handle.shutdown().await.unwrap();

    // Another app: left alone.
    let handle = daemon::spawn_with(Profile::new(tmp.path().to_path_buf()), true)
        .await
        .unwrap();
    assert!(!monoplan_cli::client::take_over(tmp.path()).await.unwrap());
    assert!(daemon::is_running(tmp.path()).unwrap(), "still hosting");
    assert!(
        monoplan_cli::client::hosted_by_app(tmp.path())
            .await
            .unwrap()
    );
    handle.shutdown().await.unwrap();

    // A command rewriting the profile holds the lock with no socket:
    // busy now, free once it drops the claim.
    let claim = monoplan_cli::client::claim(tmp.path()).await.unwrap();
    assert!(!monoplan_cli::client::take_over(tmp.path()).await.unwrap());
    assert!(matches!(
        daemon::spawn(Profile::new(tmp.path().to_path_buf())).await,
        Err(SyncError::DaemonRunning)
    ));
    drop(claim);
    assert!(monoplan_cli::client::take_over(tmp.path()).await.unwrap());

    // `claim` against a running host stops it first.
    let handle = daemon::spawn_with(Profile::new(tmp.path().to_path_buf()), true)
        .await
        .unwrap();
    let claim = monoplan_cli::client::claim(tmp.path()).await.unwrap();
    assert!(
        daemon::is_running(tmp.path()).unwrap(),
        "claim holds the lock"
    );
    handle.shutdown().await.unwrap();
    drop(claim);
    assert!(!daemon::is_running(tmp.path()).unwrap());
}
