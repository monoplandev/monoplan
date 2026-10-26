//! Sharing links and the member list (`spec/sharing-plan.md` §Invites).
//!
//! The link's secret never reaches the server. Clients send an
//! HKDF-derived **invite id** and a wrap of the doc's DEK under a second,
//! independent subkey; the server stores both, can open neither, and
//! enforces the three things it can see: owner-only issuance, expiry,
//! and single use. Issuing for the account's primary doc is refused
//! outright: the Home DEK is the account key, so sharing Home would hand
//! over every doc the account can open (`spec/encryption.md`).

use axum::extract::{Path, State};
use monoplan_protocol::{
    AcceptInviteRequest, AcceptInviteResponse, CreateInviteRequest, CreateInviteResponse, Invite,
    InvitePayloadResponse, InvitesListResponse, Member, MembersListResponse,
};
use uuid::Uuid;

use crate::auth::DeviceAuth;
use crate::auth::invites::{
    AcceptInviteOutcome, AcceptWrap, CreateInviteOutcome, DEFAULT_TTL_MILLIS, FetchInviteOutcome,
    MAX_TTL_MILLIS, MIN_TTL_MILLIS, NewInvite, RemoveMemberOutcome, accept_invite, create_invite,
    fetch_invite, list_invites, list_members, remove_member, revoke_invite,
};
use crate::auth::tokens::decode_token;
use crate::error::{ApiError, ApiResult};
use crate::http::msgpack::Msgpack;
use crate::state::AppState;

pub async fn create(
    State(state): State<AppState>,
    auth: DeviceAuth,
    Path(doc_id): Path<String>,
    Msgpack(req): Msgpack<CreateInviteRequest>,
) -> ApiResult<Msgpack<CreateInviteResponse>> {
    let doc_id = parse_doc_id(&doc_id)?;
    if doc_id == auth.primary_doc_id {
        return Err(ApiError::Forbidden(
            "the primary doc cannot be shared".into(),
        ));
    }
    let id = decode_token(&req.invite_id)
        .ok_or_else(|| ApiError::BadRequest("invalid invite id".into()))?;
    if req.wrapped_dek.is_empty() || req.wrapped_dek_nonce.is_empty() {
        return Err(ApiError::BadRequest("wrapped_dek is required".into()));
    }
    let ttl_millis = match req.expires_in_secs {
        None => DEFAULT_TTL_MILLIS,
        Some(secs) => {
            let millis = i64::try_from(secs)
                .ok()
                .and_then(|s| s.checked_mul(1000))
                .unwrap_or(MAX_TTL_MILLIS);
            millis.clamp(MIN_TTL_MILLIS, MAX_TTL_MILLIS)
        }
    };
    match create_invite(
        &state.db,
        auth.account_id,
        doc_id,
        NewInvite {
            id: id.to_vec(),
            wrapped_dek: req.wrapped_dek,
            wrapped_dek_nonce: req.wrapped_dek_nonce,
            ttl_millis,
        },
    )
    .await?
    {
        CreateInviteOutcome::Created { expires_at } => Ok(Msgpack(CreateInviteResponse {
            invite_id: req.invite_id,
            expires_at,
        })),
        CreateInviteOutcome::NotOwner => Err(ApiError::NotFound),
        CreateInviteOutcome::Duplicate => {
            Err(ApiError::Conflict("invite id already exists".into()))
        }
    }
}

pub async fn list(
    State(state): State<AppState>,
    auth: DeviceAuth,
    Path(doc_id): Path<String>,
) -> ApiResult<Msgpack<InvitesListResponse>> {
    let doc_id = parse_doc_id(&doc_id)?;
    let rows = list_invites(&state.db, auth.account_id, doc_id)
        .await?
        .ok_or(ApiError::NotFound)?;
    Ok(Msgpack(InvitesListResponse {
        invites: rows
            .into_iter()
            .map(|r| Invite {
                invite_id: hex::encode(r.id),
                created_at: r.created_at,
                expires_at: r.expires_at,
                consumed_at: r.consumed_at,
            })
            .collect(),
    }))
}

pub async fn revoke(
    State(state): State<AppState>,
    auth: DeviceAuth,
    Path((doc_id, invite_id)): Path<(String, String)>,
) -> ApiResult<()> {
    let doc_id = parse_doc_id(&doc_id)?;
    let id =
        decode_token(&invite_id).ok_or_else(|| ApiError::BadRequest("invalid invite id".into()))?;
    if revoke_invite(&state.db, auth.account_id, doc_id, id.to_vec()).await? {
        Ok(())
    } else {
        Err(ApiError::NotFound)
    }
}

/// Unauthenticated: the id is the credential, and the wrap it returns is
/// useless without the fragment secret the id was derived from.
pub async fn fetch(
    State(state): State<AppState>,
    Path(invite_id): Path<String>,
) -> ApiResult<Msgpack<InvitePayloadResponse>> {
    let id = decode_token(&invite_id).ok_or(ApiError::NotFound)?;
    match fetch_invite(&state.db, id.to_vec()).await? {
        FetchInviteOutcome::Open(p) => Ok(Msgpack(InvitePayloadResponse {
            doc_id: p.doc_id.to_string(),
            wrapped_dek: p.wrapped_dek,
            wrapped_dek_nonce: p.wrapped_dek_nonce,
            expires_at: p.expires_at,
            inviter_hint: mask_email(&p.inviter_email),
        })),
        FetchInviteOutcome::NotFound => Err(ApiError::NotFound),
        FetchInviteOutcome::Expired => Err(ApiError::Gone("invite expired".into())),
        FetchInviteOutcome::Consumed => Err(ApiError::Gone("invite already used".into())),
    }
}

