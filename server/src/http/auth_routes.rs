//! Account / password / recovery HTTP handlers.

use anyhow::Context;
use axum::extract::State;
use axum::http::{HeaderMap, header};
use monoplan_protocol::{
    DeviceCredential, DocKey, LoginRequest, LoginResponse, Membership, PasswordChangeRequest,
    PasswordResetRequest, PasswordResetResponse, PreloginRequest, PreloginResponse, RecoverRequest,
    RecoverResponse, SignupRequest, SignupResponse,
};
use uuid::Uuid;

use crate::auth::cookie;
use crate::auth::queries::{
    DocKeyUpdate, NewAccount, PasswordResetOutcome, PasswordUpdate, PasswordUpdateOutcome,
    create_account, create_device, create_recovery_session, find_account_by_email,
    find_account_by_id, list_memberships, reset_password_with_recovery_session, revoke_device,
    update_password,
};
use crate::auth::tokens::{encode_token, generate_token, sha256};
use crate::auth::{DeviceAuth, normalize_email};
use crate::error::{ApiError, ApiResult};
use crate::http::msgpack::Msgpack;
use crate::state::AppState;

/// 15 minutes per `spec/auth.md`.
const RECOVERY_SESSION_TTL_MILLIS: i64 = 15 * 60 * 1000;

pub async fn signup(
    State(state): State<AppState>,
    Msgpack(req): Msgpack<SignupRequest>,
) -> ApiResult<(HeaderMap, Msgpack<SignupResponse>)> {
    let email = normalize_email(&req.email)
        .ok_or_else(|| ApiError::BadRequest("email is required".into()))?;
    if req.device_name.trim().is_empty() {
        return Err(ApiError::BadRequest("device_name is required".into()));
    }
    if req.device_kind.trim().is_empty() {
        return Err(ApiError::BadRequest("device_kind is required".into()));
    }
    let primary_doc_id = req
        .primary_doc_id
        .as_deref()
        .map(Uuid::parse_str)
        .transpose()
        .map_err(|_| ApiError::BadRequest("primary_doc_id must be a uuid".into()))?;
    let device_token = generate_token();
    let device_token_hash = sha256(&device_token).to_vec();
    // Enrolment is mandatory at signup (`spec/encryption.md` §Recovery
    // tiers); the nullable columns stay for a future opt-out.
    let recovery = &req.recovery;
    let new = NewAccount {
        email,
        password_hash: sha256(&req.auth_secret).to_vec(),
        password_salt: req.master_salt,
        kdf_params: req.kdf_params,
        wrapped_dek: req.wrapped_dek,
        wrapped_dek_nonce: req.wrapped_dek_nonce,
        recovery_salt: Some(recovery.recovery_salt.clone()),
        recovery_auth_hash: Some(sha256(&recovery.recovery_auth_secret).to_vec()),
        recovery_wrapped_dek: Some(recovery.recovery_wrapped_dek.clone()),
        recovery_wrapped_dek_nonce: Some(recovery.recovery_wrapped_dek_nonce.clone()),
        device_name: req.device_name,
        device_kind: req.device_kind,
        device_token_hash,
        primary_doc_id,
    };
    let created = create_account(&state.db, new).await.map_err(|e| {
        if e.to_string() == "account_exists" {
            ApiError::AccountExists
        } else if e.to_string() == "doc_exists" {
            ApiError::Conflict("primary_doc_id already exists".into())
        } else {
            ApiError::Internal(e)
        }
    })?;
    let token_hex = encode_token(&device_token);
    Ok((
        cookie_headers(cookie::set_cookie(&token_hex, state.secure_cookies)),
        Msgpack(SignupResponse {
            account_id: created.account_id.to_string(),
            device_id: created.device_id.to_string(),
            device_token: token_hex,
            primary_doc_id: created.primary_doc_id.to_string(),
        }),
    ))
}

pub async fn prelogin(
    State(state): State<AppState>,
    Msgpack(req): Msgpack<PreloginRequest>,
) -> ApiResult<Msgpack<PreloginResponse>> {
    let email = normalize_email(&req.email)
        .ok_or_else(|| ApiError::BadRequest("email is required".into()))?;
    let account = find_account_by_email(&state.db, email)
        .await?
        .ok_or(ApiError::NotFound)?;
    Ok(Msgpack(PreloginResponse {
        master_salt: account.password_salt,
        kdf_params: account.kdf_params,
        recovery_salt: account.recovery_salt,
    }))
}

