use monoplan_cli::sync::Session;
use monoplan_core::LIST_INBOX;

mod support;

use support::{
    TEST_PASSWORD, TestServer, change_password, login_device, materialize_profile, recover_device,
    reopen_profile, signup_account,
};

const NEW_PASSWORD: &str = "even longer correct horse";
const RESET_PASSWORD: &str = "freshly chosen pass";

#[tokio::test]
async fn login_registers_second_device_and_pulls_existing_doc() {
    let server = TestServer::start().await;
    let signup = signup_account(&server, "auth-login-a", false).await;

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
    let session_a = Session::open_with_profile(profile_a, true).await.unwrap();
    let item_id = session_a
        .doc()
        .add_item(LIST_INBOX, "from-device-a")
        .unwrap();
    session_a.flush().await.unwrap();

    let login = login_device(&server.base, &signup.email, TEST_PASSWORD, "auth-login-b")
        .await
        .unwrap();
    assert_eq!(login.account_id, signup.account_id);
    assert_eq!(login.dek.as_bytes(), signup.dek.as_bytes());
    assert!(!login.recovery_present);

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
    let item = session_b.doc().get_item(&item_id).unwrap();
    assert_eq!(item.text, "from-device-a");
    session_b.flush().await.unwrap();
}

#[tokio::test]
async fn password_change_preserves_dek_and_existing_items_for_new_login() {
    let server = TestServer::start().await;
    let signup = signup_account(&server, "auth-password-a", false).await;

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
    let session_a = Session::open_with_profile(profile_a, true).await.unwrap();
    let item_id = session_a
        .doc()
        .add_item(LIST_INBOX, "password-change-item")
        .unwrap();
    session_a.flush().await.unwrap();

    change_password(
        &server.base,
        &signup.email,
        &signup.device_token,
        &[(signup.primary_doc_id.as_str(), &signup.dek)],
        TEST_PASSWORD,
        NEW_PASSWORD,
    )
    .await
    .unwrap();

    let login = login_device(&server.base, &signup.email, NEW_PASSWORD, "auth-password-b")
        .await
        .unwrap();
    assert_eq!(login.account_id, signup.account_id);
    assert_eq!(login.dek.as_bytes(), signup.dek.as_bytes());

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
    assert_eq!(
        session_b.doc().get_item(&item_id).unwrap().text,
        "password-change-item"
    );
    session_b.flush().await.unwrap();
}

#[tokio::test]
async fn recovery_reset_bootstraps_fresh_device_with_existing_items() {
    let server = TestServer::start().await;
    let signup = signup_account(&server, "auth-recover-a", true).await;

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
    let session_a = Session::open_with_profile(profile_a, true).await.unwrap();
    let item_id = session_a.doc().add_item(LIST_INBOX, "recover-me").unwrap();
    session_a.flush().await.unwrap();

    let recovered = recover_device(
        &server.base,
        &signup.email,
        signup.recovery_code.as_deref().unwrap(),
        RESET_PASSWORD,
        "auth-recover-b",
    )
    .await
    .unwrap();
    assert_eq!(recovered.account_id, signup.account_id);
    assert_eq!(recovered.dek.as_bytes(), signup.dek.as_bytes());

    let tmp_b = tempfile::tempdir().unwrap();
    let profile_b = materialize_profile(
        tmp_b.path(),
        &server.base,
        &recovered.account_id,
        &recovered.primary_doc_id,
        &recovered.device_id,
        &recovered.device_token,
        &recovered.dek,
        &signup.email,
        false,
    )
    .await;
    let session_b = Session::open_with_profile(profile_b, true).await.unwrap();
    assert_eq!(
        session_b.doc().get_item(&item_id).unwrap().text,
        "recover-me"
    );
    session_b.flush().await.unwrap();

    let profile_b = reopen_profile(tmp_b.path());
    let storage = monoplan_cli::storage::open_storage(&profile_b).unwrap();
    let account = storage.read_account().unwrap();
    let cursor = storage.read_sync_cursor(account.primary_doc_id).unwrap();
    assert!(
        cursor.last_acked_server_seq.0 > 0,
        "recovered device should sync history"
    );
}