pub async fn accept(
    State(state): State<AppState>,
    auth: DeviceAuth,
    Path(invite_id): Path<String>,
    Msgpack(req): Msgpack<AcceptInviteRequest>,
) -> ApiResult<Msgpack<AcceptInviteResponse>> {
    let id = decode_token(&invite_id).ok_or(ApiError::NotFound)?;
    if req.wrapped_dek.is_empty() || req.wrapped_dek_nonce.is_empty() {
        return Err(ApiError::BadRequest("wrapped_dek is required".into()));
    }
    match accept_invite(
        &state.db,
        auth.account_id,
        id.to_vec(),
        AcceptWrap {
            wrapped_dek: req.wrapped_dek,
            wrapped_dek_nonce: req.wrapped_dek_nonce,
        },
    )
    .await?
    {
        AcceptInviteOutcome::Joined { doc_id } => Ok(Msgpack(AcceptInviteResponse {
            doc_id: doc_id.to_string(),
        })),
        AcceptInviteOutcome::NotFound => Err(ApiError::NotFound),
        AcceptInviteOutcome::Expired => Err(ApiError::Gone("invite expired".into())),
        AcceptInviteOutcome::Consumed => Err(ApiError::Gone("invite already used".into())),
        AcceptInviteOutcome::AlreadyMember { .. } => {
            Err(ApiError::Conflict("already a member".into()))
        }
    }
}

pub async fn members(
    State(state): State<AppState>,
    auth: DeviceAuth,
    Path(doc_id): Path<String>,
) -> ApiResult<Msgpack<MembersListResponse>> {
    let doc_id = parse_doc_id(&doc_id)?;
    let rows = list_members(&state.db, auth.account_id, doc_id)
        .await?
        .ok_or(ApiError::NotFound)?;
    Ok(Msgpack(MembersListResponse {
        members: rows
            .into_iter()
            .map(|r| Member {
                account_id: r.account_id.to_string(),
                email: r.email,
                role: r.role,
                added_at: r.added_at,
            })
            .collect(),
    }))
}

/// `DELETE /api/docs/:doc_id/members/:account_id` — owner removes a
/// member (`spec/sharing-plan.md` §Member removal). The membership row
/// is stamped removed, then every live connection of that account with
/// the doc open gets `DocClosed` and is unbound from it. Refused for the
/// primary doc before any lookup (it has no other members to remove;
/// the guard keeps Home out of every sharing path). Self-removal is
/// not "leave" — that is a separate verb, not built — and the last
/// owner is never removable. Revocation is forward-only: the removed
/// account keeps what it already pulled.
pub async fn remove_member_route(
    State(state): State<AppState>,
    auth: DeviceAuth,
    Path((doc_id, account_id)): Path<(String, String)>,
) -> ApiResult<()> {
    let doc_id = parse_doc_id(&doc_id)?;
    if doc_id == auth.primary_doc_id {
        return Err(ApiError::Forbidden(
            "the primary doc has no members to remove".into(),
        ));
    }
    let target = Uuid::parse_str(&account_id)
        .map_err(|_| ApiError::BadRequest("invalid account id".into()))?;
    match remove_member(&state.db, auth.account_id, doc_id, target).await? {
        RemoveMemberOutcome::Removed => {
            let told = state
                .sync_sessions
                .close_for_account(doc_id, target, "membership removed");
            tracing::info!(
                doc_id = %doc_id,
                removed_account_id = %target,
                live_sessions_closed = told,
                "member removed"
            );
            Ok(())
        }
        RemoveMemberOutcome::CallerNotMember | RemoveMemberOutcome::TargetNotMember => {
            Err(ApiError::NotFound)
        }
        RemoveMemberOutcome::CallerNotOwner => Err(ApiError::Forbidden(
            "only an owner can remove members".into(),
        )),
        RemoveMemberOutcome::SelfRemoval => Err(ApiError::Conflict(
            "cannot remove yourself from a doc".into(),
        )),
        RemoveMemberOutcome::LastOwner => Err(ApiError::Conflict(
            "cannot remove the last owner of a doc".into(),
        )),
    }
}

fn parse_doc_id(s: &str) -> ApiResult<Uuid> {
    Uuid::parse_str(s).map_err(|_| ApiError::BadRequest("invalid doc id".into()))
}

/// `daniel@example.com` → `d…@example.com`. Enough for "who sent me
/// this?" without handing an unauthenticated caller the full address.
fn mask_email(email: &str) -> String {
    match email.split_once('@') {
        Some((local, domain)) => {
            let first = local.chars().next().map(String::from).unwrap_or_default();
            format!("{first}…@{domain}")
        }
        None => "…".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::mask_email;

    #[test]
    fn masks_local_part() {
        assert_eq!(mask_email("daniel@example.com"), "d…@example.com");
        assert_eq!(mask_email("x@y"), "x…@y");
        assert_eq!(mask_email("nonsense"), "…");
    }
}
