//! The accountless path (`cli/src/bootstrap.rs`): a fresh install runs
//! a local-only workspace, and `monoplan auth signup` adopts it as the
//! account's Home doc with its history intact.

mod support;

use monoplan_cli::bootstrap;
use monoplan_cli::commands::signup::{self, Credentials};
use monoplan_cli::config::Profile;
use monoplan_cli::daemon;
use monoplan_cli::sync::Engine;
use monoplan_core::{LIST_INBOX, WELCOME_ITEM};
use support::{
    TEST_PASSWORD, TestServer, login_device, materialize_profile, reopen_profile, weak_params,
};
use uuid::Uuid;

#[tokio::test]
async fn fresh_install_runs_local_only_and_signup_adopts_it() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("profile");

    // Nothing there: the first open mints the workspace.
    assert!(Profile::active_at(dir.clone()).is_none());
    let profile = bootstrap::open_or_init_at(dir.clone()).await.unwrap();
    assert!(profile.exists());
    let secrets = profile.read_secrets().unwrap();
    assert!(!secrets.has_account(), "no device token before signup");
    assert_eq!(secrets.docs.len(), 1);
    assert!(
        profile.read_config().is_err(),
        "no server to name before signup"
    );
    let (local_doc_id, local_dek) = bootstrap::local_only_doc(&profile).unwrap().unwrap();

    // A second open is a no-op: same doc, same key.
    let again = bootstrap::open_or_init_at(dir.clone()).await.unwrap();
    let (id2, dek2) = bootstrap::local_only_doc(&again).unwrap().unwrap();
    assert_eq!(id2, local_doc_id);
    assert_eq!(dek2.as_bytes(), local_dek.as_bytes());

    // An engine on it is seeded, never online, and commits locally.
    let engine = Engine::open_with_profile(reopen_profile(&dir), true)
        .await
        .unwrap();
    assert!(engine.is_local_only());
    assert!(!engine.is_online());
    let inbox = engine.doc().items_in_list(LIST_INBOX, false);
    assert!(
        inbox.iter().any(|i| i.text == WELCOME_ITEM),
        "seeded with the onboarding item: {inbox:?}"
    );
    let item_id = engine
        .doc()
        .add_item(LIST_INBOX, "captured before signing up")
        .unwrap();
    engine.flush().await.unwrap();

    // Signup adopts the workspace: same doc id on the server, same
    // key, token written, and the history ships on the first connect.
    let server = TestServer::start().await;
    let email = format!("user-{}@example.com", Uuid::now_v7());
    let outcome = signup::signup(
        &reopen_profile(&dir),
        Credentials {
            server: server.base.clone(),
            email: email.clone(),
            password: TEST_PASSWORD.into(),
            device_name: "local-only-a".into(),
            kdf_params: weak_params(),
        },
    )
    .await
    .unwrap();
    assert!(outcome.adopted);
    assert_eq!(outcome.recovery_code.split(' ').count(), 12);

    let profile = reopen_profile(&dir);
    let secrets = profile.read_secrets().unwrap();
    assert!(secrets.has_account());
    assert_eq!(profile.read_config().unwrap().server_url, server.base);
    let storage = monoplan_cli::storage::open_storage(&profile).unwrap();
    let account = storage.read_account().unwrap();
    assert_eq!(account.primary_doc_id, local_doc_id);
    let identity = account.identity.expect("an account now");
    assert_eq!(identity.email, email);
    assert_eq!(identity.account_id, outcome.account_id);
    assert!(bootstrap::local_only_doc(&profile).unwrap().is_none());

    let engine = Engine::open_with_profile(profile, true).await.unwrap();
    assert!(!engine.is_local_only());
    assert!(engine.is_online(), "signup wrote a usable device token");
    assert_eq!(
        engine.doc().get_item(&item_id).unwrap().text,
        "captured before signing up"
    );
    engine.flush().await.unwrap();

    // A second device of the account pulls the adopted history: the
    // pre-signup item and the seeded list, under the Home DEK that is
    // the old local key.
    let login = login_device(&server.base, &email, TEST_PASSWORD, "local-only-b")
        .await
        .unwrap();
    assert_eq!(login.primary_doc_id, local_doc_id.0.to_string());
    assert_eq!(login.dek.as_bytes(), local_dek.as_bytes());
    let tmp_b = tempfile::tempdir().unwrap();
    let profile_b = materialize_profile(
        tmp_b.path(),
        &server.base,
        &login.account_id,
        &login.primary_doc_id,
        &login.device_id,
        &login.device_token,
        &login.dek,
        &email,
        false,
    )
    .await;
    let engine_b = Engine::open_with_profile(profile_b, true).await.unwrap();
    assert!(engine_b.is_online());
    assert_eq!(
        engine_b.doc().get_item(&item_id).unwrap().text,
        "captured before signing up"
    );
    assert!(
        engine_b
            .doc()
            .items_in_list(LIST_INBOX, false)
            .iter()
            .any(|i| i.text == WELCOME_ITEM)
    );
    engine_b.flush().await.unwrap();
}

