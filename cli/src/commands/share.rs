//! Sharing links (`spec/sharing-plan.md` §Invites): issue, list, revoke
//! and accept, plus the member list.
//!
//! A link is `<server>/#invite_<secret>`. The secret never leaves this
//! machine except inside the printed link: the server gets an
//! HKDF-derived id and the doc's DEK wrapped under a second subkey.
//! Accepting stores the recovered DEK in `secrets.toml` under the doc
//! id, exactly where login would have put it; the CLI's sync engine
//! still runs the Home doc only (`spec/sharing-plan.md` Phase 4), so a
//! joined doc shows up in the web client first.

use anyhow::{Context, bail};
use clap::{Parser, Subcommand};
use monoplan_core::{AEAD_NONCE_LEN, InviteSecret, WrappedDek};
use monoplan_protocol::{
    AcceptInviteRequest, AcceptInviteResponse, CreateInviteRequest, CreateInviteResponse,
    InvitePayloadResponse, InvitesListResponse, MembersListResponse,
};

use crate::config::{DocSecrets, Profile};
use crate::keystore::{dek_for_doc, parse_doc_id};
use crate::net::Client;

#[derive(Parser, Debug)]
#[command(subcommand_required = true, arg_required_else_help = true)]
pub struct ShareArgs {
    #[command(subcommand)]
    pub cmd: ShareCmd,
}

#[derive(Subcommand, Debug)]
pub enum ShareCmd {
    /// Create a single-use invite link for a workspace you own. The
    /// link is printed once and cannot be recovered afterwards.
    Link {
        /// The workspace (doc) id. Never the Personal workspace.
        doc: String,
        /// Lifetime in days (server default 7, at most 30).
        #[arg(long)]
        expires_days: Option<u64>,
    },
    /// Show unused invite links on a workspace you own.
    Pending { doc: String },
    /// Revoke an unused invite link.
    Revoke { doc: String, invite_id: String },
    /// Join a workspace from an invite link (or its bare secret).
    Accept { link: String },
    /// Show the members of a workspace you belong to.
    Members { doc: String },
    /// Remove a member from a workspace you own, by email. Not
    /// yourself, and not the last owner. They keep what they already
    /// synced; the server stops serving them the workspace from now.
    Remove { doc: String, email: String },
}

pub async fn run(args: ShareArgs) -> anyhow::Result<()> {
    match args.cmd {
        ShareCmd::Link { doc, expires_days } => link(&doc, expires_days).await,
        ShareCmd::Pending { doc } => pending(&doc).await,
        ShareCmd::Revoke { doc, invite_id } => revoke(&doc, &invite_id).await,
        ShareCmd::Accept { link } => accept(&link).await,
        ShareCmd::Members { doc } => members(&doc).await,
        ShareCmd::Remove { doc, email } => remove(&doc, &email).await,
    }
}

struct Ctx {
    profile: Profile,
    client: Client,
    token: String,
    server_url: String,
}

fn ctx() -> anyhow::Result<Ctx> {
    let profile = Profile::require_active()?;
    let secrets = profile.read_secrets()?;
    // Before `config.toml`, which a local-only workspace does not have.
    let token = secrets.device_token()?.to_string();
    let config = profile.read_config()?;
    Ok(Ctx {
        client: Client::new(config.server_url.clone()),
        token,
        server_url: config.server_url,
        profile,
    })
}

async fn link(doc: &str, expires_days: Option<u64>) -> anyhow::Result<()> {
    let c = ctx()?;
    let doc_id = parse_doc_id(doc)?;
    let storage = crate::storage::open_storage(&c.profile)?;
    let account = storage.read_account()?;
    if doc_id == account.primary_doc_id {
        bail!("the Personal workspace cannot be shared: its key is the account key");
    }
    let secrets = c.profile.read_secrets()?;
    let dek = dek_for_doc(&secrets, doc_id)?;

    let secret = InviteSecret::generate();
    let w = secret.wrap(&dek)?;
    let resp: CreateInviteResponse = c
        .client
        .post_authed(
            &format!("/api/docs/{}/invites", doc_id.0),
            &c.token,
            &CreateInviteRequest {
                invite_id: hex::encode(secret.id()?),
                wrapped_dek: w.ciphertext,
                wrapped_dek_nonce: w.nonce.to_vec(),
                expires_in_secs: expires_days.map(|d| d * 24 * 60 * 60),
            },
        )
        .await
        .context("issuing the invite")?;

    let secret_hex = hex::encode(secret.as_bytes());
    println!(
        "{}/#invite_{}",
        c.server_url.trim_end_matches('/'),
        secret_hex
    );
    eprintln!();
    eprintln!("Single use. Expires {}.", format_ts(resp.expires_at));
    eprintln!(
        "Anyone with this link can join and read the whole workspace. It is shown once; \
         revoke it with `monoplan workspace share revoke {} {}`.",
        doc_id.0, resp.invite_id
    );
    Ok(())
}

async fn pending(doc: &str) -> anyhow::Result<()> {
    let c = ctx()?;
    let doc_id = parse_doc_id(doc)?;
    let resp: InvitesListResponse = c
        .client
        .get_authed(&format!("/api/docs/{}/invites", doc_id.0), &c.token)
        .await?;
    let now = now_millis();
    let open: Vec<_> = resp
        .invites
        .iter()
        .filter(|i| i.consumed_at.is_none() && i.expires_at > now)
        .collect();
    if open.is_empty() {
        println!("No unused invite links.");
        return Ok(());
    }
    for i in open {
        println!(
            "{}  created {}  expires {}",
            i.invite_id,
            format_ts(i.created_at),
            format_ts(i.expires_at)
        );
    }
    Ok(())
}

