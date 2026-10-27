//! All sqlite reads/writes touching accounts / memberships / devices /
//! recovery sessions.

use std::collections::BTreeSet;

use anyhow::Context;
use monoplan_protocol::{KdfParams, MemberRole};
use rusqlite::{OptionalExtension, Transaction, params};
use uuid::Uuid;

use crate::db::{Db, now_millis};

#[derive(Debug, Clone)]
pub struct AccountRow {
    pub id: Uuid,
    pub email: String,
    pub password_hash: Vec<u8>,
    pub password_salt: Vec<u8>,
    pub kdf_params: KdfParams,
    pub primary_doc_id: Uuid,
    pub recovery_salt: Option<Vec<u8>>,
    pub recovery_auth_hash: Option<Vec<u8>>,
}

impl AccountRow {
    pub fn recovery_present(&self) -> bool {
        self.recovery_auth_hash.is_some()
    }
}

#[derive(Debug, Clone)]
pub struct DeviceRow {
    pub id: Uuid,
    pub account_id: Uuid,
    pub name: String,
    pub kind: String,
    pub last_seen_at: i64,
    /// IP the device last connected from, as seen by the server (or
    /// the proxy in front of it). `None` until first authed request.
    pub last_seen_ip: Option<String>,
    pub created_at: i64,
}

/// One row of `doc_members` as seen from an account: the doc, the role,
/// and this account's wraps of the doc's DEK. On the account's primary
/// membership `wrapped_dek` is the Home DEK under the password KEK and
/// `recovery_wrapped_dek` (iff enrolled) the Home DEK under the recovery
/// KEK; on every other row `wrapped_dek` is that doc's DEK under the
/// Home DEK and the recovery columns are NULL. The server stores both
/// kinds opaquely (`spec/encryption.md` §Wrap-states).
#[derive(Debug, Clone)]
pub struct MembershipRow {
    pub doc_id: Uuid,
    pub role: MemberRole,
    pub wrapped_dek: Vec<u8>,
    pub wrapped_dek_nonce: Vec<u8>,
    pub recovery_wrapped_dek: Option<Vec<u8>>,
    pub recovery_wrapped_dek_nonce: Option<Vec<u8>>,
}

#[derive(Debug, Clone)]
pub struct NewAccount {
    pub email: String,
    pub password_hash: Vec<u8>,
    pub password_salt: Vec<u8>,
    pub kdf_params: KdfParams,
    /// The Home DEK (primary doc's DEK) under the password KEK — goes
    /// on the signup `owner` membership row, not the account.
    pub wrapped_dek: Vec<u8>,
    pub wrapped_dek_nonce: Vec<u8>,
    pub recovery_salt: Option<Vec<u8>>,
    pub recovery_auth_hash: Option<Vec<u8>>,
    /// The Home DEK under the recovery KEK; same membership row.
    pub recovery_wrapped_dek: Option<Vec<u8>>,
    pub recovery_wrapped_dek_nonce: Option<Vec<u8>>,
    /// Initial device created in the same transaction as the account.
    pub device_name: String,
    pub device_kind: String,
    pub device_token_hash: Vec<u8>,
    /// Client-chosen primary doc id (`SignupRequest::primary_doc_id`);
    /// minted here when absent.
    pub primary_doc_id: Option<Uuid>,
}

#[derive(Debug, Clone)]
pub struct CreatedAccount {
    pub account_id: Uuid,
    pub device_id: Uuid,
    pub primary_doc_id: Uuid,
}