/// Signup from a profile that is not local-only (none at all here)
/// still mints and seeds a fresh doc under the server's id.
#[tokio::test]
async fn signup_without_a_local_workspace_seeds_a_fresh_doc() {
    let server = TestServer::start().await;
    let tmp = tempfile::tempdir().unwrap();
    let profile = Profile::create_at(tmp.path().join("profile")).unwrap();
    let email = format!("user-{}@example.com", Uuid::now_v7());
    let outcome = signup::signup(
        &profile,
        Credentials {
            server: server.base.clone(),
            email,
            password: TEST_PASSWORD.into(),
            device_name: "fresh-a".into(),
            kdf_params: weak_params(),
        },
    )
    .await
    .unwrap();
    assert!(!outcome.adopted);
    let engine = Engine::open_with_profile(profile, true).await.unwrap();
    assert!(engine.is_online());
    assert!(
        engine
            .doc()
            .items_in_list(LIST_INBOX, false)
            .iter()
            .any(|i| i.text == WELCOME_ITEM)
    );
    engine.flush().await.unwrap();
}

/// The macOS app hosts a local-only workspace like any other and asks
/// the loop for the window's session (`spec/mac-window-plan.md` "Swift
/// side"): no identity and no server, `local_only` set, the one doc.
#[tokio::test]
async fn embedded_host_reports_a_local_only_session() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("profile");
    let profile = bootstrap::open_or_init_at(dir.clone()).await.unwrap();
    let (local_doc_id, _) = bootstrap::local_only_doc(&profile).unwrap().unwrap();

    let handle = daemon::spawn_with(reopen_profile(&dir), true)
        .await
        .unwrap();
    let info = handle.client.session().await.unwrap();
    assert!(info.local_only);
    assert_eq!(info.account_id, "");
    assert_eq!(info.email, "");
    assert_eq!(info.device_id, "");
    assert_eq!(info.server_url, "");
    assert_eq!(info.primary_doc_id, local_doc_id);
    assert_eq!(info.docs.len(), 1);
    assert_eq!(info.docs[0].doc_id, local_doc_id);
    handle.shutdown().await.unwrap();
}

/// Signing in to an existing account from a local-only workspace (the
/// app's sheet, after its warning; the CLI's `auth login` after its
/// confirmation) replaces the workspace, and the host boots the
/// account's doc afterwards.
#[tokio::test]
async fn login_over_a_local_only_workspace_then_host_boots() {
    use monoplan_cli::commands::login::{self, Credentials as LoginCredentials};
    use support::signup_account;

    let server = TestServer::start().await;
    let account = signup_account(&server, "login-over-local-a").await;

    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("profile");
    let profile = bootstrap::open_or_init_at(dir.clone()).await.unwrap();
    assert!(bootstrap::local_only_doc(&profile).unwrap().is_some());

    let outcome = login::login(
        &reopen_profile(&dir),
        LoginCredentials {
            server: server.base.clone(),
            email: account.email.clone(),
            password: TEST_PASSWORD.into(),
            device_name: "login-over-local-b".into(),
        },
    )
    .await
    .unwrap();
    assert_eq!(outcome.account_id, account.account_id);

    let handle = daemon::spawn_with(reopen_profile(&dir), true)
        .await
        .unwrap();
    let info = handle.client.session().await.unwrap();
    assert!(!info.local_only);
    assert_eq!(info.account_id, account.account_id);
    assert_eq!(info.primary_doc_id.0.to_string(), account.primary_doc_id);
    handle.shutdown().await.unwrap();
}

