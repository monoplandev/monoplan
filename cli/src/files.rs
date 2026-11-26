//! The host half of attachments (`spec/attachments.md` "Client flows"),
//! for the engine loop and its in-process clients: everything that
//! moves bytes or talks to the file routes. The doc half (entries,
//! keys, purge marks) is the core's. What `js/web/src/sync/files.ts`
//! is to the web:
//!
//!   - [`attach`]: seal under a fresh key, `PUT`, then write the entry
//!     (upload first, reference second),
//!   - [`fetch`]: `GET`, open with the entry's key, write the plaintext
//!     where the caller says; a tombstoned blob the doc still names is
//!     restored and fetched again,
//!   - [`copy_for_move`]: before a cross-doc move the server duplicates
//!     each blob into the target doc,
//!   - purge and sweep, the housekeeping: [`Housekeeping`] is the
//!     loop's state for it, [`run_jobs`] the part that waits on the
//!     server, beside the loop.
//!
//! The first three are driven by a client of the loop
//! ([`LocalClient`]): each is a doc call, a wait on the server, and a
//! second doc call, so the loop never waits on the network. They need
//! an account; a local-only workspace has no server to hold the bytes.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, bail};
use monoplan_core::{DocId, FILE_KEY_LEN, open_file, random_bytes, seal_file};
use monoplan_protocol::{CopyFileRequest, FilesListResponse};
use tokio::time::Instant;

use crate::api::moves::{self, ItemRef};
use crate::config::Profile;
use crate::daemon::LocalClient;
use crate::net::{Client, NetError};

/// Largest file that can be attached: the server's cap on the
/// plaintext (`spec/attachments.md` §Routes).
pub const MAX_FILE_BYTES: u64 = 25 * 1024 * 1024;

const DAY_MS: i64 = 24 * 60 * 60 * 1000;
/// How long after the last pulse the housekeeping runs, when there is
/// no synced purge mark to act on at once.
const SETTLE: Duration = Duration::from_secs(3);
/// When a purge that stopped on a failed request is tried again, if
/// nothing pulses sooner.
const RETRY: Duration = Duration::from_secs(60);
/// Where each doc's last sweep is kept, in the profile dir.
const SWEEP_FILE: &str = "files-sweep.json";

/// The server and this device's token, read once per flow.
pub(crate) struct Conn {
    client: Client,
    token: String,
}

impl Conn {
    pub(crate) fn open(profile: &Profile) -> anyhow::Result<Conn> {
        let secrets = profile.read_secrets()?;
        if !secrets.has_account() {
            bail!("attachments need an account");
        }
        let token = secrets.device_token()?.to_string();
        Ok(Conn {
            client: Client::new(profile.read_config()?.server_url),
            token,
        })
    }

    fn path(doc: DocId, file_id: &str) -> String {
        format!("/api/docs/{}/files/{file_id}", doc.0)
    }

    async fn put(&self, doc: DocId, file_id: &str, sealed: Vec<u8>) -> Result<(), NetError> {
        self.client
            .put_bytes(&Self::path(doc, file_id), &self.token, sealed)
            .await
    }

    async fn get(&self, doc: DocId, file_id: &str) -> Result<Vec<u8>, NetError> {
        self.client
            .get_bytes(&Self::path(doc, file_id), &self.token)
            .await
    }

    async fn delete(&self, doc: DocId, file_id: &str) -> Result<(), NetError> {
        self.client
            .delete_authed(&Self::path(doc, file_id), &self.token)
            .await
    }

    async fn tombstone(&self, doc: DocId, file_id: &str) -> Result<(), NetError> {
        let path = format!("{}/tombstone", Self::path(doc, file_id));
        self.client
            .post_authed_no_response(&path, &self.token, &serde_json::json!({}))
            .await
    }

    async fn restore(&self, doc: DocId, file_id: &str) -> Result<(), NetError> {
        let path = format!("{}/restore", Self::path(doc, file_id));
        self.client
            .post_authed_no_response(&path, &self.token, &serde_json::json!({}))
            .await
    }

    async fn copy(&self, doc: DocId, file_id: &str, to: DocId) -> Result<(), NetError> {
        let path = format!("{}/copy", Self::path(doc, file_id));
        let body = CopyFileRequest {
            to_doc_id: to.0.to_string(),
        };
        self.client
            .post_authed_no_response(&path, &self.token, &body)
            .await
    }

    async fn list(&self, doc: DocId) -> Result<FilesListResponse, NetError> {
        self.client
            .get_authed(&format!("/api/docs/{}/files", doc.0), &self.token)
            .await
    }
}

/// The HTTP status of a refusal; `None` when the request never got an
/// answer.
fn status_of(e: &NetError) -> Option<u16> {
    match e {
        NetError::Api { status, .. } => Some(status.as_u16()),
        _ => None,
    }
}

