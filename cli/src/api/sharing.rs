//! Sharing a workspace (`spec/sharing-plan.md` §Invites, §Member
//! removal): its members, single-use invite links, and leaving. Unlike
//! the rest of `api` these are not doc reads: each is an HTTP round
//! trip on the profile's device token, run anywhere a [`Profile`] is at
//! hand. `commands::share` renders them for the terminal; the Apple
//! apps call them through `monoplan-ffi`'s `Engine`, off the loop, so a
//! slow server never holds up a doc call.
//!
//! The invite secret is generated here and leaves only inside the link
//! returned by [`create_invite`]: the server gets its HKDF id and the
//! doc's DEK wrapped under the second subkey, so it cannot rebuild the
//! link, and nothing here keeps it either.

use anyhow::{Context, bail};
use monoplan_core::{DocId, InviteSecret};
use monoplan_protocol::{
    CreateInviteRequest, CreateInviteResponse, InvitesListResponse, MemberRole, MembersListResponse,
};
use serde::Serialize;

use crate::config::Profile;
use crate::keystore::dek_for_doc;
use crate::net::{Client, NetError};

/// One active member of a workspace (`GET /api/docs/:id/members`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct WorkspaceMember {
    pub account_id: String,
    pub email: String,
    pub owner: bool,
    /// Unix millis.
    pub added_at: i64,
}

/// An issued invite link nobody has used yet and that has not expired
/// (`GET /api/docs/:id/invites`, owner only). The server holds no
/// secret, so it cannot be shown as a link again, only revoked.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct PendingInvite {
    pub invite_id: String,
    /// Unix millis.
    pub created_at: i64,
    /// Unix millis.
    pub expires_at: i64,
}

/// A link just issued: the only copy of it there will ever be.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct IssuedInvite {
    /// `<server>/#invite_<secret>`.
    pub link: String,
    pub invite_id: String,
    /// Unix millis.
    pub expires_at: i64,
}

/// The server, the token and the profile, read once per call.
struct Conn {
    client: Client,
    token: String,
    server_url: String,
}

fn conn(profile: &Profile) -> anyhow::Result<Conn> {
    // The token before `config.toml`, which a local-only workspace
    // does not have: its error says why.
    let token = profile.read_secrets()?.device_token()?.to_string();
    let server_url = profile.read_config()?.server_url;
    Ok(Conn {
        client: Client::new(server_url.clone()),
        token,
        server_url,
    })
}

/// A server refusal as its own message ("only the owner can …"), a
/// network failure as such: what a person reads in the app or terminal.
fn said(e: NetError) -> anyhow::Error {
    match e {
        NetError::Api { message, .. } if !message.is_empty() => anyhow::anyhow!(message),
        NetError::Transport(_) => anyhow::anyhow!("could not reach the server"),
        other => other.into(),
    }
}

/// The workspace's active members, as the server lists them. Any
/// member may ask.
pub async fn members(profile: &Profile, doc: DocId) -> anyhow::Result<Vec<WorkspaceMember>> {
    let c = conn(profile)?;
    let resp: MembersListResponse = c
        .client
        .get_authed(&format!("/api/docs/{}/members", doc.0), &c.token)
        .await
        .map_err(said)?;
    Ok(resp
        .members
        .into_iter()
        .map(|m| WorkspaceMember {
            account_id: m.account_id,
            email: m.email,
            owner: m.role == MemberRole::Owner,
            added_at: m.added_at,
        })
        .collect())
}

/// The workspace's unused, unexpired links, oldest first. Owner only.
pub async fn pending_invites(profile: &Profile, doc: DocId) -> anyhow::Result<Vec<PendingInvite>> {
    let c = conn(profile)?;
    let resp: InvitesListResponse = c
        .client
        .get_authed(&format!("/api/docs/{}/invites", doc.0), &c.token)
        .await
        .map_err(said)?;
    let now = now_millis();
    Ok(resp
        .invites
        .into_iter()
        .filter(|i| i.consumed_at.is_none() && i.expires_at > now)
        .map(|i| PendingInvite {
            invite_id: i.invite_id,
            created_at: i.created_at,
            expires_at: i.expires_at,
        })
        .collect())
}

/// Issue a single-use link to the workspace (owner only; never Home,
/// whose key is the account key). `expires_days` defaults to the
/// server's 7 and is clamped by it to 30.
pub async fn create_invite(
    profile: &Profile,
    doc: DocId,
    expires_days: Option<u64>,
) -> anyhow::Result<IssuedInvite> {
    let c = conn(profile)?;
    let dek = dek_for_doc(&profile.read_secrets()?, doc)?;
    let secret = InviteSecret::generate();
    let w = secret.wrap(&dek)?;
    let resp: CreateInviteResponse = c
        .client
        .post_authed(
            &format!("/api/docs/{}/invites", doc.0),
            &c.token,
            &CreateInviteRequest {
                invite_id: hex::encode(secret.id()?),
                wrapped_dek: w.ciphertext,
                wrapped_dek_nonce: w.nonce.to_vec(),
                expires_in_secs: expires_days.map(|d| d * 24 * 60 * 60),
            },
        )
        .await
        .map_err(said)
        .context("issuing the invite")?;
    Ok(IssuedInvite {
        link: format!(
            "{}/#invite_{}",
            c.server_url.trim_end_matches('/'),
            hex::encode(secret.as_bytes())
        ),
        invite_id: resp.invite_id,
        expires_at: resp.expires_at,
    })
}

/// Revoke an unused link.
pub async fn revoke_invite(profile: &Profile, doc: DocId, invite_id: &str) -> anyhow::Result<()> {
    let c = conn(profile)?;
    c.client
        .delete_authed(
            &format!("/api/docs/{}/invites/{}", doc.0, invite_id.trim()),
            &c.token,
        )
        .await
        .map_err(said)
        .context("revoking the invite")
}

/// Remove a member (owner only; not the last owner). Forward-only: they
/// keep what they already synced. With the caller's own account id it
/// is leaving, which [`leave`] says more plainly.
pub async fn remove_member(profile: &Profile, doc: DocId, account_id: &str) -> anyhow::Result<()> {
    let c = conn(profile)?;
    c.client
        .delete_authed(
            &format!("/api/docs/{}/members/{}", doc.0, account_id),
            &c.token,
        )
        .await
        .map_err(said)
        .context("removing the member")
}

/// Leave a workspace the account does not own. The server then sends
/// `DocClosed { reason: "left" }` to every live session of the account,
/// this device's included, and the engine forgets the doc on it; a
/// device offline meanwhile is refused the doc on its next `OpenDoc`
/// and forgets it then.
pub async fn leave(profile: &Profile, doc: DocId, own_account_id: &str) -> anyhow::Result<()> {
    if own_account_id.is_empty() {
        bail!("no account to leave with");
    }
    let c = conn(profile)?;
    c.client
        .delete_authed(
            &format!("/api/docs/{}/members/{}", doc.0, own_account_id),
            &c.token,
        )
        .await
        .map_err(said)
        .context("leaving the workspace")
}

fn now_millis() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}
