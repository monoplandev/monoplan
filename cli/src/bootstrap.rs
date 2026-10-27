//! The local-only workspace: what a fresh install runs before (or
//! without) an account.
//!
//! The first doc command on an install with no profile mints a doc id
//! and a DEK locally, seeds the doc with the same onboarding content a
//! signup would, and writes a profile with no device token. From then
//! on every command, the daemon included, works against that doc
//! exactly as it would for an account's device, minus the server:
//! the host never connects, and replies say nothing about being
//! offline because there is nothing to be offline from. `monoplan auth
//! signup` later adopts the doc as the account's Home doc (same id,
//! same key; `spec/auth.md` §Device pairing), so nothing captured
//! before signing up is lost or re-encrypted. This mirrors the web
//! client's anonymous session.

use std::path::PathBuf;

use monoplan_core::{Dek, Doc, DocId, NotesDeltaOp};

use crate::config::{Profile, Secrets};
use crate::storage::Account;

/// The onboarding doc every new workspace starts from, whether minted
/// locally or by a fresh signup: a "Welcome" list with a few
/// learn-by-doing items, the first carrying a descriptive line in its
/// notes. The same content the web client seeds (see its runtime.ts),
/// so a workspace looks the same whichever client created it. Real
/// committed ops, so `peer_id` must be the leased one
/// (`spec/peer-id-plan.md`).
pub fn welcome_doc(peer_id: u64) -> anyhow::Result<Doc> {
    let doc = Doc::new_with_peer(peer_id)?;
    let welcome = doc.add_list("Welcome")?;
    doc.set_list_icon(&welcome, "👋")?;
    // The list is empty, so index 0 keeps the authored order.
    let welcome_items = [
        "Welcome to Monoplan",
        "Press 'space' to create a new item",
        "Tick the box (or press 'x') to complete an item",
        "Press 's' to star an item",
        "Press '?' to see all keyboard shortcuts",
    ];
    let item_ids = doc.add_items_at(&welcome, &welcome_items, 0)?;
    if let Some(first) = item_ids.first() {
        doc.apply_notes_delta(
            first,
            &[NotesDeltaOp::Insert {
                insert: "Monoplan helps you capture and organise your ideas, tasks, and projects."
                    .to_string(),
            }],
        )?;
    }
    Ok(doc)
}

/// The profile at the default root, created as a local-only workspace
/// if there is none. What every doc command opens.
pub async fn open_or_init() -> anyhow::Result<Profile> {
    open_or_init_at(Profile::root()?).await
}

/// `open_or_init` for an explicit directory (tests; an embedded host
/// handed its path).
pub async fn open_or_init_at(dir: PathBuf) -> anyhow::Result<Profile> {
    if let Some(profile) = Profile::active_at(dir.clone()) {
        return Ok(profile);
    }
    let profile = Profile::create_at(dir)?;
    // Hold the profile while it is written, so a second first command
    // racing this one waits here and finds it, and no auto-spawned
    // daemon boots a half-written one. Nothing can be hosting it yet
    // (no `secrets.toml`), so this never has to stop anything.
    let claim = crate::client::claim(&profile.dir).await?;
    if profile.exists() {
        return Ok(profile);
    }
    init_local(&profile)?;
    drop(claim);
    eprintln!(
        "created a local workspace in {}; `monoplan auth signup` syncs it across devices",
        profile.dir.display()
    );
    Ok(profile)
}

/// Write a fresh local-only workspace into `profile`: seed snapshot,
/// identity-less account row, secrets with the minted DEK and no
/// device token. The caller holds the profile (`client::claim`).
pub fn init_local(profile: &Profile) -> anyhow::Result<()> {
    let storage = crate::storage::open_storage(profile)?;
    let (doc_id, dek) = seed_local_doc(profile, &storage)?;
    storage.write_account(&Account {
        primary_doc_id: doc_id,
        identity: None,
    })?;
    // Written last: it is the "profile exists" marker.
    profile.write_secrets(&Secrets::new(None, [(doc_id, &dek)]))?;
    Ok(())
}

/// Mint a doc id and DEK and write the onboarding doc as the baseline
/// snapshot under them. The id is a locally generated UUID; the server
/// adopts it at signup (`primary_doc_id`), since membership, not the
/// id, grants access.
pub fn seed_local_doc(
    profile: &Profile,
    storage: &crate::storage::SqliteStorage,
) -> anyhow::Result<(DocId, Dek)> {
    let doc_id = DocId(uuid::Uuid::new_v4());
    let dek = Dek::generate();
    // `peer` stays in scope (lease held) until the snapshot is durably
    // written: the seed is real committed ops under that peer id.
    let peer = crate::peer::claim(&profile.dir, storage)?;
    let doc = welcome_doc(peer.peer_id)?;
    crate::storage::seed_snapshot(storage, &dek, doc_id, &doc)?;
    drop(peer);
    Ok((doc_id, dek))
}

/// The local-only workspace's doc, if `profile` is one: what signup
/// adopts. `None` when there is no profile or it already has an
/// account.
pub fn local_only_doc(profile: &Profile) -> anyhow::Result<Option<(DocId, Dek)>> {
    if !profile.exists() {
        return Ok(None);
    }
    let secrets = profile.read_secrets()?;
    if secrets.has_account() {
        return Ok(None);
    }
    let storage = crate::storage::open_storage(profile)?;
    let account = storage.read_account()?;
    let dek = crate::keystore::dek_for_doc(&secrets, account.primary_doc_id)?;
    Ok(Some((account.primary_doc_id, dek)))
}