/// A failed request as a person reads it in the app.
fn said(e: NetError) -> anyhow::Error {
    match status_of(&e) {
        Some(413) => anyhow::anyhow!("the file is over 25 MB"),
        Some(507) => anyhow::anyhow!("the workspace owner's storage is full"),
        Some(404 | 410) => unavailable(),
        Some(_) => e.into(),
        None => anyhow::anyhow!("could not reach the server"),
    }
}

fn unavailable() -> anyhow::Error {
    anyhow::anyhow!("the file is no longer available")
}

/// A name the core accepts for what the platform reported: control
/// characters out, trimmed, and cut to 255 bytes with the extension
/// kept. Empty comes back as `file`. The web's `cleanFileName`.
pub fn clean_file_name(raw: &str) -> String {
    const MAX: usize = monoplan_core::MAX_FILE_NAME_BYTES;
    let name: String = raw.chars().filter(|c| !c.is_control()).collect();
    let name = name.trim();
    if name.is_empty() {
        return "file".into();
    }
    if name.len() <= MAX {
        return name.to_string();
    }
    let ext = match name.rfind('.') {
        Some(dot) if dot > 0 && name[dot..].chars().count() <= 16 => &name[dot..],
        _ => "",
    };
    let mut stem: Vec<char> = name[..name.len() - ext.len()].chars().collect();
    loop {
        let cut: String = stem.iter().collect();
        let cut = cut.trim_end();
        if stem.len() <= 1 || cut.len() + ext.len() <= MAX {
            return format!("{cut}{ext}");
        }
        stem.pop();
    }
}

/// Upload the file at `path` and attach it to `item_id` of the
/// workspace `workspace` names (`None`: Home), as `name` (cleaned to
/// what the core accepts) and `mime` (empty when unknown). Its new file
/// id, once the entry is in the doc; the doc is untouched otherwise.
pub async fn attach(
    client: &LocalClient,
    profile: &Profile,
    workspace: Option<String>,
    item_id: String,
    path: &Path,
    name: &str,
    mime: &str,
) -> anyhow::Result<String> {
    let conn = Conn::open(profile)?;
    let size = std::fs::metadata(path)
        .with_context(|| format!("read {}", path.display()))?
        .len();
    if size > MAX_FILE_BYTES {
        bail!("the file is over 25 MB");
    }
    let doc = {
        let item_id = item_id.clone();
        client
            .call(workspace.clone(), move |cx| {
                crate::api::items::item_view(cx, &item_id)?;
                cx.doc_id().context("the workspace is not open")
            })
            .await?
    };
    let plain = std::fs::read(path).with_context(|| format!("read {}", path.display()))?;
    // The size on disk can have changed since; the bytes read are what
    // counts.
    if plain.len() as u64 > MAX_FILE_BYTES {
        bail!("the file is over 25 MB");
    }
    let key: [u8; FILE_KEY_LEN] = random_bytes();
    let file_id = hex::encode(random_bytes::<16>());
    let sealed = seal_file(&key, &plain).map_err(|e| anyhow::anyhow!("seal: {e}"))?;
    match conn.put(doc, &file_id, sealed).await {
        Ok(()) => {}
        // The id is random and ours, so the blob is already there.
        Err(e) if status_of(&e) == Some(409) => {}
        Err(e) => return Err(said(e)),
    }
    // Reference second. If the item or the doc went away during the
    // upload the blob stays unnamed and the owner's sweep collects it.
    let name = clean_file_name(name);
    let mime = if mime.len() <= monoplan_core::MAX_FILE_MIME_BYTES {
        mime.to_string()
    } else {
        String::new()
    };
    let size = plain.len() as u64;
    let id = file_id.clone();
    client
        .call(workspace, move |cx| {
            anyhow::ensure!(cx.doc_id() == Some(doc), "the workspace changed");
            cx.doc
                .attach_file(&item_id, &id, &name, &mime, size, &key)?;
            Ok(())
        })
        .await?;
    Ok(file_id)
}

/// Fetch a live attachment of the workspace and write its plaintext to
/// `dest` (a temp file of the caller's: decrypted bytes are never kept
/// in the profile). Nothing is cached; every call fetches.
pub async fn fetch(
    client: &LocalClient,
    profile: &Profile,
    workspace: Option<String>,
    file_id: String,
    dest: &Path,
) -> anyhow::Result<()> {
    let conn = Conn::open(profile)?;
    let id = file_id.clone();
    let (doc, key) = client
        .call(workspace, move |cx| {
            let key = cx.doc.file_key(&id).ok_or_else(unavailable)?;
            Ok((cx.doc_id().context("the workspace is not open")?, key))
        })
        .await?;
    let sealed = match conn.get(doc, &file_id).await {
        Ok(sealed) => sealed,
        // The doc still names a blob the server has tombstoned (an
        // undone move, a stale device's removal): the reference wins
        // inside the grace period.
        Err(e) if status_of(&e) == Some(410) => {
            conn.restore(doc, &file_id).await.map_err(said)?;
            conn.get(doc, &file_id).await.map_err(said)?
        }
        Err(e) => return Err(said(e)),
    };
    let plain = open_file(&key, &sealed).map_err(|_| unavailable())?;
    std::fs::write(dest, plain).with_context(|| format!("write {}", dest.display()))?;
    Ok(())
}

