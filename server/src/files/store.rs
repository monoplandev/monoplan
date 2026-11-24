//! Where attachment bytes live (`spec/attachments.md` §Server).
//!
//! A small interface keyed by `<doc_id>/<file_id>`. The `doc_files` row
//! is the record; this holds only the opaque bodies. An enum rather
//! than a trait object: the backends are few and chosen once at
//! startup, and the S3-compatible one slots in as a third variant.

use std::collections::HashMap;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use anyhow::Context;
use bytes::Bytes;
use tokio::io::AsyncWriteExt;
use uuid::Uuid;

use super::FileId;

/// Prefix of a body still being written by the directory backend. A
/// crash can leave one behind; `DirStore::open` removes them.
const TEMP_PREFIX: &str = ".tmp-";

#[derive(Clone)]
pub enum BlobStore {
    /// `<root>/<doc_id>/<file_id>` on the local filesystem. What
    /// self-hosters, local dev and the route tests run.
    Dir(DirStore),
    /// Process memory. For tests that never touch attachments, so they
    /// need no directory.
    Mem(MemStore),
}

impl BlobStore {
    pub async fn put(&self, doc_id: Uuid, file_id: FileId, body: Bytes) -> anyhow::Result<()> {
        match self {
            BlobStore::Dir(s) => s.put(doc_id, file_id, body).await,
            BlobStore::Mem(s) => {
                s.lock().insert((doc_id, file_id), body);
                Ok(())
            }
        }
    }

    /// `None` when nothing is stored under the key.
    pub async fn get(&self, doc_id: Uuid, file_id: FileId) -> anyhow::Result<Option<Bytes>> {
        match self {
            BlobStore::Dir(s) => s.get(doc_id, file_id).await,
            BlobStore::Mem(s) => Ok(s.lock().get(&(doc_id, file_id)).cloned()),
        }
    }

    /// Duplicate a body into another doc under the same id. `false`
    /// when the source is not stored.
    pub async fn copy(
        &self,
        from_doc: Uuid,
        file_id: FileId,
        to_doc: Uuid,
    ) -> anyhow::Result<bool> {
        match self {
            BlobStore::Dir(s) => s.copy(from_doc, file_id, to_doc).await,
            BlobStore::Mem(s) => {
                let mut map = s.lock();
                let Some(body) = map.get(&(from_doc, file_id)).cloned() else {
                    return Ok(false);
                };
                map.insert((to_doc, file_id), body);
                Ok(true)
            }
        }
    }

    /// Idempotent: a key with nothing stored is not an error.
    pub async fn delete(&self, doc_id: Uuid, file_id: FileId) -> anyhow::Result<()> {
        match self {
            BlobStore::Dir(s) => s.delete(doc_id, file_id).await,
            BlobStore::Mem(s) => {
                s.lock().remove(&(doc_id, file_id));
                Ok(())
            }
        }
    }

    /// Everything stored for a doc. Idempotent.
    pub async fn delete_doc(&self, doc_id: Uuid) -> anyhow::Result<()> {
        match self {
            BlobStore::Dir(s) => s.delete_doc(doc_id).await,
            BlobStore::Mem(s) => {
                s.lock().retain(|(d, _), _| *d != doc_id);
                Ok(())
            }
        }
    }
}

#[derive(Clone, Default)]
pub struct MemStore {
    bodies: Arc<Mutex<HashMap<(Uuid, FileId), Bytes>>>,
}

impl MemStore {
    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<(Uuid, FileId), Bytes>> {
        self.bodies.lock().unwrap_or_else(|e| e.into_inner())
    }
}

#[derive(Clone)]
pub struct DirStore {
    root: Arc<PathBuf>,
}