pub async fn login(
    State(state): State<AppState>,
    Msgpack(req): Msgpack<LoginRequest>,
) -> ApiResult<(HeaderMap, Msgpack<LoginResponse>)> {
    let email = normalize_email(&req.email).ok_or(ApiError::InvalidCredentials)?;
    let account = find_account_by_email(&state.db, email)
        .await?
        .ok_or(ApiError::InvalidCredentials)?;
    let presented = sha256(&req.auth_secret);
    if !constant_time_eq(presented.as_slice(), &account.password_hash) {
        return Err(ApiError::InvalidCredentials);
    }
    let device = if let Some(reg) = req.register_device {
        if reg.name.trim().is_empty() {
            return Err(ApiError::BadRequest("device.name is required".into()));
        }
        if reg.kind.trim().is_empty() {
            return Err(ApiError::BadRequest("device.kind is required".into()));
        }
        let token = generate_token();
        let device_id = create_device(
            &state.db,
            account.id,
            reg.name,
            reg.kind,
            sha256(&token).to_vec(),
        )
        .await?;
        Some(DeviceCredential {
            device_id: device_id.to_string(),
            device_token: encode_token(&token),
        })
    } else {
        None
    };
    let recovery_present = account.recovery_present();
    let memberships = list_memberships(&state.db, account.id)
        .await?
        .into_iter()
        .map(|m| Membership {
            doc_id: m.doc_id.to_string(),
            role: m.role,
            wrapped_dek: m.wrapped_dek,
            wrapped_dek_nonce: m.wrapped_dek_nonce,
        })
        .collect();
    // Cookie only when this call also registered a device for the
    // caller — otherwise no token was minted for them and there's
    // nothing to set.
    let headers = match device.as_ref() {
        Some(cred) => cookie_headers(cookie::set_cookie(&cred.device_token, state.secure_cookies)),
        None => HeaderMap::new(),
    };
    Ok((
        headers,
        Msgpack(LoginResponse {
            account_id: account.id.to_string(),
            primary_doc_id: account.primary_doc_id.to_string(),
            memberships,
            recovery_present,
            device,
        }),
    ))
}

pub async fn recover(
    State(state): State<AppState>,
    Msgpack(req): Msgpack<RecoverRequest>,
) -> ApiResult<Msgpack<RecoverResponse>> {
    let email = normalize_email(&req.email).ok_or(ApiError::InvalidCredentials)?;
    let account = find_account_by_email(&state.db, email)
        .await?
        .ok_or(ApiError::InvalidCredentials)?;
    // Only accounts that opted into a recovery code can use this path.
    let Some(recovery_auth_hash) = account.recovery_auth_hash.as_deref() else {
        return Err(ApiError::RecoveryNotEnrolled);
    };
    let presented = sha256(&req.recovery_auth_secret);
    if !constant_time_eq(presented.as_slice(), recovery_auth_hash) {
        return Err(ApiError::InvalidCredentials);
    }
    // The recovery code opens one wrap: the primary membership's copy
    // of the Home DEK. Every other membership is wrapped under the Home
    // DEK and is the client's to open after the reset (`GET /api/docs`).
    let doc_keys: Vec<DocKey> = list_memberships(&state.db, account.id)
        .await?
        .into_iter()
        .filter(|m| m.doc_id == account.primary_doc_id)
        .filter_map(|m| {
            Some(DocKey {
                doc_id: m.doc_id.to_string(),
                wrapped_dek: m.recovery_wrapped_dek?,
                wrapped_dek_nonce: m.recovery_wrapped_dek_nonce?,
            })
        })
        .collect();
    if doc_keys.is_empty() {
        // Enrolled (hash matched) but the primary row has no recovery
        // wrap: signup writes both together, so this is a broken
        // invariant rather than a client error.
        return Err(ApiError::Internal(anyhow::anyhow!(
            "recovery enrolled but the primary membership carries no recovery wrap"
        )));
    }
    let session_token = generate_token();
    create_recovery_session(
        &state.db,
        account.id,
        sha256(&session_token).to_vec(),
        RECOVERY_SESSION_TTL_MILLIS,
    )
    .await?;
    Ok(Msgpack(RecoverResponse {
        account_id: account.id.to_string(),
        primary_doc_id: account.primary_doc_id.to_string(),
        doc_keys,
        recovery_session_token: encode_token(&session_token),
    }))
}

pub async fn password_change(
    State(state): State<AppState>,
    auth: DeviceAuth,
    Msgpack(req): Msgpack<PasswordChangeRequest>,
) -> ApiResult<()> {
    let account = find_account_by_id(&state.db, auth.account_id)
        .await?
        .ok_or(ApiError::Unauthorized)?;
    let presented = sha256(&req.current_auth_secret);
    if !constant_time_eq(presented.as_slice(), &account.password_hash) {
        return Err(ApiError::InvalidCredentials);
    }
    let update = PasswordUpdate {
        new_password_hash: sha256(&req.new_auth_secret).to_vec(),
        new_password_salt: req.new_master_salt,
        new_kdf_params: req.new_kdf_params,
        new_doc_keys: parse_doc_keys(req.new_doc_keys)?,
    };
    match update_password(&state.db, account.id, update).await? {
        PasswordUpdateOutcome::Updated => Ok(()),
        PasswordUpdateOutcome::DocKeysMismatch { expected, got } => {
            Err(doc_keys_mismatch(&expected, &got))
        }
    }
}

