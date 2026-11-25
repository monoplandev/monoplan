//! Attachment routes (`spec/attachments.md` §Routes). All require
//! device auth and an active membership of the doc, and answer 404 to
//! anyone else, so a non-member cannot tell a doc or a file exists.
//!
//! The server sees opaque sealed bodies: no names, no types, no owning
//! item. It enforces the size cap and the quota of the doc's owner and keeps the
//! `doc_files` record. `DELETE` is a member's deliberate, permanent
//! removal and takes the row and the bytes at once. `POST .../tombstone`
//! only stamps the row, and the purge task removes what stays
//! tombstoned past the grace period.

use axum::body::Body;
use axum::extract::{Path, Request, State};
use axum::http::{HeaderValue, header};
use axum::response::{IntoResponse, Response};
use bytes::Bytes;
use monoplan_protocol::{CopyFileRequest, FilesListResponse, StoredFile};
use uuid::Uuid;

use super::json::Json;
use crate::auth::DeviceAuth;
use crate::auth::queries::is_member;
use crate::error::{ApiError, ApiResult};
use crate::files::queries::{
    InsertFile, RestoreFile, delete_file_row, find_file, insert_file, list_files,
    owner_used_bytes, restore_file, tombstone_file,
};
use crate::files::{FileId, MAX_FILE_BODY_BYTES, SEAL_OVERHEAD_BYTES};
use crate::state::AppState;

/// Parse the doc id and require an active membership of it.
async fn member_doc(state: &AppState, auth: &DeviceAuth, doc_id: &str) -> ApiResult<Uuid> {
    let doc_id =
        Uuid::parse_str(doc_id).map_err(|_| ApiError::BadRequest("invalid doc id".into()))?;
    if !is_member(&state.db, doc_id, auth.account_id).await? {
        return Err(ApiError::NotFound);
    }
    Ok(doc_id)
}

fn parse_file_id(file_id: &str) -> ApiResult<FileId> {
    FileId::parse(file_id)
        .ok_or_else(|| ApiError::BadRequest("file id must be 32 lowercase hex characters".into()))
}

pub async fn list(
    State(state): State<AppState>,
    auth: DeviceAuth,
    Path(doc_id): Path<String>,
) -> ApiResult<Json<FilesListResponse>> {
    let doc_id = member_doc(&state, &auth, &doc_id).await?;
    let listing = list_files(&state.db, doc_id, auth.account_id).await?;
    Ok(Json(FilesListResponse {
        doc_bytes: listing.doc_bytes,
        used_bytes: listing.owner_used_bytes,
        limit_bytes: listing
            .owner_used_bytes
            .map(|_| state.max_account_file_bytes),
        files: listing
            .files
            .into_iter()
            .map(|r| StoredFile {
                id: r.id.to_string(),
                bytes: r.bytes,
                created_at: r.created_at,
                deleted_at: r.deleted_at,
            })
            .collect(),
    }))
}

/// `PUT`: one request, the sealed blob as the raw body. Everything
/// that can refuse the upload is checked from the headers and the
/// database before a byte of the body is read.
pub async fn upload(
    State(state): State<AppState>,
    auth: DeviceAuth,
    Path((doc_id, file_id)): Path<(String, String)>,
    req: Request,
) -> ApiResult<()> {
    let doc_id = member_doc(&state, &auth, &doc_id).await?;
    let file_id = parse_file_id(&file_id)?;

    let declared: u64 = req
        .headers()
        .get(header::CONTENT_LENGTH)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse().ok())
        .ok_or(ApiError::LengthRequired)?;
    if declared > MAX_FILE_BODY_BYTES {
        return Err(ApiError::PayloadTooLarge(MAX_FILE_BODY_BYTES));
    }
    if declared < SEAL_OVERHEAD_BYTES {
        return Err(ApiError::BadRequest(
            "body is too short to be a sealed file".into(),
        ));
    }
    // An id that is already there is the client retrying its own
    // upload (the id is random and its own): 409, which it treats as
    // success. Checked before storing so a retry never rewrites bytes.
    if find_file(&state.db, doc_id, file_id).await?.is_some() {
        return Err(ApiError::Conflict("file already stored".into()));
    }
    let limit = state.max_account_file_bytes;
    if owner_used_bytes(&state.db, doc_id)
        .await?
        .saturating_add(declared)
        > limit
    {
        return Err(ApiError::Quota(limit));
    }

    let body = read_body(req.into_body(), declared).await?;

    // Bytes first, row second: a crash in between leaves an object no
    // row names, never a row with nothing behind it.
    state.blobs.put(doc_id, file_id, body).await?;
    let now = state.clock.now_millis();
    match insert_file(
        &state.db,
        doc_id,
        file_id,
        declared,
        auth.device_id,
        now,
        limit,
    )
    .await?
    {
        InsertFile::Inserted => Ok(()),
        // A concurrent upload of the same id won the row; the bytes
        // under the key belong to it now.
        InsertFile::Exists => Err(ApiError::Conflict("file already stored".into())),
        InsertFile::OverQuota => {
            discard(&state, doc_id, file_id).await;
            Err(ApiError::Quota(limit))
        }
    }
}

/// Read exactly the declared number of bytes. axum's default body
/// limit does not apply to a raw `Request`, so the cap is ours.
async fn read_body(body: Body, declared: u64) -> ApiResult<Bytes> {
    let bytes = axum::body::to_bytes(body, declared as usize)
        .await
        .map_err(|_| ApiError::PayloadTooLarge(MAX_FILE_BODY_BYTES))?;
    if bytes.len() as u64 != declared {
        return Err(ApiError::BadRequest(
            "body does not match Content-Length".into(),
        ));
    }
    Ok(bytes)
}

