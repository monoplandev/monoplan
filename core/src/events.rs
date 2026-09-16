//! Domain-level change events emitted by `Doc` after every commit
//! (local mutation) or remote import. Local mutation methods push
//! exact events surgically; remote imports are translated from Loro's
//! per-container diffs by `Doc::translate_captured_diffs`. Bulk/opaque
//! frames emit one `FullResync` control event instead of N synthetic item
//! events.
//!
//! These are the contract between the core and every UI layer. A
//! consumer (Solid store, SwiftUI `@Observable`, Compose `StateFlow`)
//! mirrors each event into its native reactive primitive with a
//! surgical write — no diff or reconciliation needed at the UI layer.
//!
//! Initial attachment materializes current state explicitly. After that,
//! consumers receive live deltas or an occasional `FullResync` request.

use crate::doc::{DefaultView, NotesDeltaOp, WorkflowState};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AppEvent {
    /// The doc changed by a bulk or opaque operation that is cheaper and
    /// safer for consumers to rematerialize wholesale. This is a control
    /// signal only: consumers fetch one current-state snapshot rather than
    /// receiving thousands of synthetic per-item events.
    FullResync,

    // ---------- items ----------
    /// New item appeared (local add or remote insert), or backfill on
    /// initial attach. `open_index` is the position within the *Open*
    /// projection of `list_id` (the four open workflow states;
    /// Done/binned excluded) — `None` when the item is not open. UI
    /// layers that keep per-list Open arrays splice at `open_index`.
    /// There is no global item order in the v2+ schema, so there is no
    /// doc-wide index.
    ItemAdded {
        id: String,
        list_id: String,
        text: String,
        notes: String,
        created_at: i64,
        /// Workflow register state (`spec/data-model.md` "Lifecycle").
        /// Masked by `binned_at` when that is set.
        state: WorkflowState,
        /// The workflow register's transition timestamp (unix millis) —
        /// `created_at` for items whose register is absent.
        lifecycle_at: i64,
        /// Reflection stamp: first entry into In Progress, if any.
        started_at: Option<i64>,
        /// Reflection stamp: last entry into Done, if any. View sorts use
        /// `lifecycle_at`, not this.
        done_at: Option<i64>,
        /// Bin mask: present ≡ binned (masking the workflow state).
        binned_at: Option<i64>,
        /// Date-only deadline (`YYYY-MM-DD`) or `None`. Floating local
        /// calendar date — consumers format without timezone conversion.
        deadline: Option<String>,
        /// Planned date (`YYYY-MM-DD` or `YYYY-MM-DDTHH:MM`) or `None`.
        /// Floating wall-clock; consumers format without zone conversion.
        when: Option<String>,
        /// Duration in whole minutes, or `None`.
        duration: Option<u32>,
        open_index: Option<usize>,
    },
    /// Item removed from the doc (deleteBinned / emptyBin). Toggling
    /// `binned_at` does *not* emit this — that emits
    /// `ItemLifecycleChanged`.
    ItemRemoved {
        id: String,
    },
    /// Item changed position within its list's order (an in-list
    /// reorder, or a passive shift caused by a neighbour's move).
    /// Cross-list moves emit `ItemListChanged` instead. `open_index` is
    /// the item's resulting position within its list's Open projection;
    /// `None` when the item is done/binned, whose ordering is
    /// view-local (timestamp sorts), not CRDT order.
    ItemMoved {
        id: String,
        open_index: Option<usize>,
    },
    ItemTextChanged {
        id: String,
        text: String,
    },
    ItemNotesChanged {
        id: String,
        notes: String,
    },
    /// An item's notes changed by a remote, other-tab, or undo write
    /// while a notes editor is subscribed (`Doc::subscribe_notes`).
    /// Emitted after the matching `ItemNotesChanged`; the delta is in
    /// UTF-16 units against the text the subscriber last saw. Local
    /// `apply_notes_delta` writes never echo back as this event.
    ItemNotesDelta {
        id: String,
        delta: Vec<NotesDeltaOp>,
    },
    /// Item's date-only deadline changed. The payload is the raw
    /// `YYYY-MM-DD` value after the write — `None` when cleared. The
    /// value is a floating local calendar date; consumers format it
    /// locally without timezone conversion.
    ItemDeadlineChanged {
        id: String,
        deadline: Option<String>,
    },
    /// Item's planned date changed. The payload is the raw value after
    /// the write (`YYYY-MM-DD` all-day or `YYYY-MM-DDTHH:MM` timed) —
    /// `None` when cleared. Floating; format locally.
    ItemWhenChanged {
        id: String,
        when: Option<String>,
    },
    /// Item's duration changed. The payload is the value in whole
    /// minutes after the write — `None` when cleared (including the
    /// clear that rides along with clearing `when`).
    ItemDurationChanged {
        id: String,
        duration: Option<u32>,
    },
    /// Lifecycle changed (`spec/data-model.md`). Emitted whenever the
    /// workflow register, a reflection stamp, or the `binned_at` mask
    /// transitions; the payload carries all current values so consumers
    /// resolve the state (binned mask wins while present) without
    /// rereading the doc. `open_index` is the item's position within its
    /// list's Open projection when the item is open after the change
    /// (restore / un-done re-entry point, or an open→open workflow flip
    /// that keeps it in place); `None` when it is Done/binned (consumers
    /// drop it from the Open array).
    ItemLifecycleChanged {
        id: String,
        /// Workflow register state after the change.
        state: WorkflowState,
        /// The register's transition timestamp after the change.
        lifecycle_at: i64,
        started_at: Option<i64>,
        done_at: Option<i64>,
        binned_at: Option<i64>,
        open_index: Option<usize>,
    },
    /// Item's `list_id` field changed without changing position
    /// (e.g. orphan reassignment when a list is deleted), or alongside
    /// an `ItemMoved` (cross-list drag). `open_index` is the item's
    /// position within the *new* list's Open projection; `None` when
    /// the item is done/binned.
    ItemListChanged {
        id: String,
        list_id: String,
        open_index: Option<usize>,
    },

    // ---------- lists ----------
    ListAdded {
        id: String,
        name: String,
        created_at: i64,
        /// Archive timestamp (`spec/data-model.md` "Archived lists"):
        /// `Some` when the list is archived. Carried here — not just on
        /// `ListArchivedChanged` — because snapshot/backfill consumers
        /// must materialize archived lists correctly from the add burst.
        archived_at: Option<i64>,
        index: usize,
    },
    ListRemoved {
        id: String,
    },
    ListMoved {
        id: String,
        index: usize,
    },
    ListRenamed {
        id: String,
        name: String,
    },
    /// A user-created list's display icon was set or cleared. `icon` is
    /// the literal emoji grapheme after the change, or `None` when the
    /// icon was removed (consumers fall back to the built-in glyph).
    ListIconChanged {
        id: String,
        icon: Option<String>,
    },
    /// A user-created list's saved default view was set or cleared
    /// (`spec/board.md`). `view` is the value after the change; `None`
    /// means no saved default, so clients fall back to their own. The
    /// reserved `inbox` list has no ListMeta row — its default rides on
    /// `SettingsChanged.inbox_view` instead. A client that has its own
    /// local override for this list keeps it; the default only decides
    /// what an un-overridden client renders.
    ListDefaultViewChanged {
        id: String,
        view: Option<DefaultView>,
    },
    /// A user-created list was archived or unarchived
    /// (`spec/data-model.md` "Archived lists"). `archived_at` is the
    /// value after the change — `Some(ts)` when archived, `None` when
    /// restored to the active workspace. Metadata-only: no item, order,
    /// lifecycle, or Focus events accompany it.
    ListArchivedChanged {
        id: String,
        archived_at: Option<i64>,
    },

    // ---------- focus ----------
    /// The Focus lens (`spec/focus.md`) changed — a ref was added, removed,
    /// reordered, or swept, including the auto-removal when a focused item
    /// goes Done. Carries no payload: visibility depends on item lifecycle
    /// too, so consumers re-derive `focus_view()` on this event *and* on
    /// item events. Emitted once per focus-mutating commit.
    FocusChanged,

    // ---------- workspace settings ----------
    /// Doc-level synced settings changed. The payload carries the
    /// current known value for each surfaced field so consumers can
    /// mirror a small settings object with a single write.
    SettingsChanged {
        /// When true, clients render each non-Inbox list's open-item
        /// count (all Open states) in the nav (subject to the count > 0
        /// gate). Inbox always shows its count regardless. Single global flag —
        /// there is no per-list override.
        show_list_counts: bool,
        /// The reserved `inbox` list's saved default view (`spec/board.md`),
        /// or `None` when none is saved. Inbox has no ListMeta row, so its
        /// default travels on the settings event rather than
        /// `ListDefaultViewChanged`.
        inbox_view: Option<DefaultView>,
    },
}
