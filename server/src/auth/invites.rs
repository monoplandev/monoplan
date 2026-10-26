//! Sqlite reads/writes for sharing links (`doc_invites`) and the member
//! list. See `spec/sharing-plan.md` §Invites.
//!
//! The server's share of the invite is deliberately small: it stores an
//! opaque id and a wrap it cannot open, enforces owner-only issuance,
//! expiry and single use, and turns an accept into a `doc_members` row.
//! Which key the wrap is under is a client contract, as everywhere else.

use anyhow::Context;
use monoplan_protocol::MemberRole;
use rusqlite::{OptionalExtension, params};
use uuid::Uuid;

use crate::db::{Db, now_millis};

/// Server default lifetime of a link: 7 days.
pub const DEFAULT_TTL_MILLIS: i64 = 7 * 24 * 60 * 60 * 1000;
/// Longest a client may ask for: 30 days.
pub const MAX_TTL_MILLIS: i64 = 30 * 24 * 60 * 60 * 1000;
/// Shortest: a minute, so a mistyped unit cannot mint a dead link.
pub const MIN_TTL_MILLIS: i64 = 60 * 1000;

#[derive(Debug, Clone)]
pub struct NewInvite {
    pub id: Vec<u8>,
    pub wrapped_dek: Vec<u8>,
    pub wrapped_dek_nonce: Vec<u8>,
    pub ttl_millis: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CreateInviteOutcome {
    Created {
        expires_at: i64,
    },
    /// No active `owner` membership for the caller on this doc.
    /// Reported as not-found so a non-member learns nothing.
    NotOwner,
    /// An invite with this id already exists (a client reusing a
    /// secret). Nothing written.
    Duplicate,
}

/// Insert a link for `doc_id`, issued by `account_id`, who must be an
/// active owner. The caller refuses the primary doc before calling.
pub async fn create_invite(
    db: &Db,
    account_id: Uuid,
    doc_id: Uuid,
    new: NewInvite,
) -> anyhow::Result<CreateInviteOutcome> {
    let acc_bytes = account_id.as_bytes().to_vec();
    let doc_bytes = doc_id.as_bytes().to_vec();
    let now = now_millis();
    let expires_at = now + new.ttl_millis;
    db.call(move |c| {
        let tx = c.transaction()?;
        if !is_active_owner(&tx, &doc_bytes, &acc_bytes)? {
            return Ok(CreateInviteOutcome::NotOwner);
        }
        let exists: i64 = tx.query_row(
            "SELECT COUNT(*) FROM doc_invites WHERE id = ?",
            [&new.id],
            |r| r.get(0),
        )?;
        if exists > 0 {
            return Ok(CreateInviteOutcome::Duplicate);
        }
        tx.execute(
            "INSERT INTO doc_invites (
               id, doc_id, issued_by_account_id,
               wrapped_dek, wrapped_dek_nonce,
               created_at, expires_at
             ) VALUES (?, ?, ?, ?, ?, ?, ?)",
            params![
                new.id,
                doc_bytes,
                acc_bytes,
                new.wrapped_dek,
                new.wrapped_dek_nonce,
                now,
                expires_at,
            ],
        )?;
        tx.commit()?;
        Ok(CreateInviteOutcome::Created { expires_at })
    })
    .await
    .context("auth.invites create_invite")
}

#[derive(Debug, Clone)]
pub struct InviteRow {
    pub id: Vec<u8>,
    pub created_at: i64,
    pub expires_at: i64,
    pub consumed_at: Option<i64>,
}

