//! Wire types for the auth HTTP surface.
//!
//! Conventions:
//! - Bodies are JSON (`spec/auth.md` §Wire encoding). Every byte field
//!   uses `serde_b64` and travels as a standard base64 string: these are
//!   salts, nonces and wrapped keys, a few dozen bytes each, so a readable
//!   Network tab beats a byte-native encoding here. Encrypted update
//!   blobs never cross HTTP; they ride the WebSocket as MessagePack.
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
    #[serde(with = "serde_b64")]
    pub recovery_salt: Vec<u8>,
    #[serde(with = "serde_b64")]
    pub recovery_auth_secret: Vec<u8>,
    #[serde(with = "serde_b64")]
    pub recovery_wrapped_dek: Vec<u8>,
    #[serde(with = "serde_b64")]
    pub recovery_wrapped_dek_nonce: Vec<u8>,
}

/// A doc's DEK wrapped under one key the recipient holds. Today this
/// only ever carries the *primary* doc's DEK (the Home DEK): under the
/// recovery KEK on `/recover`, under the new password KEK in
/// `new_doc_keys` uploads. `doc_id` is the uuid hex string.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DocKey {
    pub doc_id: String,
    #[serde(with = "serde_b64")]
    pub wrapped_dek: Vec<u8>,
    #[serde(with = "serde_b64")]
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
    #[serde(with = "serde_b64")]
    pub wrapped_dek: Vec<u8>,
    #[serde(with = "serde_b64")]
    pub wrapped_dek_nonce: Vec<u8>,
}

// ---------- /api/account/signup ----------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SignupRequest {
    pub email: String,
    #[serde(with = "serde_b64")]
    pub master_salt: Vec<u8>,
    pub kdf_params: KdfParams,
    #[serde(with = "serde_b64")]
    pub auth_secret: Vec<u8>,
    /// The primary doc's DEK, generated client-side, wrapped under the
    /// password KEK. The server stores this wrap on the account's `owner`
    /// membership of the primary doc.
    #[serde(with = "serde_b64")]
    pub wrapped_dek: Vec<u8>,
    #[serde(with = "serde_b64")]
    pub wrapped_dek_nonce: Vec<u8>,
    /// Client-chosen id for the primary doc, or `None` to let the server
    /// mint one. The web client signs up from an anonymous local-only
    /// session whose doc already has history under a locally minted id
    /// and a DEK it now wraps above; keeping the id means that history
    /// becomes the account's Home doc and ships on first sync instead of
    /// being discarded. The server only requires the id to be a UUID that
    /// no doc already uses (409 otherwise); membership, not the id,
    /// grants access.
    #[serde(default)]
    pub primary_doc_id: Option<String>,
    /// Mandatory since 2026-10-26: every account enrols a recovery code
    /// at signup. Opting out afterwards is future work.
    pub recovery: RecoveryMaterial,
    pub device_name: String,
    /// See `DeviceRegistration::kind`.
    pub device_kind: String,
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
    #[serde(with = "serde_b64")]
    pub master_salt: Vec<u8>,
    pub kdf_params: KdfParams,
    /// `Some` iff the account opted into recovery code at signup.
    #[serde(default, with = "serde_b64_opt")]
    pub recovery_salt: Option<Vec<u8>>,
}

// ---------- /api/account/login ----------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoginRequest {
    pub email: String,
    #[serde(with = "serde_b64")]
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
    #[serde(with = "serde_b64")]
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
    #[serde(with = "serde_b64")]
    pub current_auth_secret: Vec<u8>,
    #[serde(with = "serde_b64")]
    pub new_master_salt: Vec<u8>,
    pub new_kdf_params: KdfParams,
    #[serde(with = "serde_b64")]
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
    #[serde(with = "serde_b64")]
    pub new_master_salt: Vec<u8>,
    pub new_kdf_params: KdfParams,
    #[serde(with = "serde_b64")]
    pub new_auth_secret: Vec<u8>,
    /// The recovered Home DEK re-wrapped under the new KEK, as a
    /// one-element set naming the primary doc (400 on mismatch; the
    /// session is not consumed). Other memberships are wrapped under
    /// the Home DEK and are not touched. See `encryption.md` §Recovery.
    pub new_doc_keys: Vec<DocKey>,
    pub device_name: String,
    /// See `DeviceRegistration::kind`.
    pub device_kind: String,
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
    #[serde(with = "serde_b64")]
    pub wrapped_dek: Vec<u8>,
    #[serde(with = "serde_b64")]
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

// ---------- invites (sharing links) ----------
//
// A sharing link is a 32-byte secret in a URL fragment. The client
// derives two HKDF subkeys from it (`monoplan-core` `InviteSecret`): the
// **invite id**, sent to the server on every invite call, and the
// **invite KEK**, under which the inviter wraps the doc's DEK. The server
// stores the id and the wrap and can open neither. Ids are 64-char hex on
// the wire like every other token.

/// `POST /api/docs/:doc_id/invites` (owner only; never the primary doc).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateInviteRequest {
    /// HKDF-derived lookup id, 64-char hex. The server never sees the
    /// secret it came from.
    pub invite_id: String,
    /// The doc's DEK under the invite KEK.
    #[serde(with = "serde_b64")]
    pub wrapped_dek: Vec<u8>,
    #[serde(with = "serde_b64")]
    pub wrapped_dek_nonce: Vec<u8>,
    /// Lifetime in seconds. Server default 7 days, clamped to
    /// [60 s, 30 days].
    pub expires_in_secs: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateInviteResponse {
    pub invite_id: String,
    /// Unix millis.
    pub expires_at: i64,
}

