//! Wire types for the auth HTTP surface.
//!
//! Conventions:
//! - Every byte field uses `serde_bytes` so MessagePack emits its native
//!   `bin` family (round-trips without surprises through other languages).
//! - Account / device ids are uuid v7 hex strings on the wire (16-byte
//!   binary internally, but we expose the hex form so logs / `--json`
//!   output read sensibly without an extra encoding step).
//! - Tokens are 64-char hex (32 bytes of randomness).

use serde::{Deserialize, Serialize};

/// Parameters for the client-side Argon2id master derivation. Stored
/// per-account on the server and returned by `/prelogin` so the client
/// can derive the right master with the right cost.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub struct KdfParams {
    /// Memory cost in KiB.
    pub m_kib: u32,
    /// Time cost (iterations).
    pub t: u32,
    /// Parallelism.
    pub p: u32,
}

impl KdfParams {
    /// Current default: 64 MiB / 3 iters / parallelism 1.
    pub const DEFAULT: KdfParams = KdfParams {
        m_kib: 64 * 1024,
        t: 3,
        p: 1,
    };
}

impl Default for KdfParams {
    fn default() -> Self {
        Self::DEFAULT
    }
}

/// Recovery-code material posted at signup. The wrap inside is the
/// *primary doc's* DEK (the Home DEK, the account key) under the
/// recovery KEK; it lands on the signup membership row, not on the
/// account. No other membership ever carries a recovery wrap.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecoveryMaterial {
    #[serde(with = "serde_bytes")]
    pub recovery_salt: Vec<u8>,
    #[serde(with = "serde_bytes")]
    pub recovery_auth_secret: Vec<u8>,
    #[serde(with = "serde_bytes")]
    pub recovery_wrapped_dek: Vec<u8>,
    #[serde(with = "serde_bytes")]
    pub recovery_wrapped_dek_nonce: Vec<u8>,
}

/// A doc's DEK wrapped under one key the recipient holds. Today this
/// only ever carries the *primary* doc's DEK (the Home DEK): under the
/// recovery KEK on `/recover`, under the new password KEK in
/// `new_doc_keys` uploads. `doc_id` is the uuid hex string.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DocKey {
    pub doc_id: String,
    #[serde(with = "serde_bytes")]
    pub wrapped_dek: Vec<u8>,
    #[serde(with = "serde_bytes")]
    pub wrapped_dek_nonce: Vec<u8>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum MemberRole {
    Owner,
    Member,
}

impl MemberRole {
    pub fn as_str(self) -> &'static str {
        match self {
            MemberRole::Owner => "owner",
            MemberRole::Member => "member",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "owner" => Some(MemberRole::Owner),
            "member" => Some(MemberRole::Member),
            _ => None,
        }
    }
}

/// One active doc membership of the logging-in account: the doc, the
/// account's role on it, and the doc's DEK wrapped under a key the
/// account holds. **Which key depends on the doc**: for the primary
/// doc (`doc_id == primary_doc_id`) `wrapped_dek` is the Home DEK under
/// the password KEK; for every other doc it is that doc's DEK under the
/// Home DEK (`spec/encryption.md` §Wrap-states). Clients open the
/// primary first and must handle any number of memberships.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Membership {
    pub doc_id: String,
    pub role: MemberRole,
    #[serde(with = "serde_bytes")]
    pub wrapped_dek: Vec<u8>,
    #[serde(with = "serde_bytes")]
    pub wrapped_dek_nonce: Vec<u8>,
}

// ---------- /api/account/signup ----------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SignupRequest {
    pub email: String,
    #[serde(with = "serde_bytes")]
    pub master_salt: Vec<u8>,
    pub kdf_params: KdfParams,
    #[serde(with = "serde_bytes")]
    pub auth_secret: Vec<u8>,
    /// The primary doc's DEK, generated client-side, wrapped under the
    /// password KEK. The server mints the doc id and stores this wrap on
    /// the account's `owner` membership of that doc.
    #[serde(with = "serde_bytes")]
    pub wrapped_dek: Vec<u8>,
    #[serde(with = "serde_bytes")]
    pub wrapped_dek_nonce: Vec<u8>,
    pub recovery: Option<RecoveryMaterial>,
    pub device_name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SignupResponse {
    pub account_id: String,
    pub device_id: String,
    pub device_token: String,
    /// The account's primary (Home) doc. Server-generated at signup; the
    /// client persists it so local storage can key snapshots/state on the
    /// real doc id rather than a hardcoded "the doc" placeholder.
    pub primary_doc_id: String,
}

// ---------- /api/account/prelogin ----------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PreloginRequest {
    pub email: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PreloginResponse {
    #[serde(with = "serde_bytes")]
    pub master_salt: Vec<u8>,
    pub kdf_params: KdfParams,
    /// `Some` iff the account opted into recovery code at signup.
    #[serde(default, with = "serde_bytes_opt")]
    pub recovery_salt: Option<Vec<u8>>,
}

