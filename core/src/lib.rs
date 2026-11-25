//! Monoplan core: encryption + Loro CRDT replica + sync logic.

pub mod crypto;
pub mod doc;
pub mod events;
pub mod inbox;
pub mod places;
pub mod search;
pub mod storage;
pub mod sync;
pub mod zone;

pub use crypto::*;
pub use doc::{
    DefaultView, DeleteListMode, Doc, DocError, ExportFile, ExportItem, ExportLifecycle,
    ExportList, ExportSettings, FILE_KEY_LEN, FocusRef, FocusRetarget, INBOX_NAME, ImportSummary,
    ItemFile, ItemLifecycle, ItemView, JsonExport, LIST_INBOX, LaneSet, ListView,
    MAX_DURATION_MINUTES, MAX_FILE_MIME_BYTES, MAX_FILE_NAME_BYTES, MAX_PLACE_DETAIL_CHARS,
    MAX_PLACE_LABEL_CHARS, MAX_SUBTASK_TEXT_CHARS, MAX_SUBTASKS, NOTES_ORIGIN_PREFIX, NotesDeltaOp,
    Place, SettingsView, Subtask, VIEW_ORIGIN, WELCOME_ITEM, WorkflowState, move_items_across_docs,
};
pub use events::AppEvent;
/// Loro's version vector, re-exported so a host that holds one
/// (`Doc::oplog_vv`) needs no `loro` dependency.
pub use loro::VersionVector;
pub use places::{
    PlaceSuggestion, fold_label, merge_place_uses, place_matches, place_suggestions, place_uses,
    suggest_places,
};
pub use search::{SearchIndex, SearchKind, SearchResult, matches_name, tokenize};
pub use storage::{
    BootError, BootMeta, BootState, DocId, InFlightPush, LocalSeq, LocalStorage, MemStorage,
    PushId, RemoteWalRow, ServerSeq, SnapshotRow, StorageError, WalRow, boot_doc, has_unsynced_ops,
    load_doc, seed_snapshot,
};
pub use sync::{
    EngineOptions, Event, HelloError, Replica, hello_frame, is_memberships_changed,
    parse_hello_response, server_frame_doc_id,
};