/// The typed path the apps call (`spec/swift-bindings-plan.md`):
/// `LocalClient::call_blocking` runs `api` functions on the loop from a
/// thread off the runtime, a write raises `Changed` for the watchers
/// and shows up in the CLI's own rendering of the same rows, and a
/// workspace nobody has is its own error.
#[tokio::test(flavor = "multi_thread")]
async fn typed_calls_run_on_the_loop_and_agree_with_the_cli() {
    use monoplan_cli::api::{self, ApiError, items::ItemState, lists::ListFilter};
    use monoplan_cli::commands::{
        DocCmd,
        items::LsArgs,
        lists::{ListsArgs, ListsCmd},
    };
    use monoplan_cli::ipc::{Reply, Request, WatchEvent};

    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("profile");
    bootstrap::open_or_init_at(dir.clone()).await.unwrap();
    let handle = daemon::spawn_with(reopen_profile(&dir), true)
        .await
        .unwrap();
    let client = handle.client.clone();
    // A list made the CLI's way, for the typed calls to fill and count.
    let made = client
        .request(Request::Doc {
            workspace: None,
            cmd: DocCmd::Lists(ListsArgs {
                cmd: ListsCmd::Add {
                    name: "Errands".into(),
                },
            }),
        })
        .await;
    let Reply::Output { stdout, .. } = made else {
        panic!("unexpected: {made:?}");
    };
    let errands = stdout.trim().to_string();
    let (_, mut events) = client.watch();

    let c = client.clone();
    let list = errands.clone();
    let (id, rows, lists) = tokio::task::spawn_blocking(move || {
        let id = c
            .call_blocking(None, {
                let list = list.clone();
                move |cx| {
                    Ok(api::items::add_items(cx, &list, &["typed".to_string()], true)?.remove(0))
                }
            })
            .unwrap();
        let rows = c
            .call_blocking(None, move |cx| api::items::items(cx, &list, false))
            .unwrap();
        let lists = c
            .call_blocking(None, |cx| api::lists::lists(cx, ListFilter::Active))
            .unwrap();
        (id, rows, lists)
    })
    .await
    .unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].id, id);
    assert_eq!(rows[0].text, "typed");
    assert_eq!(rows[0].state, ItemState::Backlog);
    let row = lists.iter().find(|l| l.id == errands).unwrap();
    assert_eq!((row.name.as_str(), row.open), ("Errands", 1));
    assert!(matches!(events.recv().await.unwrap(), WatchEvent::Changed));

    // Ticked through the typed path, read back through the CLI's.
    let (c, done_id) = (client.clone(), id.clone());
    tokio::task::spawn_blocking(move || {
        c.call_blocking(None, move |cx| api::items::set_done(cx, &done_id, true))
            .unwrap();
    })
    .await
    .unwrap();
    let listing = client
        .request(Request::Doc {
            workspace: None,
            cmd: DocCmd::Ls(LsArgs {
                list: errands.clone(),
                done: true,
                json: true,
            }),
        })
        .await;
    let Reply::Output { stdout, .. } = listing else {
        panic!("unexpected: {listing:?}");
    };
    let json: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    let row = json
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["id"] == id.as_str())
        .unwrap();
    assert_eq!(row["state"], "done");
    assert!(row["done_at"].is_i64());

    let c = client.clone();
    let missing = tokio::task::spawn_blocking(move || {
        c.call_blocking(Some("nope".into()), |cx| {
            api::items::items(cx, LIST_INBOX, false)
        })
    })
    .await
    .unwrap();
    assert!(matches!(missing, Err(ApiError::NoSuchWorkspace { .. })));

    handle.shutdown().await.unwrap();
    let c = client.clone();
    let stopped = tokio::task::spawn_blocking(move || {
        c.call_blocking(None, |cx| api::items::items(cx, LIST_INBOX, false))
    })
    .await
    .unwrap();
    assert_eq!(stopped.unwrap_err(), ApiError::Stopped);
}

