//! Phase 1 of `spec/daemon-plan.md`: the resident host holds the
//! profile, stays connected, applies peer broadcasts live, and refuses
//! one-shot access while it runs.

mod support;

use std::time::Duration;

use monoplan_cli::commands::DocCmd;
use monoplan_cli::config::Profile;
use monoplan_cli::daemon;
use monoplan_cli::ipc::{self, Reply, Request, WatchEvent, read_frame, write_frame};
use monoplan_cli::sync::{Engine, SyncError};
use monoplan_core::{Doc, LIST_INBOX};
use monoplan_server::sync::queries;
use support::{
    TEST_PASSWORD, TestServer, login_device, materialize_profile, reopen_profile, signup_account,
    wait_for_updates,
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
    let err = Engine::open_with_profile(reopen_profile(tmp_a.path()), false)
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
    let engine_b = Engine::open_with_profile(profile_b, true).await.unwrap();
    assert!(engine_b.is_online());
    let item_id = engine_b
        .doc()
        .add_item(LIST_INBOX, "from-device-b")
        .unwrap();
    engine_b.flush().await.unwrap();
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
    let engine_a = Engine::open_with_profile(reopen_profile(tmp_a.path()), false)
        .await
        .unwrap();
    let item = engine_a
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
                top: false,
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
                all: false,
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
    let engine_b = Engine::open_with_profile(profile_b, true).await.unwrap();
    engine_b
        .doc()
        .add_item(LIST_INBOX, "from-device-b")
        .unwrap();
    engine_b.flush().await.unwrap();
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
                top: false,
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

    // The window's `session` request (spec/mac-window-plan.md "Swift
    // side"): the identity the loop booted with, the server, and the
    // open docs, Home first. In process only; the socket has no frame.
    let info = local.session().await.unwrap();
    assert_eq!(info.account_id, signup.account_id);
    assert_eq!(info.email, signup.email);
    assert_eq!(info.device_id, signup.device_id);
    assert_eq!(info.primary_doc_id.0.to_string(), signup.primary_doc_id);
    assert_eq!(info.server_url, server.base);
    assert!(!info.local_only);
    assert_eq!(info.docs.len(), 1);
    assert_eq!(info.docs[0].doc_id, info.primary_doc_id);
    assert_eq!(info.docs[0].name, None);

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
            top: false,
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

/// A mirror sink that hands everything to a channel: `Some(bytes)` per
/// update, `None` for `stopped`.
struct ChanSink(tokio::sync::mpsc::UnboundedSender<Option<Vec<u8>>>);

impl daemon::MirrorSink for ChanSink {
    fn update(&self, bytes: Vec<u8>) {
        let _ = self.0.send(Some(bytes));
    }
    fn stopped(&self) {
        let _ = self.0.send(None);
    }
}

async fn next_update(
    rx: &mut tokio::sync::mpsc::UnboundedReceiver<Option<Vec<u8>>>,
    what: &str,
) -> Vec<u8> {
    tokio::time::timeout(Duration::from_secs(5), rx.recv())
        .await
        .unwrap_or_else(|_| panic!("timed out waiting for mirror update: {what}"))
        .unwrap_or_else(|| panic!("sink channel dropped waiting for {what}"))
        .unwrap_or_else(|| panic!("mirror stopped waiting for {what}"))
}

/// Is `peer-<slot>.lock` held by someone (the host's lease for a mirror)?
fn slot_is_leased(dir: &std::path::Path, slot: u32) -> bool {
    let file = std::fs::OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(false)
        .open(dir.join(format!("peer-{slot}.lock")))
        .unwrap();
    match file.try_lock() {
        Ok(()) => false,
        Err(std::fs::TryLockError::WouldBlock) => true,
        Err(std::fs::TryLockError::Error(e)) => panic!("{e}"),
    }
}

/// The host side of the mirror (`spec/mac-window-plan.md`, phase 2): a
/// second in-process `Doc`, seeded by `subscribe` under a leased peer
/// of its own, converges after a CLI command on the socket and after a
/// remote device's push; an op it commits and `import`s is applied as
/// local on the host (visible to a command, WALed, pushed so another
/// device pulls it) without echoing back; a duplicate import is a
/// no-op; its slot is released when the subscription drops; and a
/// subscription outliving the loop hears `stopped`.
#[tokio::test]
async fn mirror_converges_both_ways_through_the_host() {
    let server = TestServer::start().await;
    let signup = signup_account(&server, "mirror-a").await;
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
    let mut handle = daemon::spawn_with(profile, true).await.unwrap();
    wait_until(&mut handle.status, "daemon online", |s| s.online).await;
    let local = handle.client.clone();
    let primary_doc_id = Uuid::parse_str(&signup.primary_doc_id).unwrap();

    // Seed: the host holds slot 0; the mirror is leased slot 1 and
    // loads the snapshot under that peer.
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let sub = local.subscribe(None, Box::new(ChanSink(tx))).await.unwrap();
    assert_eq!(sub.doc_id.0, primary_doc_id);
    assert!(slot_is_leased(tmp.path(), 1), "mirror's slot is held");
    let mut mirror = Doc::empty_with_peer(sub.peer_id).unwrap();
    mirror.import_oplog_updates(&sub.snapshot).unwrap();
    assert_eq!(mirror.peer_id(), sub.peer_id);
    assert!(mirror.items_in_list(LIST_INBOX, false).is_empty());

    // Host to mirror, from a CLI command on the socket.
    let mut cli = connect(tmp.path()).await;
    let add = |text: &str| Request::Doc {
        workspace: None,
        cmd: DocCmd::Add {
            list: LIST_INBOX.to_string(),
            texts: vec![text.into()],
            top: false,
        },
    };
    write_frame(&mut cli, &add("from-the-cli")).await.unwrap();
    expect_frame(&mut cli, "add output", |r| {
        matches!(r, Reply::Output { .. })
    })
    .await;
    let bytes = next_update(&mut rx, "cli add").await;
    mirror.import_oplog_updates(&bytes).unwrap();
    let texts = |doc: &Doc| -> Vec<String> {
        doc.items_in_list(LIST_INBOX, false)
            .into_iter()
            .map(|i| i.text)
            .collect()
    };
    assert_eq!(texts(&mirror), vec!["from-the-cli".to_string()]);

    // Host to mirror, from a remote device's push.
    let login = login_device(&server.base, &signup.email, TEST_PASSWORD, "mirror-b")
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
    let engine_b = Engine::open_with_profile(profile_b, true).await.unwrap();
    engine_b
        .doc()
        .add_item(LIST_INBOX, "from-device-b")
        .unwrap();
    engine_b.flush().await.unwrap();
    let bytes = next_update(&mut rx, "remote push").await;
    mirror.import_oplog_updates(&bytes).unwrap();
    assert!(texts(&mirror).contains(&"from-device-b".to_string()));
    assert_eq!(mirror.items_in_list(LIST_INBOX, false).len(), 2);

    // Mirror to host: committed under the mirror's peer, exported since
    // its last send, imported as local.
    let blobs_before = queries::fetch_updates_batch(&server.state.db, primary_doc_id, 0)
        .await
        .unwrap()
        .updates
        .len();
    let since = mirror.oplog_vv();
    let window_item = mirror.add_item(LIST_INBOX, "from-the-window").unwrap();
    let ops = mirror.export_updates_after(&since).unwrap();
    sub.import(ops.clone()).await.unwrap();
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
        Reply::Output { stdout, .. } => assert!(stdout.contains("from-the-window"), "{stdout}"),
        other => panic!("unexpected: {other:?}"),
    }
    // Pushed: the server has a new blob and another device pulls it.
    wait_for_updates(&server, primary_doc_id, blobs_before + 1).await;
    let engine_b = Engine::open_with_profile(reopen_profile(tmp_b.path()), true)
        .await
        .unwrap();
    assert_eq!(
        engine_b.doc().get_item(&window_item).map(|i| i.text),
        Some("from-the-window".to_string()),
        "device B pulled the window's op"
    );
    // Not echoed, and a duplicate import is a no-op.
    sub.import(ops).await.unwrap();
    assert!(
        tokio::time::timeout(Duration::from_millis(300), rx.recv())
            .await
            .is_err(),
        "the mirror's own ops do not come back to it"
    );
    match local
        .request(Request::Doc {
            workspace: None,
            cmd: DocCmd::Ls(monoplan_cli::commands::items::LsArgs {
                list: LIST_INBOX.to_string(),
                done: false,
                json: true,
            }),
        })
        .await
    {
        Reply::Output { stdout, .. } => {
            assert_eq!(stdout.matches("from-the-window").count(), 1, "{stdout}")
        }
        other => panic!("unexpected: {other:?}"),
    }

    // A second mirror takes the next slot and is seeded with everything
    // so far, the window's op included.
    let (tx2, mut rx2) = tokio::sync::mpsc::unbounded_channel();
    let sub2 = local
        .subscribe(None, Box::new(ChanSink(tx2)))
        .await
        .unwrap();
    assert_ne!(sub2.peer_id, sub.peer_id);
    assert!(slot_is_leased(tmp.path(), 2));
    let mut mirror2 = Doc::empty_with_peer(sub2.peer_id).unwrap();
    mirror2.import_oplog_updates(&sub2.snapshot).unwrap();
    assert_eq!(mirror2.items_in_list(LIST_INBOX, false).len(), 3);

    // Dropping a subscription releases its slot.
    drop(sub);
    tokio::time::timeout(Duration::from_secs(5), async {
        while slot_is_leased(tmp.path(), 1) {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("slot 1 released after the subscription dropped");
    assert!(
        slot_is_leased(tmp.path(), 2),
        "the other mirror keeps its slot"
    );

    // The loop ending tells a live mirror so.
    handle.shutdown().await.unwrap();
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(5), rx2.recv())
            .await
            .expect("stopped arrives"),
        Some(None)
    );
    assert!(matches!(
        sub2.import(Vec::new()).await,
        Err(daemon::MirrorError::Stopped)
    ));
    assert!(!slot_is_leased(tmp.path(), 2), "leases go with the loop");
}

/// `Request::Reconnect` (the app's wake / foreground / network-change
/// path) drops a live socket and dials again. The host must come back
/// online on that second socket and keep applying peer broadcasts: a
/// host-initiated disconnect leaves the replica's `ConnStateChanged {
/// online: false }` queued, and the pump after the next connect must
/// not mistake that stale event for the *new* socket's replica going
/// offline (which once dropped the fresh socket and left the replica
/// `Opening` with no socket, cycling "replica disconnected" /
/// "handle_connected called while already connected" forever).
#[tokio::test]
async fn reconnect_request_comes_back_online_and_stays_live() {
    let server = TestServer::start().await;
    let signup = signup_account(&server, "reconnect-a").await;
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
    wait_until(&mut handle.status, "daemon online", |s| s.online).await;
    assert_eq!(handle.status.borrow().connects, 1);

    let mut stream = connect(tmp_a.path()).await;
    write_frame(&mut stream, &Request::Reconnect).await.unwrap();
    expect_frame(&mut stream, "reconnect reply", |r| {
        matches!(r, Reply::Synced { .. })
    })
    .await;

    // Back on a second socket, with the doc open on it.
    wait_until(&mut handle.status, "online after reconnect", |s| {
        s.online && s.connects == 2
    })
    .await;

    // And still live: a peer push arrives without anyone asking.
    let login = login_device(&server.base, &signup.email, TEST_PASSWORD, "reconnect-b")
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
    let engine_b = Engine::open_with_profile(profile_b, true).await.unwrap();
    engine_b
        .doc()
        .add_item(LIST_INBOX, "after-reconnect")
        .unwrap();
    engine_b.flush().await.unwrap();
    let primary_doc_id = Uuid::parse_str(&signup.primary_doc_id).unwrap();
    let batch = queries::fetch_updates_batch(&server.state.db, primary_doc_id, 0)
        .await
        .unwrap();
    let pushed_seq = batch.updates.iter().map(|o| o.seq).max().unwrap();
    wait_until(
        &mut handle.status,
        "broadcast applied after reconnect",
        |s| s.last_contiguous_seq >= pushed_seq,
    )
    .await;
    // One socket per connect, no reconnect storm behind the scenes,
    // and the status shows nothing wrong.
    let status = handle.status.borrow().clone();
    assert_eq!(status.connects, 2, "{status:?}");
    assert!(
        status.online && status.offline_reason.is_none(),
        "{status:?}"
    );

    handle.shutdown().await.unwrap();
}
