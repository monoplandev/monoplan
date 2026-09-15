//! `DeviceAuth` extractor: validates a device token presented via
//! `Authorization: Bearer <hex>` (CLI) or the `monoplan_device` cookie
//! (web) against the `devices` table and surfaces `(account_id,
//! device_id, primary_doc_id)` to handlers. The doc id is picked up
//! at the same JOIN as the device lookup so the WS path doesn't need
//! a second query at upgrade.

use axum::extract::FromRequestParts;
use axum::http::request::Parts;
use uuid::Uuid;

use crate::auth::client_ip::{client_ip, peer_ip};
use crate::auth::cookie;
use crate::auth::queries::{find_device_by_token_hash, touch_device_last_seen};
use crate::auth::tokens::{decode_token, sha256};
use crate::error::ApiError;
use crate::state::AppState;

#[derive(Debug, Clone)]
pub struct DeviceAuth {
    pub account_id: Uuid,
    pub device_id: Uuid,
    pub primary_doc_id: Uuid,
}

impl FromRequestParts<AppState> for DeviceAuth {
    type Rejection = ApiError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let bearer = parts
            .headers
            .get(axum::http::header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            .and_then(|s| s.strip_prefix("Bearer "));
        let cookie_token = cookie::token_from_cookies(&parts.headers);
        let token = bearer.or(cookie_token).ok_or(ApiError::Unauthorized)?;
        let raw = decode_token(token).ok_or(ApiError::Unauthorized)?;
        let hash = sha256(&raw).to_vec();
        let lookup = find_device_by_token_hash(&state.db, hash)
            .await?
            .ok_or(ApiError::Unauthorized)?;
        // Best-effort touch — failure is non-fatal for this request and
        // would only surface as a slightly stale `last_seen_at`.
        let ip = client_ip(&parts.headers, peer_ip(&parts.extensions));
        let _ = touch_device_last_seen(&state.db, lookup.device_id, ip).await;
        Ok(Self {
            account_id: lookup.account_id,
            device_id: lookup.device_id,
            primary_doc_id: lookup.primary_doc_id,
        })
    }
}