impl DirStore {
    /// No IO: directories are created on first write, so a server that
    /// never stores a file never creates one.
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self {
            root: Arc::new(root.into()),
        }
    }

    /// [`DirStore::new`] plus the startup pass that removes bodies a
    /// crash left half-written.
    pub fn open(root: impl Into<PathBuf>) -> anyhow::Result<Self> {
        let store = Self::new(root);
        let removed = store.remove_stray_temp_files()?;
        if removed > 0 {
            tracing::info!(removed, "removed stray attachment temp files");
        }
        Ok(store)
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    fn doc_dir(&self, doc_id: Uuid) -> PathBuf {
        self.root.join(doc_id.to_string())
    }

    fn path(&self, doc_id: Uuid, file_id: FileId) -> PathBuf {
        self.doc_dir(doc_id).join(file_id.to_string())
    }

    fn temp_path(&self, doc_id: Uuid) -> PathBuf {
        self.doc_dir(doc_id)
            .join(format!("{TEMP_PREFIX}{}", Uuid::now_v7().simple()))
    }

    /// Write beside the destination, then rename into place: a reader
    /// never sees a partial body, and a crash leaves only a temp file.
    async fn put(&self, doc_id: Uuid, file_id: FileId, body: Bytes) -> anyhow::Result<()> {
        tokio::fs::create_dir_all(self.doc_dir(doc_id))
            .await
            .context("attachments: create doc directory")?;
        let temp = self.temp_path(doc_id);
        let written = async {
            let mut file = tokio::fs::File::create(&temp).await?;
            file.write_all(&body).await?;
            file.sync_all().await?;
            drop(file);
            tokio::fs::rename(&temp, self.path(doc_id, file_id)).await
        }
        .await;
        if written.is_err() {
            let _ = tokio::fs::remove_file(&temp).await;
        }
        written.context("attachments: write body")
    }

    async fn get(&self, doc_id: Uuid, file_id: FileId) -> anyhow::Result<Option<Bytes>> {
        match tokio::fs::read(self.path(doc_id, file_id)).await {
            Ok(body) => Ok(Some(Bytes::from(body))),
            Err(e) if e.kind() == ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e).context("attachments: read body"),
        }
    }

    async fn copy(&self, from_doc: Uuid, file_id: FileId, to_doc: Uuid) -> anyhow::Result<bool> {
        tokio::fs::create_dir_all(self.doc_dir(to_doc))
            .await
            .context("attachments: create doc directory")?;
        let temp = self.temp_path(to_doc);
        match tokio::fs::copy(self.path(from_doc, file_id), &temp).await {
            Ok(_) => {}
            Err(e) if e.kind() == ErrorKind::NotFound => return Ok(false),
            Err(e) => {
                let _ = tokio::fs::remove_file(&temp).await;
                return Err(e).context("attachments: copy body");
            }
        }
        if let Err(e) = tokio::fs::rename(&temp, self.path(to_doc, file_id)).await {
            let _ = tokio::fs::remove_file(&temp).await;
            return Err(e).context("attachments: place copied body");
        }
        Ok(true)
    }

    async fn delete(&self, doc_id: Uuid, file_id: FileId) -> anyhow::Result<()> {
        ignore_missing(tokio::fs::remove_file(self.path(doc_id, file_id)).await)
            .context("attachments: delete body")
    }

    async fn delete_doc(&self, doc_id: Uuid) -> anyhow::Result<()> {
        ignore_missing(tokio::fs::remove_dir_all(self.doc_dir(doc_id)).await)
            .context("attachments: delete doc directory")
    }

    fn remove_stray_temp_files(&self) -> anyhow::Result<usize> {
        let docs = match std::fs::read_dir(self.root.as_path()) {
            Ok(d) => d,
            Err(e) if e.kind() == ErrorKind::NotFound => return Ok(0),
            Err(e) => return Err(e).context("attachments: read root directory"),
        };
        let mut removed = 0;
        for doc in docs {
            let doc = doc?;
            if !doc.file_type()?.is_dir() {
                continue;
            }
            for entry in std::fs::read_dir(doc.path())? {
                let entry = entry?;
                if entry.file_name().to_string_lossy().starts_with(TEMP_PREFIX) {
                    std::fs::remove_file(entry.path())?;
                    removed += 1;
                }
            }
        }
        Ok(removed)
    }
}

fn ignore_missing(result: std::io::Result<()>) -> std::io::Result<()> {
    match result {
        Err(e) if e.kind() == ErrorKind::NotFound => Ok(()),
        other => other,
    }
}
