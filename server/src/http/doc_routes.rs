//! Doc list / create handlers. Both require bearer auth.
//!
//! A doc is access + a per-membership wrap of its DEK
//! (`spec/encryption.md`); creating one is purely a key-management
//! act on the server side. The wrap a creator posts is under their
//! Home DEK (the account key), never the password or recovery KEK, so
//! password change / reset / recovery need not know the doc exists.
//! The doc's history starts with the first `PushOps` the creator sends
//! after opening it.

use axum::extract::State;
use monoplan_protocol::{CreateDocRequest, CreateDocResponse, DocsListResponse, Membership};

use crate::auth::DeviceAuth;
use crate::auth::queries::{NewDoc, create_doc, list_memberships};
use crate::error::{ApiError, ApiResult};
use crate::http::msgpack::Msgpack;
use crate::state::AppState;

pub async fn list(
    State(state): State<AppState>,
    auth: DeviceAuth,
) -> ApiResult<Msgpack<DocsListResponse>> {
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
    Ok(Msgpack(DocsListResponse {
        primary_doc_id: auth.primary_doc_id.to_string(),
        memberships,
    }))
}

pub async fn create(
    State(state): State<AppState>,
    auth: DeviceAuth,
    Msgpack(req): Msgpack<CreateDocRequest>,
) -> ApiResult<Msgpack<CreateDocResponse>> {
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
    Ok(Msgpack(CreateDocResponse {
        doc_id: doc_id.to_string(),
    }))
}