/// Before a cross-doc move of `items` to the workspace `to` names
/// (`None`: Home): have the server copy each live blob of the crossing
/// items into the target doc under its own id. Fails on the first copy
/// that fails, and the caller then moves nothing. A move that carries
/// no file asks the server nothing, so it works offline as before.
pub async fn copy_for_move(
    client: &LocalClient,
    profile: &Profile,
    items: Vec<ItemRef>,
    to: Option<String>,
) -> anyhow::Result<()> {
    let crossing = client
        .call(None, move |cx| {
            moves::files_crossing(cx, &items, to.as_deref())
        })
        .await?;
    let Some(crossing) = crossing.filter(|c| !c.files.is_empty()) else {
        return Ok(());
    };
    let conn = Conn::open(profile)?;
    let to = crossing.to;
    for (doc, file_id) in &crossing.files {
        let copied = match conn.copy(*doc, file_id, to).await {
            // Tombstoned at the source though the doc names it:
            // restore, then copy.
            Err(e) if status_of(&e) == Some(410) => match conn.restore(*doc, file_id).await {
                Ok(()) => conn.copy(*doc, file_id, to).await,
                Err(e) => Err(e),
            },
            other => other,
        };
        match copied {
            Ok(()) => {}
            // The target already has the id: the item is moving back
            // to a doc it left. Its blob there may be tombstoned by now.
            Err(e) if status_of(&e) == Some(409) => {
                conn.restore(to, file_id).await.map_err(said)?;
            }
            // The source has no such blob: the file was already
            // unavailable, and stays so after the move.
            Err(e) if status_of(&e) == Some(404) => {}
            Err(e) => {
                return Err(said(e).context("the item's files could not be moved, so it stays"));
            }
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------
// Housekeeping: purge and sweep (`spec/attachments.md` "Purge",
// "Garbage"). The loop pulses after each server frame and when the
// socket comes up. A pulse that finds entries marked for purge on a doc
// with nothing unsent runs at once, so a removal reaches the server as
// soon as its mark has; any other pulse settles a few seconds after the
// last one. Nothing acts on a doc whose own changes are still local, so
// a removal is never acted on before the mark that explains it is on
// the server.
// ---------------------------------------------------------------------

/// What one run asks the server for a doc.
#[derive(Debug)]
pub(crate) struct Job {
    pub doc: DocId,
    /// Every entry marked for purge.
    pub marked: Vec<String>,
    /// Those among them whose bytes are destroyed at once; the rest
    /// are tombstoned.
    pub destroy: HashSet<String>,
    /// The doc's daily sweep is due: fetch the list too.
    pub list: bool,
}

/// What the server answered for a [`Job`].
#[derive(Debug)]
pub(crate) struct Outcome {
    pub doc: DocId,
    /// The ids the server answered for (done, or already gone): their
    /// entries can be forgotten.
    pub gone: Vec<String>,
    /// False when the purge stopped on a failed request.
    pub complete: bool,
    pub list: Option<FilesListResponse>,
}

/// Run the jobs against the server, one request at a time.
pub(crate) async fn run_jobs(conn: Conn, jobs: Vec<Job>) -> Vec<Outcome> {
    let mut out = Vec::with_capacity(jobs.len());
    for job in jobs {
        let mut gone = Vec::new();
        let mut complete = true;
        for file_id in &job.marked {
            let sent = if job.destroy.contains(file_id) {
                conn.delete(job.doc, file_id).await
            } else {
                conn.tombstone(job.doc, file_id).await
            };
            match sent.as_ref().map_err(status_of) {
                Ok(()) | Err(Some(404 | 410)) => gone.push(file_id.clone()),
                Err(status) => {
                    if status.is_some() {
                        tracing::warn!("files: purge stopped: {:?}", sent.err());
                    }
                    complete = false;
                    break;
                }
            }
        }
        let list = if job.list && complete {
            match conn.list(job.doc).await {
                Ok(list) => Some(list),
                Err(e) => {
                    if status_of(&e).is_some() {
                        tracing::warn!("files: sweep skipped: {e}");
                    }
                    None
                }
            }
        } else {
            None
        };
        out.push(Outcome {
            doc: job.doc,
            gone,
            complete,
            list,
        });
    }
    out
}

/// Tombstone blobs no entry names (the sweep's second half), beside
/// the loop. Stops at the first request that gets no answer.
pub(crate) async fn tombstone_orphans(conn: Conn, doc: DocId, file_ids: Vec<String>) {
    for file_id in file_ids {
        if let Err(e) = conn.tombstone(doc, &file_id).await
            && status_of(&e).is_none()
        {
            return;
        }
    }
}

/// The rows of a doc's list the sweep tombstones: not tombstoned
/// already, over a day old, and named by no entry of the doc, live or
/// marked.
pub(crate) fn orphans(list: &FilesListResponse, named: &[String], cutoff: i64) -> Vec<String> {
    let named: HashSet<&str> = named.iter().map(String::as_str).collect();
    list.files
        .iter()
        .filter(|row| {
            row.deleted_at.is_none() && row.created_at < cutoff && !named.contains(row.id.as_str())
        })
        .map(|row| row.id.clone())
        .collect()
}

/// The loop's housekeeping state.
pub(crate) struct Housekeeping {
    /// When the next run starts, if one is wanted.
    pub due: Option<Instant>,
    /// The run in flight.
    pub running: Option<tokio::task::JoinHandle<Vec<Outcome>>>,
    /// Docs whose last purge stopped on a failed request: their marks
    /// wait out the settle delay again, so a server that keeps failing
    /// is not asked once per frame.
    pub stalled: HashSet<DocId>,
    /// When each doc was last swept on this device (unix millis), so a
    /// doc is swept at most once a day.
    swept: HashMap<String, i64>,
    sweep_path: PathBuf,
}

impl Housekeeping {
    pub fn new(profile_dir: &Path) -> Self {
        let sweep_path = profile_dir.join(SWEEP_FILE);
        let swept = std::fs::read(&sweep_path)
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default();
        Housekeeping {
            due: None,
            running: None,
            stalled: HashSet::new(),
            swept,
            sweep_path,
        }
    }

    /// Something happened: run now when `at_once` (a synced purge mark
    /// is waiting), else once things go quiet.
    pub fn pulse(&mut self, at_once: bool) {
        self.due = Some(if at_once {
            Instant::now()
        } else {
            Instant::now() + SETTLE
        });
    }

    /// A purge stopped short: try again later if nothing pulses first.
    pub fn retry(&mut self) {
        if self.due.is_none() {
            self.due = Some(Instant::now() + RETRY);
        }
    }

    pub fn sweep_due(&self, doc: DocId, now: i64) -> bool {
        match self.swept.get(&doc.0.to_string()) {
            // A stamp in the future is a clock that moved: sweep.
            Some(last) => now - last >= DAY_MS || now < *last,
            None => true,
        }
    }

    pub fn stamp_sweep(&mut self, doc: DocId, now: i64) {
        self.swept.insert(doc.0.to_string(), now);
        // Lost on a failed write: the doc is swept again next run,
        // which is harmless.
        if let Ok(bytes) = serde_json::to_vec(&self.swept) {
            let _ = std::fs::write(&self.sweep_path, bytes);
        }
    }

    pub fn forget(&mut self, doc: DocId) {
        self.stalled.remove(&doc);
    }
}

/// What the sweep keeps of entries and rows younger than this: an
/// upload still on its way to its commit.
pub(crate) const SWEEP_AGE_MS: i64 = DAY_MS;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clean_file_name_strips_trims_and_keeps_the_extension() {
        assert_eq!(clean_file_name("  a\u{0}b.txt \n"), "ab.txt");
        assert_eq!(clean_file_name("\u{7}"), "file");
        let long = format!("{}.pdf", "é".repeat(200));
        let cut = clean_file_name(&long);
        assert!(cut.len() <= 255, "{}", cut.len());
        assert!(cut.ends_with(".pdf"));
        assert!(cut.starts_with('é'));
        // No extension worth keeping: cut on a character boundary.
        let cut = clean_file_name(&"é".repeat(200));
        assert_eq!(cut.len(), 254);
    }

    #[test]
    fn orphans_are_old_live_rows_no_entry_names() {
        let row = |id: &str, created_at, deleted_at| monoplan_protocol::StoredFile {
            id: id.into(),
            bytes: 1,
            created_at,
            deleted_at,
        };
        let list = FilesListResponse {
            doc_bytes: 0,
            used_bytes: Some(0),
            limit_bytes: Some(0),
            files: vec![
                row("named", 0, None),
                row("young", 100, None),
                row("gone", 0, Some(5)),
                row("orphan", 0, None),
            ],
        };
        assert_eq!(orphans(&list, &["named".into()], 50), vec!["orphan"]);
    }
}
