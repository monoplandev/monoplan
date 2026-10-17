use argon2::password_hash::{PasswordHasher, SaltString};
use argon2::{Algorithm, Argon2, Params, Version};
use axum::Router;
use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode};
use monoplan_server::{AppState, router};
use rusqlite::params;
use serde::Deserialize;
use tower::ServiceExt;
use uuid::Uuid;

const ADMIN_PASSWORD: &str = "correct horse battery staple";

async fn test_app(admin_enabled: bool) -> Router {
    let mut state = AppState::open_in_memory().await.unwrap();
    seed_account_and_devices(&state, 2).await;
    if admin_enabled {
        state = state
            .with_admin_password_hash(test_password_hash())
            .unwrap();
    }
    router(state)
}

#[derive(Debug, Deserialize, PartialEq)]
struct Stats {
    accounts: u64,
    devices: u64,
}

#[tokio::test]
async fn admin_route_is_absent_when_password_hash_is_unset() {
    let response = test_app(false).await.oneshot(request(None)).await.unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn admin_route_rejects_missing_and_invalid_credentials() {
    let app = test_app(true).await;

    let missing = app.clone().oneshot(request(None)).await.unwrap();
    assert_eq!(missing.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(
        missing.headers().get("www-authenticate").unwrap(),
        "Bearer realm=\"monoplan-admin\""
    );

    let invalid = app.oneshot(request(Some("wrong password"))).await.unwrap();
    assert_eq!(invalid.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn admin_stats_returns_account_and_current_device_counts() {
    let response = test_app(true)
        .await
        .oneshot(request(Some(ADMIN_PASSWORD)))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    assert_eq!(
        serde_json::from_slice::<Stats>(&body).unwrap(),
        Stats {
            accounts: 1,
            devices: 2,
        }
    );
}

fn request(password: Option<&str>) -> Request<Body> {
    let mut request = Request::builder().uri("/admin/stats");
    if let Some(password) = password {
        request = request.header("authorization", format!("Bearer {password}"));
    }
    request.body(Body::empty()).unwrap()
}

fn test_password_hash() -> String {
    let params = Params::new(8, 1, 1, None).unwrap();
    let argon2 = Argon2::new(Algorithm::Argon2id, Version::V0x13, params);
    let salt = SaltString::encode_b64(b"monoplan-admin-test").unwrap();
    argon2
        .hash_password(ADMIN_PASSWORD.as_bytes(), &salt)
        .unwrap()
        .to_string()
}

async fn seed_account_and_devices(state: &AppState, device_count: usize) {
    let account_id = Uuid::now_v7();
    let doc_id = Uuid::now_v7();
    state
        .db
        .call(move |connection| {
            let transaction = connection.transaction()?;
            transaction.execute(
                "INSERT INTO docs (id, created_at) VALUES (?, 1)",
                [doc_id.as_bytes().as_slice()],
            )?;
            transaction.execute(
                "INSERT INTO accounts (
                    id, email, password_hash, password_salt,
                    kdf_m_kib, kdf_t, kdf_p, primary_doc_id, created_at
                 ) VALUES (?, 'admin-test@example.com', ?, ?, 8, 1, 1, ?, 1)",
                params![
                    account_id.as_bytes().as_slice(),
                    vec![1u8; 32],
                    vec![2u8; 16],
                    doc_id.as_bytes().as_slice(),
                ],
            )?;
            transaction.execute(
                "INSERT INTO doc_members (
                    doc_id, account_id, role, wrapped_dek, wrapped_dek_nonce, added_at
                 ) VALUES (?, ?, 'owner', ?, ?, 1)",
                params![
                    doc_id.as_bytes().as_slice(),
                    account_id.as_bytes().as_slice(),
                    vec![3u8; 48],
                    vec![4u8; 24],
                ],
            )?;
            for index in 0..device_count {
                let device_id = Uuid::now_v7();
                transaction.execute(
                    "INSERT INTO devices (
                        id, account_id, name, auth_token_hash,
                        last_seen_at, created_at
                     ) VALUES (?, ?, ?, ?, 1, 1)",
                    params![
                        device_id.as_bytes().as_slice(),
                        account_id.as_bytes().as_slice(),
                        format!("device-{index}"),
                        vec![index as u8; 32],
                    ],
                )?;
            }
            transaction.commit()?;
            Ok(())
        })
        .await
        .unwrap();
}
