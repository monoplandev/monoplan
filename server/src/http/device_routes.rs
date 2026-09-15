//! Device list / register / rename / revoke handlers. All require bearer auth.

use axum::extract::{Path, State};
use monoplan_protocol::{
    Device, DeviceCredential, DeviceRegistration, DeviceRenameRequest, DevicesListResponse,
};
use uuid::Uuid;

use crate::auth::DeviceAuth;
use crate::auth::queries::{create_device, list_devices, rename_device, revoke_device};
use crate::auth::tokens::{encode_token, generate_token, sha256};
use crate::error::{ApiError, ApiResult};
use crate::http::msgpack::Msgpack;
use crate::state::AppState;

pub async fn list(
    State(state): State<AppState>,
    auth: DeviceAuth,
) -> ApiResult<Msgpack<DevicesListResponse>> {
    let rows = list_devices(&state.db, auth.account_id).await?;
    let server_last_seq =
        crate::sync::queries::latest_doc_seq(&state.db, auth.primary_doc_id).await?;
    Ok(Msgpack(DevicesListResponse {
        devices: rows
            .into_iter()
            .map(|d| Device {
                id: d.id.to_string(),
                name: d.name,
                last_seen_at: d.last_seen_at,
                last_seen_ip: d.last_seen_ip,
                created_at: d.created_at,
                last_acked_seq: d.last_acked_seq.max(0) as u64,
            })
            .collect(),
        server_last_seq,
    }))
}

pub async fn register(
    State(state): State<AppState>,
    auth: DeviceAuth,
    Msgpack(req): Msgpack<DeviceRegistration>,
) -> ApiResult<Msgpack<DeviceCredential>> {
    if req.name.trim().is_empty() {
        return Err(ApiError::BadRequest("name is required".into()));
    }
    let token = generate_token();
    let device_id = create_device(
        &state.db,
        auth.account_id,
        req.name,
        sha256(&token).to_vec(),
    )
    .await?;
    Ok(Msgpack(DeviceCredential {
        device_id: device_id.to_string(),
        device_token: encode_token(&token),
    }))
}

pub async fn revoke(
    State(state): State<AppState>,
    auth: DeviceAuth,
    Path(device_id): Path<String>,
) -> ApiResult<()> {
    let target = Uuid::parse_str(&device_id)
        .map_err(|_| ApiError::BadRequest("invalid device id".into()))?;
    let removed = revoke_device(&state.db, auth.account_id, target).await?;
    if !removed {
        return Err(ApiError::NotFound);
    }
    Ok(())
}

pub async fn rename(
    State(state): State<AppState>,
    auth: DeviceAuth,
    Path(device_id): Path<String>,
    Msgpack(req): Msgpack<DeviceRenameRequest>,
) -> ApiResult<()> {
    let name = req.name.trim();
    if name.is_empty() {
        return Err(ApiError::BadRequest("name is required".into()));
    }
    let target = Uuid::parse_str(&device_id)
        .map_err(|_| ApiError::BadRequest("invalid device id".into()))?;
    let renamed = rename_device(&state.db, auth.account_id, target, name.to_owned()).await?;
    if !renamed {
        return Err(ApiError::NotFound);
    }
    Ok(())
}