async fn revoke(doc: &str, invite_id: &str) -> anyhow::Result<()> {
    let c = ctx()?;
    let doc_id = parse_doc_id(doc)?;
    c.client
        .delete_authed(
            &format!("/api/docs/{}/invites/{}", doc_id.0, invite_id.trim()),
            &c.token,
        )
        .await
        .context("revoking the invite")?;
    println!("Revoked.");
    Ok(())
}

async fn accept(link: &str) -> anyhow::Result<()> {
    let c = ctx()?;
    let secret_hex = parse_link(link).context("not an invite link or secret")?;
    let secret = InviteSecret::from_bytes(&hex::decode(&secret_hex)?)?;
    let id = hex::encode(secret.id()?);

    let payload: InvitePayloadResponse = c
        .client
        .get_unauthed(&format!("/api/invites/{id}"))
        .await
        .context("looking the invite up")?;
    let dek = secret.unwrap(&wrapped(&payload.wrapped_dek, &payload.wrapped_dek_nonce)?)?;

    let storage = crate::storage::open_storage(&c.profile)?;
    let account = storage.read_account()?;
    let mut secrets = c.profile.read_secrets()?;
    let home = dek_for_doc(&secrets, account.primary_doc_id)?;
    let w = home.wrap_dek(&dek)?;
    let joined: AcceptInviteResponse = c
        .client
        .post_authed(
            &format!("/api/invites/{id}/accept"),
            &c.token,
            &AcceptInviteRequest {
                wrapped_dek: w.ciphertext,
                wrapped_dek_nonce: w.nonce.to_vec(),
            },
        )
        .await
        .context("accepting the invite")?;
    if joined.doc_id != payload.doc_id {
        bail!("server joined a different doc than the invite named");
    }

    secrets.docs.insert(
        joined.doc_id.clone(),
        DocSecrets {
            dek_hex: hex::encode(dek.as_bytes()),
        },
    );
    c.profile.write_secrets(&secrets)?;
    println!(
        "Joined workspace {} (invited by {}).",
        joined.doc_id, payload.inviter_hint
    );
    eprintln!(
        "The CLI syncs the Personal workspace only for now; the shared workspace is available in the web client."
    );
    Ok(())
}

async fn members(doc: &str) -> anyhow::Result<()> {
    let c = ctx()?;
    let doc_id = parse_doc_id(doc)?;
    let resp: MembersListResponse = c
        .client
        .get_authed(&format!("/api/docs/{}/members", doc_id.0), &c.token)
        .await?;
    for m in resp.members {
        println!(
            "{}  {}  joined {}",
            m.email,
            m.role.as_str(),
            format_ts(m.added_at)
        );
    }
    Ok(())
}

async fn remove(doc: &str, email: &str) -> anyhow::Result<()> {
    let c = ctx()?;
    let doc_id = parse_doc_id(doc)?;
    let wanted = email.trim();
    let list: MembersListResponse = c
        .client
        .get_authed(&format!("/api/docs/{}/members", doc_id.0), &c.token)
        .await
        .context("listing the members")?;
    let target = list
        .members
        .iter()
        .find(|m| m.email.eq_ignore_ascii_case(wanted))
        .with_context(|| format!("{wanted} is not a member of this workspace"))?;
    c.client
        .delete_authed(
            &format!("/api/docs/{}/members/{}", doc_id.0, target.account_id),
            &c.token,
        )
        .await
        .context("removing the member")?;
    println!("Removed {}.", target.email);
    eprintln!(
        "They keep whatever they already synced; the server serves them nothing further from this workspace."
    );
    Ok(())
}

/// The 64-hex secret out of `…#invite_<secret>`, or a bare secret.
fn parse_link(input: &str) -> Option<String> {
    let s = input.trim();
    let candidate = match s.rsplit_once("#invite_") {
        Some((_, rest)) => rest,
        None => s,
    };
    let candidate = candidate.trim_end_matches(['/', '#']);
    (candidate.len() == 64 && candidate.bytes().all(|b| b.is_ascii_hexdigit()))
        .then(|| candidate.to_ascii_lowercase())
}

fn wrapped(ciphertext: &[u8], nonce: &[u8]) -> anyhow::Result<WrappedDek> {
    let nonce: [u8; AEAD_NONCE_LEN] = nonce
        .try_into()
        .map_err(|_| anyhow::anyhow!("server returned a wrap nonce of wrong length"))?;
    Ok(WrappedDek {
        ciphertext: ciphertext.to_vec(),
        nonce,
    })
}

fn now_millis() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// Unix millis as a local-time-free ISO-ish stamp (UTC); enough for a
/// terminal line without pulling in a tz crate.
fn format_ts(ms: i64) -> String {
    let secs = ms.div_euclid(1000);
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400);
    let (y, m, d) = civil_from_days(days);
    format!(
        "{y:04}-{m:02}-{d:02} {:02}:{:02} UTC",
        rem / 3600,
        (rem % 3600) / 60
    )
}

/// Howard Hinnant's days-to-civil; proleptic Gregorian.
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_links_and_bare_secrets() {
        let s = "a".repeat(64);
        assert_eq!(
            parse_link(&format!("https://x.example/#invite_{s}")),
            Some(s.clone())
        );
        assert_eq!(parse_link(&format!("  {s}\n")), Some(s.clone()));
        assert_eq!(parse_link(&s.to_uppercase()), Some(s.clone()));
        assert_eq!(parse_link("https://x.example/#inbox"), None);
        assert_eq!(parse_link(&s[1..]), None);
    }

    #[test]
    fn formats_epoch_days() {
        assert_eq!(format_ts(0), "1970-01-01 00:00 UTC");
        // 2026-10-21T00:00:00Z
        assert_eq!(format_ts(1_792_540_800_000), "2026-10-21 00:00 UTC");
    }
}