/// One row of `GET /api/docs/:doc_id/invites` (owner only). The server
/// holds no secret, so a listed invite cannot be turned back into a
/// link; it can only be revoked.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Invite {
    pub invite_id: String,
    pub created_at: i64,
    pub expires_at: i64,
    /// Set once accepted (single use).
    pub consumed_at: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InvitesListResponse {
    pub invites: Vec<Invite>,
}

/// `GET /api/invites/:invite_id` (unauthenticated: holding the id is the
/// credential, and the wrap is useless without the fragment secret).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InvitePayloadResponse {
    pub doc_id: String,
    #[serde(with = "serde_b64")]
    pub wrapped_dek: Vec<u8>,
    #[serde(with = "serde_b64")]
    pub wrapped_dek_nonce: Vec<u8>,
    pub expires_at: i64,
    /// The inviter's email, masked (`d…@example.com`), so the invitee
    /// can see who the link came from before signing in.
    pub inviter_hint: String,
}

/// `POST /api/invites/:invite_id/accept` (authed). The body is the doc's
/// DEK re-wrapped under the acceptor's **Home DEK**, exactly as
/// `CreateDocRequest` carries it: it becomes the new membership's wrap.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AcceptInviteRequest {
    #[serde(with = "serde_b64")]
    pub wrapped_dek: Vec<u8>,
    #[serde(with = "serde_b64")]
    pub wrapped_dek_nonce: Vec<u8>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AcceptInviteResponse {
    pub doc_id: String,
}

/// One row of `GET /api/docs/:doc_id/members` (any active member).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Member {
    pub account_id: String,
    pub email: String,
    pub role: MemberRole,
    pub added_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MembersListResponse {
    pub members: Vec<Member>,
}

// ---------- /api/devices ----------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeviceRegistration {
    pub name: String,
    /// What sort of client this is, so device lists can show an icon.
    /// `web` for the browser client (installed PWA included); a native
    /// app reports its host platform: `macos`, `linux`, `windows`,
    /// `ios`, `android`. Self-reported and stored verbatim, like `name`;
    /// the server only requires it to be non-empty.
    pub kind: String,
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
    /// See `DeviceRegistration::kind`.
    pub kind: String,
    pub last_seen_at: i64,
    /// IP the device last connected from, as the server saw it. `None`
    /// until the device's first authed request.
    pub last_seen_ip: Option<String>,
    pub created_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DevicesListResponse {
    pub devices: Vec<Device>,
}

// ---------- error envelope ----------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ErrorBody {
    pub code: String,
    pub message: String,
}

/// Byte fields as standard (padded) base64 strings.
pub mod serde_b64 {
    use base64::Engine;
    use base64::engine::general_purpose::STANDARD;
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(v: &[u8], s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&STANDARD.encode(v))
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<u8>, D::Error> {
        let s = <std::borrow::Cow<'de, str>>::deserialize(d)?;
        STANDARD
            .decode(s.as_bytes())
            .map_err(serde::de::Error::custom)
    }
}

/// `Option<Vec<u8>>` as base64 or `null`.
pub mod serde_b64_opt {
    use base64::Engine;
    use base64::engine::general_purpose::STANDARD;
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(v: &Option<Vec<u8>>, s: S) -> Result<S::Ok, S::Error> {
        match v {
            Some(b) => s.serialize_some(&STANDARD.encode(b)),
            None => s.serialize_none(),
        }
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Option<Vec<u8>>, D::Error> {
        let v: Option<std::borrow::Cow<'de, str>> = Option::deserialize(d)?;
        v.map(|s| {
            STANDARD
                .decode(s.as_bytes())
                .map_err(serde::de::Error::custom)
        })
        .transpose()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn byte_fields_round_trip_as_base64_json() {
        let req = CreateDocRequest {
            wrapped_dek: vec![0, 1, 2, 255],
            wrapped_dek_nonce: vec![],
        };
        let json = serde_json::to_string(&req).unwrap();
        assert_eq!(json, r#"{"wrapped_dek":"AAEC/w==","wrapped_dek_nonce":""}"#);
        let back: CreateDocRequest = serde_json::from_str(&json).unwrap();
        assert_eq!(back.wrapped_dek, req.wrapped_dek);
        assert!(back.wrapped_dek_nonce.is_empty());
    }

    #[test]
    fn optional_bytes_are_null_or_base64() {
        let none = PreloginResponse {
            master_salt: vec![7; 3],
            kdf_params: KdfParams::DEFAULT,
            recovery_salt: None,
        };
        let json = serde_json::to_string(&none).unwrap();
        assert!(json.contains(r#""recovery_salt":null"#));
        let back: PreloginResponse = serde_json::from_str(&json).unwrap();
        assert!(back.recovery_salt.is_none());
        // Field absent entirely → `default` kicks in.
        let back: PreloginResponse =
            serde_json::from_str(r#"{"master_salt":"BwcH","kdf_params":{"m_kib":8,"t":1,"p":1}}"#)
                .unwrap();
        assert_eq!(back.master_salt, vec![7; 3]);
        assert!(back.recovery_salt.is_none());
        let some = PreloginResponse {
            recovery_salt: Some(vec![1, 2]),
            ..none
        };
        let back: PreloginResponse =
            serde_json::from_str(&serde_json::to_string(&some).unwrap()).unwrap();
        assert_eq!(back.recovery_salt, Some(vec![1, 2]));
    }

    #[test]
    fn malformed_base64_is_rejected() {
        let err = serde_json::from_str::<CreateDocRequest>(
            r#"{"wrapped_dek":"not base64!","wrapped_dek_nonce":""}"#,
        )
        .unwrap_err();
        assert!(err.to_string().contains("Invalid"), "{err}");
    }
}