pub async fn password_reset(
    State(state): State<AppState>,
    Msgpack(req): Msgpack<PasswordResetRequest>,
) -> ApiResult<(HeaderMap, Msgpack<PasswordResetResponse>)> {
    if req.device_name.trim().is_empty() {
        return Err(ApiError::BadRequest("device_name is required".into()));
    }
    if req.device_kind.trim().is_empty() {
        return Err(ApiError::BadRequest("device_kind is required".into()));
    }
    let token = crate::auth::tokens::decode_token(&req.recovery_session_token)
        .ok_or(ApiError::RecoverySessionInvalid)?;
    let update = PasswordUpdate {
        new_password_hash: sha256(&req.new_auth_secret).to_vec(),
        new_password_salt: req.new_master_salt,
        new_kdf_params: req.new_kdf_params,
        new_doc_keys: parse_doc_keys(req.new_doc_keys)?,
    };
    // Session validation + consumption and the password/wrap update
    // share one transaction, so a rejected update leaves the session
    // usable for a corrected retry inside its TTL.
    let account_id =
        match reset_password_with_recovery_session(&state.db, sha256(&token).to_vec(), update)
            .await?
        {
            PasswordResetOutcome::Updated { account_id } => account_id,
            PasswordResetOutcome::SessionInvalid => return Err(ApiError::RecoverySessionInvalid),
            PasswordResetOutcome::DocKeysMismatch { expected, got } => {
                return Err(doc_keys_mismatch(&expected, &got));
            }
        };

    let device_token = generate_token();
    let device_id = create_device(
        &state.db,
        account_id,
        req.device_name,
        req.device_kind,
        sha256(&device_token).to_vec(),
    )
    .await?;
    // Look up primary_doc_id so the resetting device can key its local
    // storage without an extra round trip. Account must exist — we just
    // updated it above — so a missing row is internal.
    let primary_doc_id = find_account_by_id(&state.db, account_id)
        .await?
        .map(|a| a.primary_doc_id)
        .ok_or_else(|| {
            ApiError::Internal(anyhow::anyhow!("account missing after password reset"))
        })?;

    let token_hex = encode_token(&device_token);
    Ok((
        cookie_headers(cookie::set_cookie(&token_hex, state.secure_cookies)),
        Msgpack(PasswordResetResponse {
            device_id: device_id.to_string(),
            device_token: token_hex,
            primary_doc_id: primary_doc_id.to_string(),
        }),
    ))
}

#[tracing::instrument(
    skip(state),
    fields(account_id = %auth.account_id, device_id = %auth.device_id)
)]
pub async fn logout(State(state): State<AppState>, auth: DeviceAuth) -> ApiResult<(HeaderMap, ())> {
    // `revoke_device` is idempotent — a stale cookie pointing at an
    // already-revoked device won't 404 us into a useless error here.
    let _ = revoke_device(&state.db, auth.account_id, auth.device_id)
        .await
        .context("auth.logout revoke_device")?;
    Ok((
        cookie_headers(cookie::clear_cookie(state.secure_cookies)),
        (),
    ))
}

/// Wire `DocKey`s → query-layer updates. A malformed doc id is a client
/// bug, surfaced as 400 before anything touches the db. The set check
/// (exactly the primary doc) happens inside the transaction.
fn parse_doc_keys(keys: Vec<DocKey>) -> ApiResult<Vec<DocKeyUpdate>> {
    keys.into_iter()
        .map(|k| {
            let doc_id = Uuid::parse_str(&k.doc_id).map_err(|_| {
                ApiError::BadRequest(format!("new_doc_keys: malformed doc_id {:?}", k.doc_id))
            })?;
            Ok(DocKeyUpdate {
                doc_id,
                wrapped_dek: k.wrapped_dek,
                wrapped_dek_nonce: k.wrapped_dek_nonce,
            })
        })
        .collect()
}

fn doc_keys_mismatch(expected: &[Uuid], got: &[Uuid]) -> ApiError {
    let fmt = |ids: &[Uuid]| {
        ids.iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join(", ")
    };
    ApiError::BadRequest(format!(
        "new_doc_keys must name exactly the account's primary doc: expected [{}], got [{}]",
        fmt(expected),
        fmt(got)
    ))
}

fn cookie_headers(value: axum::http::HeaderValue) -> HeaderMap {
    let mut headers = HeaderMap::new();
    headers.insert(header::SET_COOKIE, value);
    headers
}

/// Constant-time byte slice equality. For password / token comparisons.
fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}