/// The typed reads that span workspaces get their workspaces from the
/// loop (`Host::with_ctx`): rows come back tagged with the doc they
/// are from, where the same functions over bare docs tag nothing.
#[tokio::test(flavor = "multi_thread")]
async fn typed_reads_that_span_workspaces_tag_their_rows() {
    use monoplan_cli::api::{self, agenda::AgendaQuery, items::Place};

    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("profile");
    bootstrap::open_or_init_at(dir.clone()).await.unwrap();
    let handle = daemon::spawn_with(reopen_profile(&dir), true)
        .await
        .unwrap();
    let home = handle.client.session().await.unwrap().primary_doc_id;
    let home = home.0.to_string();

    let c = handle.client.clone();
    let (id, starred, agenda, placed, hits, binned) = tokio::task::spawn_blocking(move || {
        let id = c
            .call_blocking(None, |cx| {
                let id = api::items::add_items(cx, LIST_INBOX, &["lift".into()], true)?.remove(0);
                api::items::set_when(cx, &id, Some("2030-01-04"))?;
                api::items::set_place(
                    cx,
                    &id,
                    Some(Place {
                        label: "Gym".into(),
                        lat: Some(1.5),
                        lon: Some(2.5),
                        address: None,
                        reference: None,
                    }),
                    false,
                )?;
                api::items::star(cx, &id, 0)?;
                Ok(id)
            })
            .unwrap();
        let starred = c
            .call_blocking(None, api::starred::starred_everywhere)
            .unwrap();
        let agenda = c
            .call_blocking(None, |cx| {
                api::agenda::agenda(
                    cx,
                    &AgendaQuery {
                        days: 36_500,
                        today: Some("2026-11-14"),
                        everywhere: true,
                    },
                )
            })
            .unwrap();
        let placed = c
            .call_blocking(None, |cx| api::places::placed_items(cx, "", 0))
            .unwrap();
        let hits = c
            .call_blocking(None, |cx| api::find::find(cx, "lif", 10))
            .unwrap();
        let binned = c
            .call_blocking(None, {
                let id = id.clone();
                move |cx| {
                    api::items::set_binned(cx, &id, true)?;
                    api::bin::binned_everywhere(cx)
                }
            })
            .unwrap();
        (id, starred, agenda, placed, hits, binned)
    })
    .await
    .unwrap();

    let tag = Some(home);
    assert_eq!(starred.len(), 1);
    assert_eq!(
        (starred[0].id.as_str(), &starred[0].doc_id),
        (id.as_str(), &tag)
    );
    assert_eq!(starred[0].list_name.as_deref(), Some("Inbox"));
    let day = agenda
        .iter()
        .find(|g| g.day.as_deref() == Some("2030-01-04"))
        .unwrap();
    assert_eq!(
        (day.rows[0].id.as_str(), &day.rows[0].doc_id),
        (id.as_str(), &tag)
    );
    assert!(agenda[0].today);
    assert_eq!(placed.len(), 1);
    assert_eq!(
        (placed[0].item.id.as_str(), &placed[0].doc_id),
        (id.as_str(), &tag)
    );
    assert_eq!(hits[0].id, id);
    assert_eq!(hits[0].doc_id, tag);
    assert_eq!(binned.len(), 1);
    assert_eq!(
        (binned[0].id.as_str(), &binned[0].doc_id),
        (id.as_str(), &tag)
    );

    handle.shutdown().await.unwrap();
}