pub async fn create_account(db: &Db, new: NewAccount) -> anyhow::Result<CreatedAccount> {
    let account_id = Uuid::now_v7();
    let device_id = Uuid::now_v7();
    let primary_doc_id = new.primary_doc_id.unwrap_or_else(Uuid::now_v7);
    let now = now_millis();
    let acc_bytes = account_id.as_bytes().to_vec();
    let dev_bytes = device_id.as_bytes().to_vec();
    let doc_bytes = primary_doc_id.as_bytes().to_vec();
    let dup_marker = "UNIQUE constraint failed: accounts.email";
    let doc_dup_marker = "UNIQUE constraint failed: docs.id";
    let res: anyhow::Result<()> = db
        .call(move |c| {
            let tx = c.transaction()?;
            // Mint the primary doc first so the accounts FK is satisfiable.
            tx.execute(
                "INSERT INTO docs (id, created_at) VALUES (?, ?)",
                params![doc_bytes, now],
            )?;
            tx.execute(
                "INSERT INTO accounts (
                   id, email, password_hash, password_salt,
                   kdf_m_kib, kdf_t, kdf_p,
                   primary_doc_id,
                   recovery_salt, recovery_auth_hash,
                   created_at
                 ) VALUES (
                   ?, ?, ?, ?,
                   ?, ?, ?,
                   ?,
                   ?, ?,
                   ?
                 )",
                params![
                    acc_bytes,
                    new.email,
                    new.password_hash,
                    new.password_salt,
                    new.kdf_params.m_kib as i64,
                    new.kdf_params.t as i64,
                    new.kdf_params.p as i64,
                    doc_bytes,
                    new.recovery_salt,
                    new.recovery_auth_hash,
                    now,
                ],
            )?;
            // The account's one guaranteed membership: owner of its
            // primary doc, carrying the wraps the client posted.
            tx.execute(
                "INSERT INTO doc_members (
                   doc_id, account_id, role,
                   wrapped_dek, wrapped_dek_nonce,
                   recovery_wrapped_dek, recovery_wrapped_dek_nonce,
                   added_at
                 ) VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
                params![
                    doc_bytes,
                    acc_bytes,
                    MemberRole::Owner.as_str(),
                    new.wrapped_dek,
                    new.wrapped_dek_nonce,
                    new.recovery_wrapped_dek,
                    new.recovery_wrapped_dek_nonce,
                    now,
                ],
            )?;
            tx.execute(
                "INSERT INTO devices (id, account_id, name, kind, auth_token_hash, last_seen_at, created_at)
                 VALUES (?, ?, ?, ?, ?, ?, ?)",
                params![
                    dev_bytes,
                    acc_bytes,
                    new.device_name,
                    new.device_kind,
                    new.device_token_hash,
                    now,
                    now,
                ],
            )?;
            tx.commit()?;
            Ok(())
        })
        .await
        .map_err(|e: anyhow::Error| {
            // Surface unique-violation as a typed error caller can detect.
            if e.to_string().contains(dup_marker) {
                anyhow::anyhow!("account_exists")
            } else if e.to_string().contains(doc_dup_marker) {
                anyhow::anyhow!("doc_exists")
            } else {
                e
            }
        });
    res?;
    Ok(CreatedAccount {
        account_id,
        device_id,
        primary_doc_id,
    })
}

pub async fn find_account_by_email(db: &Db, email: String) -> anyhow::Result<Option<AccountRow>> {
    db.call(move |c| {
        c.query_row(
            "SELECT id, email, password_hash, password_salt,
                    kdf_m_kib, kdf_t, kdf_p,
                    primary_doc_id,
                    recovery_salt, recovery_auth_hash
             FROM accounts WHERE email = ?",
            [email],
            row_to_account,
        )
        .optional()
    })
    .await
}

pub async fn find_account_by_id(db: &Db, account_id: Uuid) -> anyhow::Result<Option<AccountRow>> {
    let id_bytes = account_id.as_bytes().to_vec();
    db.call(move |c| {
        c.query_row(
            "SELECT id, email, password_hash, password_salt,
                    kdf_m_kib, kdf_t, kdf_p,
                    primary_doc_id,
                    recovery_salt, recovery_auth_hash
             FROM accounts WHERE id = ?",
            [id_bytes],
            row_to_account,
        )
        .optional()
    })
    .await
}

/// Active memberships of `account_id`, oldest first.
pub async fn list_memberships(db: &Db, account_id: Uuid) -> anyhow::Result<Vec<MembershipRow>> {
    let acc_bytes = account_id.as_bytes().to_vec();
    db.call(move |c| {
        let mut stmt = c.prepare(
            "SELECT doc_id, role, wrapped_dek, wrapped_dek_nonce,
                    recovery_wrapped_dek, recovery_wrapped_dek_nonce
             FROM doc_members
             WHERE account_id = ? AND removed_at IS NULL
             ORDER BY added_at, doc_id",
        )?;
        let rows = stmt
            .query_map([acc_bytes], row_to_membership)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    })
    .await
}

/// The wrap for a doc created by an existing account (`POST /api/docs`):
/// the new doc's DEK under the creator's Home DEK. Non-primary rows
/// never carry a recovery wrap.
#[derive(Debug, Clone)]
pub struct NewDoc {
    pub wrapped_dek: Vec<u8>,
    pub wrapped_dek_nonce: Vec<u8>,
}

