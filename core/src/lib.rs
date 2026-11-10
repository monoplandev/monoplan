//! Monoplan core: encryption + Loro CRDT replica + sync logic.

pub mod crypto;
pub mod doc;
pub mod events;
pub mod places;
pub mod search;
pub mod storage;
pub mod sync;

pub use crypto::*;
pub use doc::{
    DefaultView, Doc, DocError, EVENTS_NAME, ExportItem, ExportLifecycle, ExportList,
    ExportSettings, FocusRef, INBOX_NAME, ImportSummary, ItemLifecycle, ItemView, JsonExport,
    LIST_EVENTS, LIST_INBOX, LaneSet, ListView, MAX_DURATION_MINUTES, MAX_PLACE_DETAIL_CHARS,
    MAX_PLACE_LABEL_CHARS, MIRROR_ORIGIN, NOTES_ORIGIN_PREFIX, NotesDeltaOp, Place, SettingsView,
    WELCOME_ITEM, WorkflowState,
};
pub use events::AppEvent;
/// Loro's version vector, re-exported so hosts that keep one per
/// consumer (the daemon's mirror fan-out) need no `loro` dependency.
pub use loro::VersionVector;
pub use places::{PlaceSuggestion, fold_label, place_matches, suggest_places};
pub use search::{SearchIndex, SearchKind, SearchResult, matches_name, tokenize};
pub use storage::{
    BootError, BootMeta, BootState, DocId, InFlightPush, LocalSeq, LocalStorage, MemStorage,
    PushId, RemoteWalRow, ServerSeq, SnapshotRow, StorageError, WalRow, boot_doc, has_unsynced_ops,
    load_doc, seed_snapshot,
};
pub use sync::{
    EngineOptions, Event, HelloError, Replica, hello_frame, parse_hello_response,
    server_frame_doc_id,
};