/// Remove bytes stored for a row that then could not be written.
async fn discard(state: &AppState, doc_id: Uuid, file_id: FileId) {
    if let Err(err) = state.blobs.delete(doc_id, file_id).await {
        tracing::error!(doc_id = %doc_id, file_id = %file_id, error = %err, "could not discard an unrecorded attachment body");
    }
}

pub async fn download(
    State(state): State<AppState>,
    auth: DeviceAuth,
    Path((doc_id, file_id)): Path<(String, String)>,
) -> ApiResult<Response> {
    let doc_id = member_doc(&state, &auth, &doc_id).await?;
    let file_id = parse_file_id(&file_id)?;
    let row = find_file(&state.db, doc_id, file_id)
        .await?
        .ok_or(ApiError::NotFound)?;
    if row.deleted_at.is_some() {
        return Err(ApiError::Gone("file is deleted".into()));
    }
    let Some(body) = state.blobs.get(doc_id, file_id).await? else {
        tracing::error!(doc_id = %doc_id, file_id = %file_id, "attachment row has no stored bytes");
        return Err(ApiError::NotFound);
    };
    Ok((
        [(
            header::CONTENT_TYPE,
            HeaderValue::from_static("application/octet-stream"),
        )],
        body,
    )
        .into_response())
}

/// `DELETE`: remove the file for good, by any member, tombstoned or
/// not. The bytes first, then the row, so a request that fails
/// between the two can be repeated (a row without bytes is already a
/// 404 to `GET`). 404 when there is no such file.
pub async fn delete(
    State(state): State<AppState>,
    auth: DeviceAuth,
    Path((doc_id, file_id)): Path<(String, String)>,
) -> ApiResult<()> {
    let doc_id = member_doc(&state, &auth, &doc_id).await?;
    let file_id = parse_file_id(&file_id)?;
    if find_file(&state.db, doc_id, file_id).await?.is_none() {
        return Err(ApiError::NotFound);
    }
    state.blobs.delete(doc_id, file_id).await?;
    delete_file_row(&state.db, doc_id, file_id).await?;
    Ok(())
}

/// `POST .../tombstone`: stamp the file as deleted without destroying
/// it, by any member. Idempotent. For a removal that may yet be taken
/// back (the release half of a cross-doc move, the sweep).
pub async fn tombstone(
    State(state): State<AppState>,
    auth: DeviceAuth,
    Path((doc_id, file_id)): Path<(String, String)>,
) -> ApiResult<()> {
    let doc_id = member_doc(&state, &auth, &doc_id).await?;
    let file_id = parse_file_id(&file_id)?;
    if !tombstone_file(&state.db, doc_id, file_id, state.clock.now_millis()).await? {
        return Err(ApiError::NotFound);
    }
    Ok(())
}

pub async fn restore(
    State(state): State<AppState>,
    auth: DeviceAuth,
    Path((doc_id, file_id)): Path<(String, String)>,
) -> ApiResult<()> {
    let doc_id = member_doc(&state, &auth, &doc_id).await?;
    let file_id = parse_file_id(&file_id)?;
    let limit = state.max_account_file_bytes;
    match restore_file(&state.db, doc_id, file_id, limit).await? {
        RestoreFile::Restored => Ok(()),
        RestoreFile::NotFound => Err(ApiError::NotFound),
        RestoreFile::OverQuota => Err(ApiError::Quota(limit)),
    }
}

/// `POST .../copy`: duplicate a live file into another doc under the
/// same id, without the bytes leaving the server. The caller must be a
/// member of both docs; the copy counts against the quota of the target's
/// owner, and needs the room even when that is the source's owner too.
pub async fn copy(
    State(state): State<AppState>,
    auth: DeviceAuth,
    Path((doc_id, file_id)): Path<(String, String)>,
    Json(req): Json<CopyFileRequest>,
) -> ApiResult<()> {
    let doc_id = member_doc(&state, &auth, &doc_id).await?;
    let file_id = parse_file_id(&file_id)?;
    let to_doc_id = member_doc(&state, &auth, &req.to_doc_id).await?;

    let source = find_file(&state.db, doc_id, file_id)
        .await?
        .ok_or(ApiError::NotFound)?;
    if source.deleted_at.is_some() {
        return Err(ApiError::Gone("file is deleted".into()));
    }
    if find_file(&state.db, to_doc_id, file_id).await?.is_some() {
        return Err(ApiError::Conflict(
            "file already stored in the target".into(),
        ));
    }
    let limit = state.max_account_file_bytes;
    if owner_used_bytes(&state.db, to_doc_id)
        .await?
        .saturating_add(source.bytes)
        > limit
    {
        return Err(ApiError::Quota(limit));
    }

    if !state.blobs.copy(doc_id, file_id, to_doc_id).await? {
        tracing::error!(doc_id = %doc_id, file_id = %file_id, "attachment row has no stored bytes");
        return Err(ApiError::NotFound);
    }
    let now = state.clock.now_millis();
    match insert_file(
        &state.db,
        to_doc_id,
        file_id,
        source.bytes,
        auth.device_id,
        now,
        limit,
    )
    .await?
    {
        InsertFile::Inserted => Ok(()),
        InsertFile::Exists => Err(ApiError::Conflict(
            "file already stored in the target".into(),
        )),
        InsertFile::OverQuota => {
            discard(&state, to_doc_id, file_id).await;
            Err(ApiError::Quota(limit))
        }
    }
}