/// Mint a doc and its `owner` membership for `account_id` in one
/// transaction. Returns the new doc id.
pub async fn create_doc(db: &Db, account_id: Uuid, new: NewDoc) -> anyhow::Result<Uuid> {
    let doc_id = Uuid::now_v7();
    let now = now_millis();
    let acc_bytes = account_id.as_bytes().to_vec();
    let doc_bytes = doc_id.as_bytes().to_vec();
    db.call(move |c| {
        let tx = c.transaction()?;
        tx.execute(
            "INSERT INTO docs (id, created_at) VALUES (?, ?)",
            params![doc_bytes, now],
        )?;
        tx.execute(
            "INSERT INTO doc_members (
               doc_id, account_id, role,
               wrapped_dek, wrapped_dek_nonce,
               added_at
             ) VALUES (?, ?, ?, ?, ?, ?)",
            params![
                doc_bytes,
                acc_bytes,
                MemberRole::Owner.as_str(),
                new.wrapped_dek,
                new.wrapped_dek_nonce,
                now,
            ],
        )?;
        tx.commit()?;
        Ok(())
    })
    .await?;
    Ok(doc_id)
}

/// Result of [`delete_doc`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeleteDocOutcome {
    Deleted,
    /// No active `owner` membership for this account on this doc (the
    /// doc does not exist, or the caller is not its owner). Reported as
    /// not-found so a non-member learns nothing.
    NotOwner,
}

/// Delete a doc the caller owns, with every row that hangs off it: updates,
/// snapshots, per-device frontiers, the sequence counter, the invites
/// and the memberships. One transaction. Deletion is unilateral: other
/// active members lose the doc too (`spec/sharing-plan.md` §Deletion);
/// the route tells their live sessions with `DocClosed`, and anything
/// they already synced stays on their devices. The caller is
/// responsible for refusing the account's primary doc *before* calling
/// (a request-level decision, and the `accounts.primary_doc_id` FK would
/// reject the delete anyway).
pub async fn delete_doc(
    db: &Db,
    account_id: Uuid,
    doc_id: Uuid,
) -> anyhow::Result<DeleteDocOutcome> {
    let acc_bytes = account_id.as_bytes().to_vec();
    let doc_bytes = doc_id.as_bytes().to_vec();
    db.call(move |c| {
        let tx = c.transaction()?;
        let owner: i64 = tx.query_row(
            "SELECT COUNT(*) FROM doc_members
             WHERE doc_id = ? AND account_id = ? AND role = ? AND removed_at IS NULL",
            params![doc_bytes, acc_bytes, MemberRole::Owner.as_str()],
            |r| r.get(0),
        )?;
        if owner == 0 {
            return Ok(DeleteDocOutcome::NotOwner);
        }
        for sql in [
            "DELETE FROM updates WHERE doc_id = ?",
            "DELETE FROM snapshots WHERE doc_id = ?",
            "DELETE FROM device_doc_frontiers WHERE doc_id = ?",
            "DELETE FROM doc_sequences WHERE doc_id = ?",
            "DELETE FROM doc_invites WHERE doc_id = ?",
            "DELETE FROM doc_members WHERE doc_id = ?",
            "DELETE FROM docs WHERE id = ?",
        ] {
            tx.execute(sql, params![doc_bytes])?;
        }
        tx.commit()?;
        Ok(DeleteDocOutcome::Deleted)
    })
    .await
    .context("auth.queries delete_doc")
}

/// Does `account_id` hold an active membership on `doc_id`? The single
/// access decision behind `OpenDoc` (`sync/ws.rs::can_open`).
pub async fn is_member(db: &Db, doc_id: Uuid, account_id: Uuid) -> anyhow::Result<bool> {
    let doc_bytes = doc_id.as_bytes().to_vec();
    let acc_bytes = account_id.as_bytes().to_vec();
    db.call(move |c| {
        let n: i64 = c.query_row(
            "SELECT COUNT(*) FROM doc_members
             WHERE doc_id = ? AND account_id = ? AND removed_at IS NULL",
            params![doc_bytes, acc_bytes],
            |r| r.get(0),
        )?;
        Ok(n > 0)
    })
    .await
}

/// The Home DEK re-wrapped under the account's *new* password KEK,
/// tagged with the doc it belongs to (the primary doc).
#[derive(Debug, Clone)]
pub struct DocKeyUpdate {
    pub doc_id: Uuid,
    pub wrapped_dek: Vec<u8>,
    pub wrapped_dek_nonce: Vec<u8>,
}