/// Every invite on `doc_id`, newest first, for an active owner; `None`
/// when the caller is not one. Expired and consumed rows are included
/// so the client can show what happened to a link; it filters as it
/// likes.
pub async fn list_invites(
    db: &Db,
    account_id: Uuid,
    doc_id: Uuid,
) -> anyhow::Result<Option<Vec<InviteRow>>> {
    let acc_bytes = account_id.as_bytes().to_vec();
    let doc_bytes = doc_id.as_bytes().to_vec();
    db.call(move |c| {
        let tx = c.transaction()?;
        if !is_active_owner(&tx, &doc_bytes, &acc_bytes)? {
            return Ok(None);
        }
        let mut stmt = tx.prepare(
            "SELECT id, created_at, expires_at, consumed_at
             FROM doc_invites WHERE doc_id = ?
             ORDER BY created_at DESC, id",
        )?;
        let rows = stmt
            .query_map([&doc_bytes], |r| {
                Ok(InviteRow {
                    id: r.get(0)?,
                    created_at: r.get(1)?,
                    expires_at: r.get(2)?,
                    consumed_at: r.get(3)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(Some(rows))
    })
    .await
    .context("auth.invites list_invites")
}

/// Delete one invite of `doc_id`; owner only. `false` when the caller is
/// not an active owner or no such invite exists on this doc.
pub async fn revoke_invite(
    db: &Db,
    account_id: Uuid,
    doc_id: Uuid,
    invite_id: Vec<u8>,
) -> anyhow::Result<bool> {
    let acc_bytes = account_id.as_bytes().to_vec();
    let doc_bytes = doc_id.as_bytes().to_vec();
    db.call(move |c| {
        let tx = c.transaction()?;
        if !is_active_owner(&tx, &doc_bytes, &acc_bytes)? {
            return Ok(false);
        }
        let n = tx.execute(
            "DELETE FROM doc_invites WHERE id = ? AND doc_id = ?",
            params![invite_id, doc_bytes],
        )?;
        tx.commit()?;
        Ok(n > 0)
    })
    .await
    .context("auth.invites revoke_invite")
}

#[derive(Debug, Clone)]
pub struct InvitePayload {
    pub doc_id: Uuid,
    pub wrapped_dek: Vec<u8>,
    pub wrapped_dek_nonce: Vec<u8>,
    pub expires_at: i64,
    pub inviter_email: String,
}

#[derive(Debug, Clone)]
pub enum FetchInviteOutcome {
    Open(InvitePayload),
    NotFound,
    Expired,
    Consumed,
}

/// `(doc_id, wrapped_dek, nonce, expires_at, consumed_at, inviter email)`.
type InviteFetchRow = (Vec<u8>, Vec<u8>, Vec<u8>, i64, Option<i64>, String);

/// The wrap and routing data behind an id, for the unauthenticated
/// fetch. Distinguishes expired from consumed so the client can say
/// which; neither leaks anything the id holder could not already infer.
pub async fn fetch_invite(db: &Db, invite_id: Vec<u8>) -> anyhow::Result<FetchInviteOutcome> {
    let now = now_millis();
    db.call(move |c| {
        let row: Option<InviteFetchRow> = c
            .query_row(
                "SELECT i.doc_id, i.wrapped_dek, i.wrapped_dek_nonce,
                        i.expires_at, i.consumed_at, a.email
                 FROM doc_invites i
                 JOIN accounts a ON a.id = i.issued_by_account_id
                 WHERE i.id = ?",
                [&invite_id],
                |r| {
                    Ok((
                        r.get(0)?,
                        r.get(1)?,
                        r.get(2)?,
                        r.get(3)?,
                        r.get(4)?,
                        r.get(5)?,
                    ))
                },
            )
            .optional()?;
        let Some((doc_blob, wrapped_dek, wrapped_dek_nonce, expires_at, consumed_at, email)) = row
        else {
            return Ok(FetchInviteOutcome::NotFound);
        };
        if consumed_at.is_some() {
            return Ok(FetchInviteOutcome::Consumed);
        }
        if expires_at < now {
            return Ok(FetchInviteOutcome::Expired);
        }
        Ok(FetchInviteOutcome::Open(InvitePayload {
            doc_id: uuid_from_blob(doc_blob)?,
            wrapped_dek,
            wrapped_dek_nonce,
            expires_at,
            inviter_email: email,
        }))
    })
    .await
    .context("auth.invites fetch_invite")
}

#[derive(Debug, Clone)]
pub struct AcceptWrap {
    pub wrapped_dek: Vec<u8>,
    pub wrapped_dek_nonce: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AcceptInviteOutcome {
    Joined {
        doc_id: Uuid,
    },
    NotFound,
    Expired,
    Consumed,
    /// The acceptor already holds an active membership (the inviter
    /// opening their own link, say). The invite is left open for the
    /// person it was meant for.
    AlreadyMember {
        doc_id: Uuid,
    },
}

/// Validate and consume the invite and insert the acceptor's `member`
/// row in one transaction, so two holders of the same link cannot both
/// join: the second sees `Consumed`. A previously removed member
/// re-joining has their row reactivated with the new wrap.
pub async fn accept_invite(
    db: &Db,
    account_id: Uuid,
    invite_id: Vec<u8>,
    wrap: AcceptWrap,
) -> anyhow::Result<AcceptInviteOutcome> {
    let acc_bytes = account_id.as_bytes().to_vec();
    let now = now_millis();
    db.call(move |c| {
        let tx = c.transaction()?;
        let row: Option<(Vec<u8>, i64, Option<i64>)> = tx
            .query_row(
                "SELECT doc_id, expires_at, consumed_at FROM doc_invites WHERE id = ?",
                [&invite_id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()?;
        let Some((doc_bytes, expires_at, consumed_at)) = row else {
            return Ok(AcceptInviteOutcome::NotFound);
        };
        let doc_id = uuid_from_blob(doc_bytes.clone())?;
        if consumed_at.is_some() {
            return Ok(AcceptInviteOutcome::Consumed);
        }
        if expires_at < now {
            return Ok(AcceptInviteOutcome::Expired);
        }
        let existing: Option<Option<i64>> = tx
            .query_row(
                "SELECT removed_at FROM doc_members WHERE doc_id = ? AND account_id = ?",
                params![doc_bytes, acc_bytes],
                |r| r.get(0),
            )
            .optional()?;
        match existing {
            Some(None) => return Ok(AcceptInviteOutcome::AlreadyMember { doc_id }),
            Some(Some(_)) => {
                tx.execute(
                    "UPDATE doc_members
                     SET role = ?, wrapped_dek = ?, wrapped_dek_nonce = ?,
                         recovery_wrapped_dek = NULL, recovery_wrapped_dek_nonce = NULL,
                         added_at = ?, removed_at = NULL
                     WHERE doc_id = ? AND account_id = ?",
                    params![
                        MemberRole::Member.as_str(),
                        wrap.wrapped_dek,
                        wrap.wrapped_dek_nonce,
                        now,
                        doc_bytes,
                        acc_bytes,
                    ],
                )?;
            }
            None => {
                tx.execute(
                    "INSERT INTO doc_members (
                       doc_id, account_id, role,
                       wrapped_dek, wrapped_dek_nonce,
                       added_at
                     ) VALUES (?, ?, ?, ?, ?, ?)",
                    params![
                        doc_bytes,
                        acc_bytes,
                        MemberRole::Member.as_str(),
                        wrap.wrapped_dek,
                        wrap.wrapped_dek_nonce,
                        now,
                    ],
                )?;
            }
        }
        tx.execute(
            "UPDATE doc_invites SET consumed_at = ?, consumed_by_account_id = ? WHERE id = ?",
            params![now, acc_bytes, invite_id],
        )?;
        tx.commit()?;
        Ok(AcceptInviteOutcome::Joined { doc_id })
    })
    .await
    .context("auth.invites accept_invite")
}

#[derive(Debug, Clone)]
pub struct MemberRow {
    pub account_id: Uuid,
    pub email: String,
    pub role: MemberRole,
    pub added_at: i64,
}

/// Active members of `doc_id`, oldest first, for any active member of
/// it; `None` when the caller is not one.
pub async fn list_members(
    db: &Db,
    account_id: Uuid,
    doc_id: Uuid,
) -> anyhow::Result<Option<Vec<MemberRow>>> {
    let acc_bytes = account_id.as_bytes().to_vec();
    let doc_bytes = doc_id.as_bytes().to_vec();
    db.call(move |c| {
        let tx = c.transaction()?;
        let member: i64 = tx.query_row(
            "SELECT COUNT(*) FROM doc_members
             WHERE doc_id = ? AND account_id = ? AND removed_at IS NULL",
            params![&doc_bytes, &acc_bytes],
            |r| r.get(0),
        )?;
        if member == 0 {
            return Ok(None);
        }
        let mut stmt = tx.prepare(
            "SELECT m.account_id, a.email, m.role, m.added_at
             FROM doc_members m JOIN accounts a ON a.id = m.account_id
             WHERE m.doc_id = ? AND m.removed_at IS NULL
             ORDER BY m.added_at, m.account_id",
        )?;
        let rows = stmt
            .query_map([&doc_bytes], |r| {
                let role_text: String = r.get(2)?;
                let role = MemberRole::parse(&role_text).ok_or_else(|| {
                    rusqlite::Error::FromSqlConversionFailure(
                        2,
                        rusqlite::types::Type::Text,
                        Box::<dyn std::error::Error + Send + Sync + 'static>::from(format!(
                            "unknown doc_members.role {role_text:?}"
                        )),
                    )
                })?;
                Ok(MemberRow {
                    account_id: uuid_from_blob(r.get::<_, Vec<u8>>(0)?)?,
                    email: r.get(1)?,
                    role,
                    added_at: r.get(3)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(Some(rows))
    })
    .await
    .context("auth.invites list_members")
}

/// What `remove_member` found; the route maps each to a status.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RemoveMemberOutcome {
    Removed,
    /// The caller holds no active membership (404: learns nothing).
    CallerNotMember,
    /// The caller is a member but not an owner (403).
    CallerNotOwner,
    /// The target is the caller: leaving is a separate verb (409).
    SelfRemoval,
    /// The target is the doc's only active owner (409).
    LastOwner,
    /// The target holds no active membership (404).
    TargetNotMember,
}

/// Owner removes a member (`spec/sharing-plan.md` §Member removal):
/// stamps `removed_at` on the target's row. Forward-only: the row
/// stays (an accepted invite reactivates it) and the removed account
/// keeps whatever it already pulled. The horizon, `is_member` and the
/// member list all filter on `removed_at IS NULL`, so the server
/// stops serving the doc to that account from this commit; live
/// connections are told by the caller (`SyncSessions::close_for_account`).
pub async fn remove_member(
    db: &Db,
    caller: Uuid,
    doc_id: Uuid,
    target: Uuid,
) -> anyhow::Result<RemoveMemberOutcome> {
    let caller_bytes = caller.as_bytes().to_vec();
    let target_bytes = target.as_bytes().to_vec();
    let doc_bytes = doc_id.as_bytes().to_vec();
    db.call(move |c| {
        let tx = c.transaction()?;
        let caller_role: Option<String> = tx
            .query_row(
                "SELECT role FROM doc_members
                 WHERE doc_id = ? AND account_id = ? AND removed_at IS NULL",
                params![&doc_bytes, &caller_bytes],
                |r| r.get(0),
            )
            .optional()?;
        match caller_role.as_deref().and_then(MemberRole::parse) {
            None => return Ok(RemoveMemberOutcome::CallerNotMember),
            Some(MemberRole::Member) => return Ok(RemoveMemberOutcome::CallerNotOwner),
            Some(MemberRole::Owner) => {}
        }
        if caller == target {
            return Ok(RemoveMemberOutcome::SelfRemoval);
        }
        let target_role: Option<String> = tx
            .query_row(
                "SELECT role FROM doc_members
                 WHERE doc_id = ? AND account_id = ? AND removed_at IS NULL",
                params![&doc_bytes, &target_bytes],
                |r| r.get(0),
            )
            .optional()?;
        let Some(target_role) = target_role.as_deref().and_then(MemberRole::parse) else {
            return Ok(RemoveMemberOutcome::TargetNotMember);
        };
        if target_role == MemberRole::Owner {
            let owners: i64 = tx.query_row(
                "SELECT COUNT(*) FROM doc_members
                 WHERE doc_id = ? AND role = ? AND removed_at IS NULL",
                params![&doc_bytes, MemberRole::Owner.as_str()],
                |r| r.get(0),
            )?;
            if owners <= 1 {
                return Ok(RemoveMemberOutcome::LastOwner);
            }
        }
        tx.execute(
            "UPDATE doc_members SET removed_at = ?
             WHERE doc_id = ? AND account_id = ? AND removed_at IS NULL",
            params![now_millis(), &doc_bytes, &target_bytes],
        )?;
        tx.commit()?;
        Ok(RemoveMemberOutcome::Removed)
    })
    .await
    .context("auth.invites remove_member")
}

fn is_active_owner(
    tx: &rusqlite::Transaction<'_>,
    doc_bytes: &[u8],
    acc_bytes: &[u8],
) -> rusqlite::Result<bool> {
    let n: i64 = tx.query_row(
        "SELECT COUNT(*) FROM doc_members
         WHERE doc_id = ? AND account_id = ? AND role = ? AND removed_at IS NULL",
        params![doc_bytes, acc_bytes, MemberRole::Owner.as_str()],
        |r| r.get(0),
    )?;
    Ok(n > 0)
}

fn uuid_from_blob(b: Vec<u8>) -> rusqlite::Result<Uuid> {
    let arr: [u8; 16] = b.try_into().map_err(|_| {
        rusqlite::Error::FromSqlConversionFailure(
            0,
            rusqlite::types::Type::Blob,
            Box::<dyn std::error::Error + Send + Sync + 'static>::from("invalid uuid blob"),
        )
    })?;
    Ok(Uuid::from_bytes(arr))
}