// ---------- /api/account/login ----------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoginRequest {
    pub email: String,
    #[serde(with = "serde_bytes")]
    pub auth_secret: Vec<u8>,
    /// If set, atomically register this client as a device and return a
    /// `device_token`. This is the device-2 bootstrap path; subsequent
    /// device additions can also use `POST /api/devices` with bearer auth.
    pub register_device: Option<DeviceRegistration>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoginResponse {
    pub account_id: String,
    /// The account's primary (Home) doc. New devices use it to key
    /// local storage; existing devices already hold the same value.
    /// Always appears in `memberships`.
    pub primary_doc_id: String,
    /// Every active membership with its password-KEK wrap. The client
    /// unwraps each with the KEK it just derived.
    pub memberships: Vec<Membership>,
    pub recovery_present: bool,
    /// Present iff `register_device` was set on the request.
    pub device: Option<DeviceCredential>,
}

// ---------- /api/account/recover ----------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecoverRequest {
    pub email: String,
    #[serde(with = "serde_bytes")]
    pub recovery_auth_secret: Vec<u8>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecoverResponse {
    pub account_id: String,
    pub primary_doc_id: String,
    /// Exactly one entry: the primary membership's recovery-KEK wrap
    /// of the Home DEK. Released only after the recovery auth proof.
    /// The client unwraps it, re-wraps it under its new password KEK
    /// for `/password/reset`, and opens every other membership's wrap
    /// (from `GET /api/docs` after the reset) with the Home DEK.
    pub doc_keys: Vec<DocKey>,
    /// Single-use, 15-min TTL. Required for `/password/reset`.
    pub recovery_session_token: String,
}

// ---------- /api/account/password/change ----------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PasswordChangeRequest {
    /// Re-verify the user's *current* password before accepting the change.
    /// Defends against a hijacked logged-in session changing the password.
    #[serde(with = "serde_bytes")]
    pub current_auth_secret: Vec<u8>,
    #[serde(with = "serde_bytes")]
    pub new_master_salt: Vec<u8>,
    pub new_kdf_params: KdfParams,
    #[serde(with = "serde_bytes")]
    pub new_auth_secret: Vec<u8>,
    /// The Home DEK re-wrapped under the new KEK, as a one-element
    /// set naming the primary doc. The server rejects the change (400)
    /// unless the set of doc ids is exactly `{ primary_doc_id }`. Other
    /// memberships are wrapped under the Home DEK and are not touched.
    pub new_doc_keys: Vec<DocKey>,
}

// ---------- /api/account/password/reset ----------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PasswordResetRequest {
    pub recovery_session_token: String,
    #[serde(with = "serde_bytes")]
    pub new_master_salt: Vec<u8>,
    pub new_kdf_params: KdfParams,
    #[serde(with = "serde_bytes")]
    pub new_auth_secret: Vec<u8>,
    /// The recovered Home DEK re-wrapped under the new KEK, as a
    /// one-element set naming the primary doc (400 on mismatch; the
    /// session is not consumed). Other memberships are wrapped under
    /// the Home DEK and are not touched. See `encryption.md` §Recovery.
    pub new_doc_keys: Vec<DocKey>,
    pub device_name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PasswordResetResponse {
    pub device_id: String,
    pub device_token: String,
    /// The account's primary (Home) doc. Returned so the new device can
    /// key local storage without an extra round trip.
    pub primary_doc_id: String,
}

// ---------- /api/docs ----------

/// `POST /api/docs`: create a doc the calling account owns. The client
/// generates the doc's DEK and wraps it under its **Home DEK** (never
/// the password or recovery KEK: those wrap the Home DEK only). The
/// server mints the doc id and inserts the `owner` membership carrying
/// this wrap; password change, reset and recovery leave it alone.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateDocRequest {
    #[serde(with = "serde_bytes")]
    pub wrapped_dek: Vec<u8>,
    #[serde(with = "serde_bytes")]
    pub wrapped_dek_nonce: Vec<u8>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateDocResponse {
    pub doc_id: String,
}

/// `GET /api/docs`: the calling account's active memberships, in the
/// same shape `/login` returns them, for a device that wants to learn
/// about docs created since it logged in.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DocsListResponse {
    pub primary_doc_id: String,
    pub memberships: Vec<Membership>,
}

// ---------- /api/devices ----------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeviceRegistration {
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeviceRenameRequest {
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeviceCredential {
    pub device_id: String,
    pub device_token: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Device {
    pub id: String,
    pub name: String,
    pub last_seen_at: i64,
    /// IP the device last connected from, as the server saw it. `None`
    /// until the device's first authed request.
    pub last_seen_ip: Option<String>,
    pub created_at: i64,
    /// Highest op seq this device has acked (0 = never acked).
    pub last_acked_seq: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DevicesListResponse {
    pub devices: Vec<Device>,
    /// Head of the account's op log, so clients can render how far
    /// behind each device is.
    pub server_last_seq: u64,
}

// ---------- error envelope ----------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ErrorBody {
    pub code: String,
    pub message: String,
}

mod serde_bytes_opt {
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    pub fn serialize<S: Serializer>(v: &Option<Vec<u8>>, s: S) -> Result<S::Ok, S::Error> {
        match v {
            Some(b) => serde_bytes::Bytes::new(b).serialize(s),
            None => s.serialize_none(),
        }
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Option<Vec<u8>>, D::Error> {
        let v: Option<serde_bytes::ByteBuf> = Option::deserialize(d)?;
        Ok(v.map(|b| b.into_vec()))
    }
}
