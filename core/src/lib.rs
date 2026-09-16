//! Monoplan core: encryption + Loro CRDT engine + sync logic.

pub mod crypto;
pub mod doc;
pub mod events;
pub mod storage;
pub mod sync;

pub use crypto::*;
pub use doc::{
    DefaultView, Doc, DocError, ExportItem, ExportLifecycle, ExportList, ExportSettings,
    INBOX_NAME, ImportSummary, ItemLifecycle, ItemView, JsonExport, LIST_INBOX, LaneSet, ListView,
    MAX_DURATION_MINUTES, NOTES_ORIGIN_PREFIX, NotesDeltaOp, SettingsView, WorkflowState,
};
pub use events::AppEvent;
pub use storage::{
    BootError, BootMeta, BootState, DocId, InFlightPush, LocalSeq, LocalStorage, MemStorage,
    PushId, RemoteWalRow, ServerSeq, SnapshotRow, StorageError, WalRow, boot_doc, has_unsynced_ops,
    load_doc, seed_snapshot,
};
pub use sync::{EngineOptions, Event, SyncEngine};
