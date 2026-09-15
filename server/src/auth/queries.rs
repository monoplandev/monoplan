//! All sqlite reads/writes touching accounts / devices / recovery sessions.

use anyhow::Context;
use monoplan_protocol::KdfParams;
use rusqlite::{OptionalExtension, params};
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
    pub wrapped_dek: Vec<u8>,
    pub wrapped_dek_nonce: Vec<u8>,
    pub recovery_salt: Option<Vec<u8>>,
    pub recovery_auth_hash: Option<Vec<u8>>,
    pub recovery_wrapped_dek: Option<Vec<u8>>,
    pub recovery_wrapped_dek_nonce: Option<Vec<u8>>,
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
    pub last_seen_at: i64,
    /// IP the device last connected from, as seen by the server (or
    /// the proxy in front of it). `None` until first authed request.
    pub last_seen_ip: Option<String>,
    pub created_at: i64,
    pub last_acked_seq: i64,
}

#[derive(Debug, Clone)]
pub struct NewAccount {
    pub email: String,
    pub password_hash: Vec<u8>,
    pub password_salt: Vec<u8>,
    pub kdf_params: KdfParams,
    pub wrapped_dek: Vec<u8>,
    pub wrapped_dek_nonce: Vec<u8>,
    pub recovery_salt: Option<Vec<u8>>,
    pub recovery_auth_hash: Option<Vec<u8>>,
    pub recovery_wrapped_dek: Option<Vec<u8>>,
    pub recovery_wrapped_dek_nonce: Option<Vec<u8>>,
    /// Initial device created in the same transaction as the account.
    pub device_name: String,
    pub device_token_hash: Vec<u8>,
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
    let primary_doc_id = Uuid::now_v7();
    let now = now_millis();
    let acc_bytes = account_id.as_bytes().to_vec();
    let dev_bytes = device_id.as_bytes().to_vec();
    let doc_bytes = primary_doc_id.as_bytes().to_vec();
    let dup_marker = "UNIQUE constraint failed: accounts.email";
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
                   wrapped_dek, wrapped_dek_nonce,
                   recovery_salt, recovery_auth_hash,
                   recovery_wrapped_dek, recovery_wrapped_dek_nonce,
                   created_at
                 ) VALUES (
                   ?, ?, ?, ?,
                   ?, ?, ?,
                   ?,
                   ?, ?,
                   ?, ?,
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
                    new.wrapped_dek,
                    new.wrapped_dek_nonce,
                    new.recovery_salt,
                    new.recovery_auth_hash,
                    new.recovery_wrapped_dek,
                    new.recovery_wrapped_dek_nonce,
                    now,
                ],
            )?;
            tx.execute(
                "INSERT INTO devices (id, account_id, name, auth_token_hash, last_acked_seq, last_seen_at, created_at)
                 VALUES (?, ?, ?, ?, 0, ?, ?)",
                params![
                    dev_bytes,
                    acc_bytes,
                    new.device_name,
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
                    wrapped_dek, wrapped_dek_nonce,
                    recovery_salt, recovery_auth_hash,
                    recovery_wrapped_dek, recovery_wrapped_dek_nonce
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
                    wrapped_dek, wrapped_dek_nonce,
                    recovery_salt, recovery_auth_hash,
                    recovery_wrapped_dek, recovery_wrapped_dek_nonce
             FROM accounts WHERE id = ?",
            [id_bytes],
            row_to_account,
        )
        .optional()
    })
    .await
}

#[derive(Debug, Clone)]
pub struct PasswordUpdate {
    pub account_id: Uuid,
    pub new_password_hash: Vec<u8>,
    pub new_password_salt: Vec<u8>,
    pub new_kdf_params: KdfParams,
    pub new_wrapped_dek: Vec<u8>,
    pub new_wrapped_dek_nonce: Vec<u8>,
}

pub async fn update_password(db: &Db, u: PasswordUpdate) -> anyhow::Result<()> {
    let id_bytes = u.account_id.as_bytes().to_vec();
    db.call(move |c| {
        c.execute(
            "UPDATE accounts
             SET password_hash = ?, password_salt = ?,
                 kdf_m_kib = ?, kdf_t = ?, kdf_p = ?,
                 wrapped_dek = ?, wrapped_dek_nonce = ?
             WHERE id = ?",
            params![
                u.new_password_hash,
                u.new_password_salt,
                u.new_kdf_params.m_kib as i64,
                u.new_kdf_params.t as i64,
                u.new_kdf_params.p as i64,
                u.new_wrapped_dek,
                u.new_wrapped_dek_nonce,
                id_bytes,
            ],
        )
    })
    .await?;
    Ok(())
}

pub async fn create_device(
    db: &Db,
    account_id: Uuid,
    name: String,
    token_hash: Vec<u8>,
) -> anyhow::Result<Uuid> {
    let device_id = Uuid::now_v7();
    let id_bytes = device_id.as_bytes().to_vec();
    let acc_bytes = account_id.as_bytes().to_vec();
    let now = now_millis();
    db.call(move |c| {
        c.execute(
            "INSERT INTO devices (id, account_id, name, auth_token_hash, last_acked_seq, last_seen_at, created_at)
             VALUES (?, ?, ?, ?, 0, ?, ?)",
            params![id_bytes, acc_bytes, name, token_hash, now, now],
        )
    })
    .await?;
    Ok(device_id)
}

pub async fn list_devices(db: &Db, account_id: Uuid) -> anyhow::Result<Vec<DeviceRow>> {
    let acc_bytes = account_id.as_bytes().to_vec();
    db.call(move |c| {
        let mut stmt = c.prepare(
            "SELECT id, account_id, name, last_seen_at, created_at, last_acked_seq, last_seen_ip
             FROM devices WHERE account_id = ? ORDER BY created_at",
        )?;
        let rows = stmt
            .query_map([acc_bytes], row_to_device)?
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

#[derive(Debug, Clone)]
pub struct ConsumedRecoverySession {
    pub account_id: Uuid,
}

/// Atomically validate + consume a recovery session token. Returns
/// `Some(account_id)` only if the session existed, was not previously
/// consumed, and has not expired. Sets `consumed_at` on success.
pub async fn consume_recovery_session(
    db: &Db,
    token_hash: Vec<u8>,
) -> anyhow::Result<Option<ConsumedRecoverySession>> {
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
            return Ok(None);
        };
        if consumed_at.is_some() || expires_at < now {
            return Ok(None);
        }
        tx.execute(
            "UPDATE recovery_sessions SET consumed_at = ? WHERE token_hash = ?",
            params![now, token_hash],
        )?;
        tx.commit()?;
        let account_id = uuid_from_blob(acc_blob)?;
        Ok(Some(ConsumedRecoverySession { account_id }))
    })
    .await
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
        wrapped_dek: r.get(8)?,
        wrapped_dek_nonce: r.get(9)?,
        recovery_salt: r.get(10)?,
        recovery_auth_hash: r.get(11)?,
        recovery_wrapped_dek: r.get(12)?,
        recovery_wrapped_dek_nonce: r.get(13)?,
    })
}

fn row_to_device(r: &rusqlite::Row<'_>) -> rusqlite::Result<DeviceRow> {
    Ok(DeviceRow {
        id: uuid_from_blob(r.get::<_, Vec<u8>>(0)?)?,
        account_id: uuid_from_blob(r.get::<_, Vec<u8>>(1)?)?,
        name: r.get(2)?,
        last_seen_at: r.get(3)?,
        created_at: r.get(4)?,
        last_acked_seq: r.get(5)?,
        last_seen_ip: r.get(6)?,
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
