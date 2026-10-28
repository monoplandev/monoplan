//! Doc list / create / delete handlers. All require bearer auth.
//!
//! A doc is access + a per-membership wrap of its DEK
//! (`spec/encryption.md`); creating one is purely a key-management
//! act on the server side. The wrap a creator posts is under their
//! Home DEK (the account key), never the password or recovery KEK, so
//! password change / reset / recovery need not know the doc exists.
//! The doc's history starts with the first `PushUpdates` the creator sends
//! after opening it.
//!
//! Deleting is owner-only and never allowed on the account's primary
//! doc (`accounts.primary_doc_id`, the Home: non-deletable and
//! non-leavable per `spec/sharing-plan.md`). The server drops the doc
//! and every row under it; the client drops its local copy and key.

use axum::extract::{Path, State};
use monoplan_protocol::{CreateDocRequest, CreateDocResponse, DocsListResponse, Membership};

use super::json::Json;
use crate::auth::DeviceAuth;
use crate::auth::queries::{DeleteDocOutcome, NewDoc, create_doc, delete_doc, list_memberships};
use crate::error::{ApiError, ApiResult};
use crate::state::AppState;
use uuid::Uuid;

pub async fn list(
    State(state): State<AppState>,
    auth: DeviceAuth,
) -> ApiResult<Json<DocsListResponse>> {
    let memberships = list_memberships(&state.db, auth.account_id)
        .await?
        .into_iter()
        .map(|m| Membership {
            doc_id: m.doc_id.to_string(),
            role: m.role,
            wrapped_dek: m.wrapped_dek,
            wrapped_dek_nonce: m.wrapped_dek_nonce,
        })
        .collect();
    Ok(Json(DocsListResponse {
        primary_doc_id: auth.primary_doc_id.to_string(),
        memberships,
    }))
}

pub async fn create(
    State(state): State<AppState>,
    auth: DeviceAuth,
    Json(req): Json<CreateDocRequest>,
) -> ApiResult<Json<CreateDocResponse>> {
    if req.wrapped_dek.is_empty() || req.wrapped_dek_nonce.is_empty() {
        return Err(ApiError::BadRequest("wrapped_dek is required".into()));
    }
    let doc_id = create_doc(
        &state.db,
        auth.account_id,
        NewDoc {
            wrapped_dek: req.wrapped_dek,
            wrapped_dek_nonce: req.wrapped_dek_nonce,
        },
    )
    .await?;
    Ok(Json(CreateDocResponse {
        doc_id: doc_id.to_string(),
    }))
}

pub async fn delete(
    State(state): State<AppState>,
    auth: DeviceAuth,
    Path(doc_id): Path<String>,
) -> ApiResult<()> {
    let doc_id =
        Uuid::parse_str(&doc_id).map_err(|_| ApiError::BadRequest("invalid doc id".into()))?;
    if doc_id == auth.primary_doc_id {
        return Err(ApiError::Forbidden(
            "the primary doc cannot be deleted".into(),
        ));
    }
    match delete_doc(&state.db, auth.account_id, doc_id).await? {
        DeleteDocOutcome::Deleted => {
            // Unilateral: every other member's live session learns now;
            // the deleting device forgets the doc itself on this reply.
            let told = state
                .sync_sessions
                .close_doc(doc_id, Some(auth.device_id), "doc deleted");
            tracing::info!(doc_id = %doc_id, live_sessions_closed = told, "doc deleted");
            Ok(())
        }
        DeleteDocOutcome::NotOwner => Err(ApiError::NotFound),
    }
}
