//! The accountless path (`cli/src/bootstrap.rs`): a fresh install runs
//! a local-only workspace, and `monoplan auth signup` adopts it as the
//! account's Home doc with its history intact.

mod support;

use monoplan_cli::bootstrap;
use monoplan_cli::commands::signup::{self, Credentials};
use monoplan_cli::config::Profile;
use monoplan_cli::sync::Session;
use monoplan_core::LIST_INBOX;
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

    // A session on it is seeded, never online, and commits locally.
    let session = Session::open_with_profile(reopen_profile(&dir), true)
        .await
        .unwrap();
    assert!(session.is_local_only());
    assert!(!session.is_online());
    let lists = session.doc().all_lists();
    assert!(
        lists.iter().any(|l| l.name == "Welcome"),
        "seeded with the onboarding list: {lists:?}"
    );
    let item_id = session
        .doc()
        .add_item(LIST_INBOX, "captured before signing up")
        .unwrap();
    session.flush().await.unwrap();

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

    let session = Session::open_with_profile(profile, true).await.unwrap();
    assert!(!session.is_local_only());
    assert!(session.is_online(), "signup wrote a usable device token");
    assert_eq!(
        session.doc().get_item(&item_id).unwrap().text,
        "captured before signing up"
    );
    session.flush().await.unwrap();

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
    let session_b = Session::open_with_profile(profile_b, true).await.unwrap();
    assert!(session_b.is_online());
    assert_eq!(
        session_b.doc().get_item(&item_id).unwrap().text,
        "captured before signing up"
    );
    assert!(
        session_b
            .doc()
            .all_lists()
            .iter()
            .any(|l| l.name == "Welcome")
    );
    session_b.flush().await.unwrap();
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
    let session = Session::open_with_profile(profile, true).await.unwrap();
    assert!(session.is_online());
    assert!(
        session
            .doc()
            .all_lists()
            .iter()
            .any(|l| l.name == "Welcome")
    );
    session.flush().await.unwrap();
}