/// New password material plus the re-wrap of the Home DEK. The KEK is
/// changing, and the Home DEK is the only key it wraps, so the update
/// is rejected unless `new_doc_keys` names exactly the account's
/// primary doc (see [`PasswordUpdateOutcome::DocKeysMismatch`]). Every
/// other membership is wrapped under the Home DEK and is left alone.
#[derive(Debug, Clone)]
pub struct PasswordUpdate {
    pub new_password_hash: Vec<u8>,
    pub new_password_salt: Vec<u8>,
    pub new_kdf_params: KdfParams,
    pub new_doc_keys: Vec<DocKeyUpdate>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PasswordUpdateOutcome {
    Updated,
    /// `new_doc_keys` did not name exactly the primary doc. Nothing was
    /// written.
    DocKeysMismatch {
        expected: Vec<Uuid>,
        got: Vec<Uuid>,
    },
}

/// Logged-in password change. One transaction: password material on
/// `accounts`, then the primary membership's `wrapped_dek`.
pub async fn update_password(
    db: &Db,
    account_id: Uuid,
    u: PasswordUpdate,
) -> anyhow::Result<PasswordUpdateOutcome> {
    db.call(move |c| {
        let tx = c.transaction()?;
        let outcome = apply_password_update(&tx, account_id, &u)?;
        if outcome == PasswordUpdateOutcome::Updated {
            tx.commit()?;
        }
        Ok(outcome)
    })
    .await
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PasswordResetOutcome {
    Updated {
        account_id: Uuid,
    },
    /// No such session, already consumed, or expired. Nothing written.
    SessionInvalid,
    /// Session was valid but `new_doc_keys` did not match; nothing
    /// written — the session is *not* consumed, so the client can retry
    /// with a corrected set inside the TTL.
    DocKeysMismatch {
        expected: Vec<Uuid>,
        got: Vec<Uuid>,
    },
}

/// Recovery-path password reset: validate + consume the recovery
/// session and apply the password update in one transaction, so a
/// rejected update never burns the session.
pub async fn reset_password_with_recovery_session(
    db: &Db,
    token_hash: Vec<u8>,
    u: PasswordUpdate,
) -> anyhow::Result<PasswordResetOutcome> {
    let now = now_millis();
    db.call(move |c| {
        let tx = c.transaction()?;
        let row: Option<(Vec<u8>, i64, Option<i64>)> = tx
            .query_row(
                "SELECT account_id, expires_at, consumed_at
                 FROM recovery_sessions WHERE token_hash = ?",
                [&token_hash],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()?;
        let Some((acc_blob, expires_at, consumed_at)) = row else {
            return Ok(PasswordResetOutcome::SessionInvalid);
        };
        if consumed_at.is_some() || expires_at < now {
            return Ok(PasswordResetOutcome::SessionInvalid);
        }
        let account_id = uuid_from_blob(acc_blob)?;
        match apply_password_update(&tx, account_id, &u)? {
            PasswordUpdateOutcome::DocKeysMismatch { expected, got } => {
                Ok(PasswordResetOutcome::DocKeysMismatch { expected, got })
            }
            PasswordUpdateOutcome::Updated => {
                tx.execute(
                    "UPDATE recovery_sessions SET consumed_at = ? WHERE token_hash = ?",
                    params![now, token_hash],
                )?;
                tx.commit()?;
                Ok(PasswordResetOutcome::Updated { account_id })
            }
        }
    })
    .await
}

/// Shared body of change + reset. Checks that `new_doc_keys` names
/// exactly the account's primary doc, then writes password material
/// and the primary membership's wrap. Returns without writing on a
/// mismatch; the caller decides whether to commit.
///
/// Only the primary row is touched: it is the one row whose wrap is
/// under the password KEK. A non-primary membership's wrap is under the
/// Home DEK, which a password change does not alter, so re-wrapping it
/// here would be wrong (the client would have to upload a Home-DEK
/// wrap under a `new_doc_keys` name that promises a KEK wrap).
fn apply_password_update(
    tx: &Transaction<'_>,
    account_id: Uuid,
    u: &PasswordUpdate,
) -> rusqlite::Result<PasswordUpdateOutcome> {
    let acc_bytes = account_id.as_bytes().to_vec();
    let primary_doc_id = tx.query_row(
        "SELECT primary_doc_id FROM accounts WHERE id = ?",
        [&acc_bytes],
        |r| uuid_from_blob(r.get::<_, Vec<u8>>(0)?),
    )?;
    let expected: BTreeSet<Uuid> = [primary_doc_id].into_iter().collect();
    let got: BTreeSet<Uuid> = u.new_doc_keys.iter().map(|k| k.doc_id).collect();
    if expected != got || got.len() != u.new_doc_keys.len() {
        return Ok(PasswordUpdateOutcome::DocKeysMismatch {
            expected: expected.into_iter().collect(),
            got: u.new_doc_keys.iter().map(|k| k.doc_id).collect(),
        });
    }
    tx.execute(
        "UPDATE accounts
         SET password_hash = ?, password_salt = ?,
             kdf_m_kib = ?, kdf_t = ?, kdf_p = ?
         WHERE id = ?",
        params![
            u.new_password_hash,
            u.new_password_salt,
            u.new_kdf_params.m_kib as i64,
            u.new_kdf_params.t as i64,
            u.new_kdf_params.p as i64,
            acc_bytes,
        ],
    )?;
    // Exactly one key after the set check above.
    for k in &u.new_doc_keys {
        let n = tx.execute(
            "UPDATE doc_members SET wrapped_dek = ?, wrapped_dek_nonce = ?
             WHERE doc_id = ? AND account_id = ? AND removed_at IS NULL",
            params![
                k.wrapped_dek,
                k.wrapped_dek_nonce,
                k.doc_id.as_bytes().to_vec(),
                acc_bytes,
            ],
        )?;
        // The primary membership is non-leavable, so an active row must
        // exist; anything else is a broken invariant, not a client error.
        if n != 1 {
            return Err(rusqlite::Error::StatementChangedRows(n));
        }
    }
    Ok(PasswordUpdateOutcome::Updated)
}

pub async fn create_device(
    db: &Db,
    account_id: Uuid,
    name: String,
    kind: String,
    token_hash: Vec<u8>,
) -> anyhow::Result<Uuid> {
    let device_id = Uuid::now_v7();
    let id_bytes = device_id.as_bytes().to_vec();
    let acc_bytes = account_id.as_bytes().to_vec();
    let now = now_millis();
    db.call(move |c| {
        c.execute(
            "INSERT INTO devices (id, account_id, name, kind, auth_token_hash, last_seen_at, created_at)
             VALUES (?, ?, ?, ?, ?, ?, ?)",
            params![id_bytes, acc_bytes, name, kind, token_hash, now, now],
        )
    })
    .await?;
    Ok(device_id)
}

/// List an account's devices.
pub async fn list_devices(db: &Db, account_id: Uuid) -> anyhow::Result<Vec<DeviceRow>> {
    let acc_bytes = account_id.as_bytes().to_vec();
    db.call(move |c| {
        let mut stmt = c.prepare(
            "SELECT id, account_id, name, last_seen_at, created_at, last_seen_ip, kind
             FROM devices WHERE account_id = ? ORDER BY created_at",
        )?;
        let rows = stmt
            .query_map(params![acc_bytes], row_to_device)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    })
    .await
}

pub async fn revoke_device(db: &Db, account_id: Uuid, device_id: Uuid) -> anyhow::Result<bool> {
    let acc_bytes = account_id.as_bytes().to_vec();
    let dev_bytes = device_id.as_bytes().to_vec();
    let n = db
        .call(move |c| {
            c.execute(
                "DELETE FROM devices WHERE id = ? AND account_id = ?",
                params![dev_bytes, acc_bytes],
            )
        })
        .await
        .context("auth.queries revoke_device delete row")?;
    Ok(n > 0)
}

pub async fn rename_device(
    db: &Db,
    account_id: Uuid,
    device_id: Uuid,
    name: String,
) -> anyhow::Result<bool> {
    let acc_bytes = account_id.as_bytes().to_vec();
    let dev_bytes = device_id.as_bytes().to_vec();
    let n = db
        .call(move |c| {
            c.execute(
                "UPDATE devices SET name = ? WHERE id = ? AND account_id = ?",
                params![name, dev_bytes, acc_bytes],
            )
        })
        .await
        .context("auth.queries rename_device update row")?;
    Ok(n > 0)
}

#[derive(Debug, Clone)]
pub struct DeviceLookup {
    pub account_id: Uuid,
    pub device_id: Uuid,
    pub primary_doc_id: Uuid,
}

pub async fn find_device_by_token_hash(
    db: &Db,
    token_hash: Vec<u8>,
) -> anyhow::Result<Option<DeviceLookup>> {
    db.call(move |c| {
        c.query_row(
            "SELECT d.id, d.account_id, a.primary_doc_id
             FROM devices d
             JOIN accounts a ON a.id = d.account_id
             WHERE d.auth_token_hash = ?",
            [token_hash],
            |r| {
                let id = uuid_from_blob(r.get::<_, Vec<u8>>(0)?)?;
                let account_id = uuid_from_blob(r.get::<_, Vec<u8>>(1)?)?;
                let primary_doc_id = uuid_from_blob(r.get::<_, Vec<u8>>(2)?)?;
                Ok(DeviceLookup {
                    account_id,
                    device_id: id,
                    primary_doc_id,
                })
            },
        )
        .optional()
    })
    .await
}

/// Bump `last_seen_at` and record the client IP the request arrived
/// from. `ip == None` (no connect info, unparseable forwarded header)
/// leaves the stored IP untouched rather than clearing it.
pub async fn touch_device_last_seen(
    db: &Db,
    device_id: Uuid,
    ip: Option<String>,
) -> anyhow::Result<()> {
    let id_bytes = device_id.as_bytes().to_vec();
    let now = now_millis();
    db.call(move |c| {
        c.execute(
            "UPDATE devices SET last_seen_at = ?, last_seen_ip = COALESCE(?, last_seen_ip) WHERE id = ?",
            params![now, ip, id_bytes],
        )
    })
    .await?;
    Ok(())
}

pub async fn create_recovery_session(
    db: &Db,
    account_id: Uuid,
    token_hash: Vec<u8>,
    ttl_millis: i64,
) -> anyhow::Result<()> {
    let acc_bytes = account_id.as_bytes().to_vec();
    let expires_at = now_millis() + ttl_millis;
    db.call(move |c| {
        c.execute(
            "INSERT INTO recovery_sessions (token_hash, account_id, expires_at) VALUES (?, ?, ?)",
            params![token_hash, acc_bytes, expires_at],
        )
    })
    .await?;
    Ok(())
}

fn row_to_account(r: &rusqlite::Row<'_>) -> rusqlite::Result<AccountRow> {
    Ok(AccountRow {
        id: uuid_from_blob(r.get::<_, Vec<u8>>(0)?)?,
        email: r.get(1)?,
        password_hash: r.get(2)?,
        password_salt: r.get(3)?,
        kdf_params: KdfParams {
            m_kib: r.get::<_, i64>(4)? as u32,
            t: r.get::<_, i64>(5)? as u32,
            p: r.get::<_, i64>(6)? as u32,
        },
        primary_doc_id: uuid_from_blob(r.get::<_, Vec<u8>>(7)?)?,
        recovery_salt: r.get(8)?,
        recovery_auth_hash: r.get(9)?,
    })
}

fn row_to_membership(r: &rusqlite::Row<'_>) -> rusqlite::Result<MembershipRow> {
    let role_text: String = r.get(1)?;
    let role = MemberRole::parse(&role_text).ok_or_else(|| {
        rusqlite::Error::FromSqlConversionFailure(
            1,
            rusqlite::types::Type::Text,
            Box::<dyn std::error::Error + Send + Sync + 'static>::from(format!(
                "unknown doc_members.role {role_text:?}"
            )),
        )
    })?;
    Ok(MembershipRow {
        doc_id: uuid_from_blob(r.get::<_, Vec<u8>>(0)?)?,
        role,
        wrapped_dek: r.get(2)?,
        wrapped_dek_nonce: r.get(3)?,
        recovery_wrapped_dek: r.get(4)?,
        recovery_wrapped_dek_nonce: r.get(5)?,
    })
}

fn row_to_device(r: &rusqlite::Row<'_>) -> rusqlite::Result<DeviceRow> {
    Ok(DeviceRow {
        id: uuid_from_blob(r.get::<_, Vec<u8>>(0)?)?,
        account_id: uuid_from_blob(r.get::<_, Vec<u8>>(1)?)?,
        name: r.get(2)?,
        last_seen_at: r.get(3)?,
        created_at: r.get(4)?,
        last_seen_ip: r.get(5)?,
        kind: r.get(6)?,
    })
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
