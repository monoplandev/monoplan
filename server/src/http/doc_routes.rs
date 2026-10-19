//! Doc list / create handlers. Both require bearer auth.
//!
//! A doc is access + a per-membership wrap of its DEK
//! (`spec/encryption.md`); creating one is purely a key-management
//! act on the server side. The doc's history starts with the first
//! `PushOps` the creator sends after opening it.

use axum::extract::State;
use monoplan_protocol::{CreateDocRequest, CreateDocResponse, DocsListResponse, Membership};

use crate::auth::DeviceAuth;
use crate::auth::queries::{NewDoc, create_doc, find_account_by_id, list_memberships};
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
    let recovery = match (&req.recovery_wrapped_dek, &req.recovery_wrapped_dek_nonce) {
        (Some(w), Some(n)) if !w.is_empty() && !n.is_empty() => true,
        (None, None) => false,
        _ => {
            return Err(ApiError::BadRequest(
                "recovery_wrapped_dek and recovery_wrapped_dek_nonce go together".into(),
            ));
        }
    };
    // An enrolled account must wrap every doc under its recovery KEK
    // too: `/password/reset` closes any membership it cannot re-wrap,
    // so a doc created without the wrap would be lost on reset.
    let account = find_account_by_id(&state.db, auth.account_id)
        .await?
        .ok_or(ApiError::Unauthorized)?;
    if account.recovery_present() && !recovery {
        return Err(ApiError::BadRequest(
            "account has a recovery code; recovery_wrapped_dek is required".into(),
        ));
    }
    let doc_id = create_doc(
        &state.db,
        auth.account_id,
        NewDoc {
            wrapped_dek: req.wrapped_dek,
            wrapped_dek_nonce: req.wrapped_dek_nonce,
            recovery_wrapped_dek: req.recovery_wrapped_dek,
            recovery_wrapped_dek_nonce: req.recovery_wrapped_dek_nonce,
        },
    )
    .await?;
    Ok(Msgpack(CreateDocResponse {
        doc_id: doc_id.to_string(),
    }))
}
