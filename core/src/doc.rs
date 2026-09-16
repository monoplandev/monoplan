//! Loro CRDT layer: typed mutations, persistence, op-stream framing,
//! and a deterministic logical-state fingerprint.
//!
//! Layout matches `spec/data-model.md` (schema v4):
//! - root container `items` (`LoroMap`) — keyed by the item's stable
//!   UUID; each value is a child `LoroMap` with `id`, `text`,
//!   `location`, `created_at`, optional `notes`, optional `lifecycle`
//!   (the atomic workflow register `[state, at]`), optional `binned_at`
//!   (the bin mask), and the optional reflection stamps `started_at` /
//!   `done_at`. The resolved lifecycle is the register's state, masked
//!   by `binned_at` while that is present (`spec/data-model.md`
//!   "Lifecycle"). An absent/unparseable register reads as
//!   `[Backlog, created_at]`.
//! - root container `lists` (`LoroMovableList`) — each entry is a
//!   `LoroMap` with `id`, `name`, `created_at`.
//! - root container `settings` (`LoroMap`) — account-wide synced
//!   workspace settings not owned by a specific list row.
//! - one root container `order/<list-id>` (`LoroMovableList`) per
//!   logical list — **scalar entries only**, each an encoded
//!   `"<item_id>:<placement_id>"` string. Ordering lives here;
//!   everything else lives on the item map. There is no document-wide
//!   item list: reordering one list touches only that list's container.
//!
//! An item's `location` is a single atomic register encoding
//! `"<list_id>:<placement_id>"`. It is authoritative for membership;
//! an order entry is *visible* only when its placement matches the
//! item's current location (see `spec/data-model.md` "Projection
//! invariants"). Stale/duplicate entries left behind by concurrent
//! cross-list moves are harmless and cleaned by [`Doc::reconcile`].
//!
//! Binned is a lifecycle & items keep their location. One well-known
//! list id is *reserved*: [`LIST_INBOX`]. It has **no ListMeta row** —
//! items reference it by string id and clients render it with a
//! hardcoded label ("Inbox").
//!
//! The struct holds a `last_persisted_vv` — the local WAL capture
//! cursor. Everything at or below it has been durably appended to the
//! local encrypted WAL (see `spec/local-storage.md`); `pending_export`
//! hands the engine exactly the commits past it. This cursor is about
//! *local durability only* — what the server has is tracked separately
//! by the engine's `server_known_vv`.

#[cfg(not(target_arch = "wasm32"))]
use std::time::{SystemTime, UNIX_EPOCH};

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::{Arc, Mutex};

use loro::event::{Diff as LoroDiff, DiffEvent, ListDiffItem};
use loro::{
    CommitOptions, Container, ContainerID, EventTriggerKind, ExportMode, Index, LoroDoc, LoroMap,
    LoroMovableList, LoroText, LoroValue, Subscription, TextDelta, UndoManager, UpdateOptions,
    ValueOrContainer, VersionVector,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::crypto::{AEAD_NONCE_LEN, Dek};
use crate::events::AppEvent;
use monoplan_protocol::EncryptedBlob;

pub const LIST_INBOX: &str = "inbox";
pub const INBOX_NAME: &str = "Inbox";

const ROOT_ITEMS: &str = "items";
const ROOT_LISTS: &str = "lists";
const ROOT_SETTINGS: &str = "settings";
/// Root-container name prefix for per-list order containers:
/// `order/main`, `order/<list-uuid>`. The container for a deleted list
/// is simply never projected again (root containers can't be removed).
const ORDER_PREFIX: &str = "order/";
/// Reserved singleton container name: the curated Focus lens
/// (`spec/focus.md`). A `LoroMovableList` of encoded `FocusRef` scalars
/// — never child containers, same discipline as order containers. Its
/// own element order *is* the Focus order.
const FOCUS_CONTAINER: &str = "focus";

const KEY_ID: &str = "id";
const KEY_TEXT: &str = "text";
const KEY_NOTES: &str = "notes";
/// Commit origin prefix for notes delta writes (`spec/notes-plan.md`
/// Phase 2). The workspace `UndoManager` excludes it, so typing in an
/// open notes editor never lands on the workspace undo stack; the
/// editor's own history owns notes undo while it is open.
pub const NOTES_ORIGIN_PREFIX: &str = "notes:";
/// Atomic location register: `"<list_id>:<placement_id>"`. Written as
/// one scalar so list membership and placement can never be torn apart
/// by concurrent edits. See `Location`.
const KEY_LOCATION: &str = "location";
/// Atomic workflow register (`spec/data-model.md` "Lifecycle"): a plain
/// `LoroValue` list `[state, at]` — the current `WorkflowState` code
/// (`0..=4`) and the unix millis it was entered. Whole-value LWW, so
/// state and timestamp can never be torn apart by concurrent edits.
/// Absent ≡ `[Backlog, created_at]`; new items omit it.
const KEY_LIFECYCLE: &str = "lifecycle";
/// Reflection stamp: set (write-once) the first time the item enters
/// In Progress; never cleared. Feeds analytics; no view reads it.
const KEY_STARTED_AT: &str = "started_at";
/// Optional date-only deadline: a floating local calendar date in
/// `YYYY-MM-DD` format (no time, no timezone). Absent ≡ no deadline;
/// the mutation deletes the key when cleared. Written on its own scalar
/// register; malformed values never reach the doc (validated in
/// `set_item_deadline`).
const KEY_DEADLINE: &str = "deadline";
/// Optional planned date: a floating local `YYYY-MM-DD` (all-day) or
/// `YYYY-MM-DDTHH:MM` (timed) wall-clock intent. Absent ≡ unset; the
/// mutation deletes the key when cleared. Shape-discriminated, one
/// register, so date and time can never tear under concurrent edit.
/// Validated in `set_item_when`; see `spec/calendar-plan.md`.
const KEY_WHEN: &str = "when";
/// Optional duration in whole minutes (`1..=MAX_DURATION_MINUTES`), a
/// length rather than an end so that moving `when` on one device and
/// setting the length on another can never produce an item that ends
/// before it starts. Meaningful only beside a timed `when`; views ignore
/// it otherwise. Absent ≡ unset; the mutation deletes the key when
/// cleared. Validated in `set_item_duration`; see `spec/calendar-plan.md`.
const KEY_DURATION: &str = "duration";
/// Upper bound on `duration`: one week of minutes.
pub const MAX_DURATION_MINUTES: u32 = 7 * 24 * 60;
const KEY_NAME: &str = "name";
/// Optional per-list display icon. Stored as the literal emoji grapheme
/// the user picked (e.g. `"📥"`); absent/empty means "no icon, render the
/// built-in fallback". A future SVG-icon set can share this key via a
/// `"svg:<id>"` prefix convention without a schema change.
const KEY_ICON: &str = "icon";
/// Optional saved default view for a list — the lens a client renders
/// before the user overrides it locally. One **encoded scalar string**
/// (`DefaultView`, e.g. `"board:backlog,in_progress,done"`), so the mode
/// and the visible-lane set can never be torn apart by concurrent saves.
/// Absent ≡ no saved default; clients fall back to their own built-in
/// default.
const KEY_VIEW: &str = "view";
const KEY_CREATED_AT: &str = "created_at";
/// Reflection stamp: set on each entry into Done; never cleared, so it
/// survives later binning and un-doing. View sorts use the workflow
/// register's `at`, not this.
const KEY_DONE_AT: &str = "done_at";
/// Bin mask: present ≡ binned (masking the workflow register); absent ≡
/// not binned. Restore deletes the key.
const KEY_BINNED_AT: &str = "binned_at";
/// ListMeta archive timestamp (`spec/data-model.md` "Archived lists").
/// Absent ≡ active; present ≡ archived. Written by `set_list_archived`;
/// unarchiving deletes the key. Metadata-only — archiving never touches
/// items, order containers, lifecycle, or Focus.
const KEY_ARCHIVED_AT: &str = "archived_at";
/// Global "show counts on non-Inbox lists" flag. Lives on the doc-level
/// settings map; Inbox's own count is always visible (when non-zero) and
/// is not gated by this. Absent ≡ false — written only when toggled on
/// (and removed when toggled back off) so docs that have never enabled
/// it carry no key.
const KEY_SHOW_LIST_COUNTS: &str = "show_list_counts";
/// The reserved `inbox` (Inbox) list's saved default view. Inbox has no
/// ListMeta row, so its `KEY_VIEW` equivalent lives on the doc-level
/// settings map — same encoding, same absent ≡ no-default semantics.
const KEY_INBOX_VIEW: &str = "inbox_view";
/// Batch lifecycle mutations at/above this many ids stop emitting
/// surgical per-item events and fall back to one whole-doc rebuild +
/// diff. Matches the web store's coarse-projection threshold so both
/// layers flip regimes together.
const BULK_LIFECYCLE_EVENT_THRESHOLD: usize = 64;
/// Captured operations touching at least this many items abandon per-item
/// diff translation for the whole-doc resync fallback — one O(doc) pass
/// beats per-item projection syncs for bulk imports or undo steps.
const DIFF_TRANSLATE_MAX_DIRTY: usize = 64;

/// One step of a plain-text delta over an item's notes, in the Quill
/// delta shape (`spec/notes-plan.md` "Editor bindings"). Positions and
/// counts are **UTF-16 code units**, the unit browser editors count in;
/// the core converts to Loro's Unicode-scalar indices internally so no
/// client ever handles the unit question. Serialises as
/// `{"retain": n}` / `{"insert": "s"}` / `{"delete": n}`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum NotesDeltaOp {
    Retain { retain: usize },
    Insert { insert: String },
    Delete { delete: usize },
}

#[derive(Debug, thiserror::Error)]
pub enum DocError {
    #[error("loro: {0}")]
    Loro(String),
    #[error("item not found: {0}")]
    ItemNotFound(String),
    #[error("list not found: {0}")]
    ListNotFound(String),
    #[error("can't delete the built-in list `{0}`")]
    CannotDeleteBuiltin(String),
    #[error("can't move the built-in list `{0}`")]
    CannotMoveBuiltin(String),
    #[error("can't rename the built-in list `{0}`")]
    CannotRenameBuiltin(String),
    #[error("item is not in the bin")]
    NotBinned,
    #[error("invalid input: {0}")]
    Invalid(String),
    #[error("crypto: {0}")]
    Crypto(#[from] crate::crypto::CryptoError),
    #[error("persistence decode: {0}")]
    Persistence(#[from] rmp_serde::decode::Error),
    #[error("persistence encode: {0}")]
    PersistenceEncode(#[from] rmp_serde::encode::Error),
}

impl From<loro::LoroError> for DocError {
    fn from(e: loro::LoroError) -> Self {
        DocError::Loro(e.to_string())
    }
}

impl From<loro::LoroEncodeError> for DocError {
    fn from(e: loro::LoroEncodeError) -> Self {
        DocError::Loro(e.to_string())
    }
}

/// Workflow state held by the atomic `lifecycle` register
/// (`spec/data-model.md` "Lifecycle"): a five-step ladder, the first four
/// of which are *Open*. Bin is **not** a workflow state — it is the
/// orthogonal `binned_at` mask.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum WorkflowState {
    Backlog = 0,
    Todo = 1,
    InProgress = 2,
    Review = 3,
    Done = 4,
}

impl WorkflowState {
    /// The register's stored integer code.
    pub fn code(self) -> i64 {
        self as i64
    }

    /// Decode a stored register code. An unrecognized code (a newer
    /// client's state) reads as `None`; the caller degrades to Backlog.
    pub fn from_code(code: i64) -> Option<WorkflowState> {
        match code {
            0 => Some(WorkflowState::Backlog),
            1 => Some(WorkflowState::Todo),
            2 => Some(WorkflowState::InProgress),
            3 => Some(WorkflowState::Review),
            4 => Some(WorkflowState::Done),
            _ => None,
        }
    }

    /// Canonical export / event name (`spec/data-model.md` "Workflow
    /// register — the v2 → v3 break").
    pub fn name(self) -> &'static str {
        match self {
            WorkflowState::Backlog => "backlog",
            WorkflowState::Todo => "todo",
            WorkflowState::InProgress => "in_progress",
            WorkflowState::Review => "review",
            WorkflowState::Done => "done",
        }
    }

    /// Inverse of [`name`](Self::name).
    pub fn parse_name(s: &str) -> Option<WorkflowState> {
        match s {
            "backlog" => Some(WorkflowState::Backlog),
            "todo" => Some(WorkflowState::Todo),
            "in_progress" => Some(WorkflowState::InProgress),
            "review" => Some(WorkflowState::Review),
            "done" => Some(WorkflowState::Done),
            _ => None,
        }
    }

    /// Open = Backlog | Todo | In Progress | Review. The four open states
    /// share each list's single manual order.
    pub fn is_open(self) -> bool {
        self <= WorkflowState::Review
    }
}

/// API-level resolved lifecycle of an item (`spec/data-model.md`): the
/// workflow register's state, or `Binned` while the `binned_at` mask is
/// present. This is the target vocabulary of `set_item_lifecycle`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ItemLifecycle {
    Backlog,
    Todo,
    InProgress,
    Review,
    Done,
    Binned,
}

impl From<WorkflowState> for ItemLifecycle {
    fn from(s: WorkflowState) -> Self {
        match s {
            WorkflowState::Backlog => ItemLifecycle::Backlog,
            WorkflowState::Todo => ItemLifecycle::Todo,
            WorkflowState::InProgress => ItemLifecycle::InProgress,
            WorkflowState::Review => ItemLifecycle::Review,
            WorkflowState::Done => ItemLifecycle::Done,
        }
    }
}

impl ItemLifecycle {
    /// The workflow state this lifecycle names, or `None` for `Binned`
    /// (which is the mask, not a register state).
    pub fn workflow_state(self) -> Option<WorkflowState> {
        match self {
            ItemLifecycle::Backlog => Some(WorkflowState::Backlog),
            ItemLifecycle::Todo => Some(WorkflowState::Todo),
            ItemLifecycle::InProgress => Some(WorkflowState::InProgress),
            ItemLifecycle::Review => Some(WorkflowState::Review),
            ItemLifecycle::Done => Some(WorkflowState::Done),
            ItemLifecycle::Binned => None,
        }
    }
}

/// Stable view of a single item, surfaced to clients (CLI list, fingerprint).
/// `state`/`lifecycle_at` mirror the workflow register (with the
/// absent-register fallback `[Backlog, created_at]` already applied);
/// `binned_at` is the orthogonal bin mask; `started_at`/`done_at` are the
/// reflection stamps. `list_id` is derived from the atomic `location`
/// register.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ItemView {
    pub id: String,
    pub text: String,
    pub notes: String,
    pub list_id: String,
    /// Workflow register state (`spec/data-model.md` "Lifecycle").
    /// Masked by `binned_at` when that is present — see
    /// [`lifecycle`](Self::lifecycle) for the resolved value.
    pub state: WorkflowState,
    /// Unix millis the register's state was entered; `created_at` when
    /// the register is absent.
    pub lifecycle_at: i64,
    /// Optional date-only deadline — a floating local calendar date in
    /// `YYYY-MM-DD` format. `None` ≡ no deadline. The core stores and
    /// echoes the raw string; it never parses it into a timestamp.
    pub deadline: Option<String>,
    /// Optional planned date — `YYYY-MM-DD` (all-day) or
    /// `YYYY-MM-DDTHH:MM` (timed), floating. `None` ≡ unset. Raw string,
    /// never parsed into a timestamp; sorts by plain string compare.
    pub when: Option<String>,
    /// Optional duration in whole minutes, `1..=MAX_DURATION_MINUTES`.
    /// `None` ≡ unset. Only meaningful beside a timed `when`.
    pub duration: Option<u32>,
    pub created_at: i64,
    /// Reflection stamp: first entry into In Progress (write-once).
    pub started_at: Option<i64>,
    /// Reflection stamp: last entry into Done; never cleared. The Done
    /// view sorts by `lifecycle_at`, not this.
    pub done_at: Option<i64>,
    pub binned_at: Option<i64>,
}

impl ItemView {
    /// Workflow register says Done (regardless of the bin mask).
    pub fn is_done(&self) -> bool {
        self.state == WorkflowState::Done
    }
    pub fn is_binned(&self) -> bool {
        self.binned_at.is_some()
    }
    /// Open (visible in a per-list view): one of the four open workflow
    /// states and not binned.
    pub fn is_open(&self) -> bool {
        !self.is_binned() && self.state.is_open()
    }
    /// Resolved lifecycle: `Binned` while the mask is present, else the
    /// workflow register's state.
    pub fn lifecycle(&self) -> ItemLifecycle {
        if self.is_binned() {
            ItemLifecycle::Binned
        } else {
            self.state.into()
        }
    }
}

/// The five board lanes in left-to-right (ladder) order.
const ALL_LANES: [WorkflowState; 5] = [
    WorkflowState::Backlog,
    WorkflowState::Todo,
    WorkflowState::InProgress,
    WorkflowState::Review,
    WorkflowState::Done,
];

/// Which board lanes render (`spec/board.md` "Lane visibility"): a subset
/// of the five lanes, held as a bitmask over [`WorkflowState::code`].
/// Callers keep it non-empty — at least one lane always renders — and
/// [`DefaultView::encode`] treats an empty set as [`LaneSet::ALL`] rather
/// than write an unparseable register.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct LaneSet(u8);

impl LaneSet {
    pub const ALL: LaneSet = LaneSet(0b1_1111);
    pub const NONE: LaneSet = LaneSet(0);

    fn bit(state: WorkflowState) -> u8 {
        1 << state.code()
    }

    pub fn contains(self, state: WorkflowState) -> bool {
        self.0 & Self::bit(state) != 0
    }

    /// This set with `state` shown or hidden.
    pub fn with(self, state: WorkflowState, visible: bool) -> LaneSet {
        if visible {
            LaneSet(self.0 | Self::bit(state))
        } else {
            LaneSet(self.0 & !Self::bit(state))
        }
    }

    pub fn is_empty(self) -> bool {
        self.0 == 0
    }

    /// Visible lanes in ladder order.
    pub fn iter(self) -> impl Iterator<Item = WorkflowState> {
        ALL_LANES.into_iter().filter(move |s| self.contains(*s))
    }
}

impl FromIterator<WorkflowState> for LaneSet {
    fn from_iter<I: IntoIterator<Item = WorkflowState>>(iter: I) -> LaneSet {
        iter.into_iter()
            .fold(LaneSet::NONE, |set, s| set.with(s, true))
    }
}

/// A list's saved default view (`spec/board.md`): which lens a client
/// renders the list in when it has no local override of its own, and —
/// for the board lens — which lanes it shows.
///
/// Stored as a single encoded scalar string so a concurrent save on
/// another device replaces the whole view atomically (same rationale as
/// [`Location`]) rather than merging a mode from one device with a lane
/// set from another:
///
/// ```text
/// "list" | "board" | "board:" lane ("," lane)*
/// lane   = "backlog" | "todo" | "in_progress" | "review" | "done"
/// ```
///
/// The lane list names the *visible* lanes in ladder order; bare
/// `"board"` is the canonical form for all five. `lanes` is only
/// meaningful for the board lens; the list lens always encodes as bare
/// `"list"`. Unrecognized strings — an unknown lens, an unknown lane name,
/// an empty lane list — parse to `None` (treated as "no saved default"),
/// so a future client writing a form this build doesn't know about
/// degrades to the local default instead of rendering something wrong.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DefaultView {
    /// `true` ≡ the board lens, `false` ≡ the flat list view.
    pub board: bool,
    /// Visible board lanes. Always [`LaneSet::ALL`] for the list lens.
    pub lanes: LaneSet,
}

impl DefaultView {
    pub const LIST: DefaultView = DefaultView {
        board: false,
        lanes: LaneSet::ALL,
    };
    pub const BOARD: DefaultView = DefaultView {
        board: true,
        lanes: LaneSet::ALL,
    };

    pub fn encode(&self) -> String {
        if !self.board {
            return "list".to_string();
        }
        if self.lanes == LaneSet::ALL || self.lanes.is_empty() {
            return "board".to_string();
        }
        let names: Vec<&str> = self.lanes.iter().map(WorkflowState::name).collect();
        format!("board:{}", names.join(","))
    }

    pub fn parse(s: &str) -> Option<DefaultView> {
        match s {
            "list" => return Some(DefaultView::LIST),
            "board" => return Some(DefaultView::BOARD),
            _ => {}
        }
        let lanes = s.strip_prefix("board:")?;
        if lanes.is_empty() {
            return None;
        }
        let lanes: LaneSet = lanes
            .split(',')
            .map(WorkflowState::parse_name)
            .collect::<Option<LaneSet>>()?;
        Some(DefaultView { board: true, lanes })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListView {
    pub id: String,
    pub name: String,
    /// The user's chosen display icon (a literal emoji grapheme), or
    /// `None` when unset. Consumers render a built-in fallback for
    /// `None`. Reserved `inbox` (Inbox) has no ListMeta row, so it is
    /// always `None` here.
    pub icon: Option<String>,
    /// The list's saved default view, or `None` when the user has never
    /// saved one. Reserved `inbox` has no ListMeta row — its default
    /// lives on [`SettingsView::inbox_view`].
    pub default_view: Option<DefaultView>,
    /// Archive timestamp (`spec/data-model.md` "Archived lists"):
    /// `None` ≡ active, `Some(ts)` ≡ archived. Pure ListMeta metadata —
    /// items, ordering, and Focus are untouched by archiving.
    pub archived_at: Option<i64>,
    pub created_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SettingsView {
    /// When true, clients render each non-Inbox list's open-item count
    /// (Backlog + Live) in the nav (subject to the count > 0 gate).
    /// Inbox's count is always shown regardless. Single global flag;
    /// default false.
    pub show_list_counts: bool,
    /// The reserved `inbox` (Inbox) list's saved default view. Inbox has
    /// no ListMeta row, so its default lives here rather than on
    /// [`ListView::default_view`]. `None` ≡ no saved default.
    pub inbox_view: Option<DefaultView>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct JsonExport {
    pub version: u32,
    pub settings: ExportSettings,
    pub lists: Vec<ExportList>,
    pub items: Vec<ExportItem>,
    /// Focus lens membership (`spec/focus.md`): item ids in Focus order.
    /// Only visible refs are exported (bare local form — the cross-doc
    /// `doc_id:item_id` form never rides an export today). Skipped when
    /// empty so pre-focus dumps stay byte-identical.
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub focus: Vec<String>,
}

/// Counts surfaced to the UI after a successful `import_json`.
/// `items_skipped` covers entries dropped because their `text` was
/// empty after trim — defensively guarded so a malformed export can't
/// land empty rows in the doc.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportSummary {
    pub lists_added: usize,
    pub items_added: usize,
    pub items_skipped: usize,
    /// Focus refs re-established for imported items (`spec/focus.md`).
    pub focus_added: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportSettings {
    pub show_list_counts: bool,
    /// Inbox's saved default view in its encoded form (`"list"`,
    /// `"board"`, `"board:<lanes>"` — see [`DefaultView`]). Skipped when
    /// unset so pre-default-view dumps stay byte-identical.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub inbox_view: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportList {
    pub id: String,
    pub name: String,
    /// The list's display icon (a literal emoji grapheme). Skipped when
    /// unset so pre-icon dumps stay byte-identical; the reserved Inbox
    /// carries no icon.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub icon: Option<String>,
    /// The list's saved default view in its encoded form (`"list"`,
    /// `"board"`, `"board:<lanes>"` — see [`DefaultView`]). Skipped when
    /// unset so pre-default-view dumps stay byte-identical. The reserved
    /// Inbox entry carries none; its default rides on
    /// [`ExportSettings::inbox_view`].
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub view: Option<String>,
    /// Archive timestamp (`spec/data-model.md` "Archived lists").
    /// Skipped when unset so pre-archive dumps stay byte-identical;
    /// old exports without the field import as active.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub archived_at: Option<i64>,
    pub created_at: Option<i64>,
    pub builtin: bool,
}

/// The workflow register in export form (`spec/data-model.md` "Workflow
/// register"): the state as a name plus the unix millis it was entered.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportLifecycle {
    /// `"backlog" | "todo" | "in_progress" | "review" | "done"`. An
    /// unrecognized name in a hand-edited export degrades to Backlog on
    /// import.
    pub state: String,
    pub at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportItem {
    pub id: String,
    pub text: String,
    pub notes: String,
    pub list_id: String,
    /// Workflow register (`{state, at}`), emitted for every item.
    pub lifecycle: ExportLifecycle,
    /// Date-only deadline (`YYYY-MM-DD`). Skipped when unset to keep
    /// pre-deadline dumps byte-identical.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub deadline: Option<String>,
    /// Planned date (`YYYY-MM-DD` or `YYYY-MM-DDTHH:MM`). Skipped when
    /// unset so older dumps stay byte-identical.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub when: Option<String>,
    /// Duration in whole minutes. Skipped when unset so older dumps stay
    /// byte-identical.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub duration: Option<u32>,
    pub created_at: i64,
    /// Reflection stamp (first entry into In Progress). Skipped when
    /// unset; v2 exports never carry it.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub started_at: Option<i64>,
    /// In a v3 export: the `done_at` reflection stamp. In a v2 export:
    /// done-as-state — mapped to register `[Done, done_at]` on import.
    pub done_at: Option<i64>,
    pub binned_at: Option<i64>,
}

// ---------- location / order-entry encoding ----------

/// Atomic item placement: which list an item is in, and which order
/// entry is the canonical one for it. Encoded as a single scalar string
/// (`"<list_id>:<placement_id>"`) so both halves are written in one
/// register op — no independently-mergeable sub-fields to conflict.
/// `:` is reserved: ids are uuid-v7 hex or the literal `inbox`.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Location {
    list_id: String,
    placement_id: String,
}

impl Location {
    fn encode(&self) -> String {
        format!("{}:{}", self.list_id, self.placement_id)
    }
    fn parse(s: &str) -> Option<Location> {
        let (list_id, placement_id) = s.split_once(':')?;
        if list_id.is_empty() {
            return None;
        }
        Some(Location {
            list_id: list_id.to_string(),
            placement_id: placement_id.to_string(),
        })
    }
}

/// One element of an `order/<list-id>` container:
/// `"<item_id>:<placement_id>"`.
#[derive(Debug, Clone, PartialEq, Eq)]
struct OrderEntry {
    item_id: String,
    placement_id: String,
}

impl OrderEntry {
    fn encode(&self) -> String {
        format!("{}:{}", self.item_id, self.placement_id)
    }
    fn parse(s: &str) -> Option<OrderEntry> {
        let (item_id, placement_id) = s.split_once(':')?;
        if item_id.is_empty() {
            return None;
        }
        Some(OrderEntry {
            item_id: item_id.to_string(),
            placement_id: placement_id.to_string(),
        })
    }
}

fn order_root_name(list_id: &str) -> String {
    format!("{ORDER_PREFIX}{list_id}")
}

/// One element of the reserved `focus` container (`spec/focus.md`).
/// Encoded as a scalar string: bare `"<item_id>"` for a local-doc ref
/// (the only form the emitter writes today), or `"<doc_id>:<item_id>"`
/// for a future cross-doc ref. Split on the **first** `:`; uuid-hex
/// components mean `:` never collides. A colon-less string is a local
/// ref — that is the forward-compat hook for sharing.
#[derive(Debug, Clone, PartialEq, Eq)]
struct FocusRef {
    /// `None` ≡ local doc. `Some(doc_id)` is a foreign ref, unresolvable
    /// today — projection skips it (later: renders a placeholder).
    doc_id: Option<String>,
    item_id: String,
}

impl FocusRef {
    fn local(item_id: &str) -> FocusRef {
        FocusRef {
            doc_id: None,
            item_id: item_id.to_string(),
        }
    }
    fn encode(&self) -> String {
        match &self.doc_id {
            Some(d) => format!("{d}:{}", self.item_id),
            None => self.item_id.clone(),
        }
    }
    fn parse(s: &str) -> Option<FocusRef> {
        if s.is_empty() {
            return None;
        }
        match s.split_once(':') {
            Some((doc_id, item_id)) => {
                if doc_id.is_empty() || item_id.is_empty() {
                    return None;
                }
                Some(FocusRef {
                    doc_id: Some(doc_id.to_string()),
                    item_id: item_id.to_string(),
                })
            }
            None => Some(FocusRef {
                doc_id: None,
                item_id: s.to_string(),
            }),
        }
    }
    fn is_local(&self) -> bool {
        self.doc_id.is_none()
    }
}

/// Classification of the current `focus` container against the item
/// index: which raw slots project to a visible Open item, and which are
/// dead garbage (unparseable, foreign-doc, missing, non-open, or a
/// later duplicate of an already-visible item) to be swept.
struct FocusScan {
    /// Raw encoded elements in container order.
    raw: Vec<String>,
    /// Visible item ids in container order, deduped (first wins).
    visible_item_ids: Vec<String>,
    /// Raw indices that are dead garbage, ascending.
    dead_idx: Vec<usize>,
}

// ---------- disposable projection index ----------

/// Per-item slice of the state the projection needs, mirrored in
/// memory so per-mutation work never touches Loro containers beyond
/// the mutation itself.
#[derive(Debug, Clone, PartialEq, Eq)]
struct ItemMeta {
    list_id: String,
    placement_id: String,
    open: bool,
    created_at: i64,
}

/// Resolution of a target index against a list's projection — the raw
/// container position to write at, and (when exact) the open splice
/// position. See [`ProjectionIndex::plan_target`].
struct TargetPlan {
    raw_pos: usize,
    open_pos: Option<usize>,
}

/// Disposable in-memory mirror of everything ordering-related.
/// Maintained incrementally by local mutations, surgically by remote
/// diff translation, and rebuilt wholesale from the doc on boot or
/// fallback. Never persisted.
#[derive(Default)]
struct ProjectionIndex {
    /// item id → location/lifecycle slice.
    meta: HashMap<String, ItemMeta>,
    /// list id → item ids located there (authoritative membership).
    members: HashMap<String, HashSet<String>>,
    /// list id → positional mirror of `order/<list-id>`. `None` slots
    /// keep unparseable entries position-aligned with the container.
    raw_orders: HashMap<String, Vec<Option<OrderEntry>>>,
    /// list id → open projection (visible entries filtered to open
    /// items, then the deterministic fallback tail). Lists with no open
    /// items carry no key.
    open_by_list: HashMap<String, Vec<String>>,
    /// list id → number of *visible* entries in that list's order
    /// container (any lifecycle). `members(list).len() == visible` ⟺ the
    /// list has no fallback tail — the precondition for the O(open)
    /// splice fast paths; anything tail-adjacent falls back to a full
    /// `refresh_open` walk. Lists with zero visible entries carry no
    /// key.
    visible_counts: HashMap<String, usize>,
}

impl ProjectionIndex {
    /// Visible entry ids of `list_id` in container order (all lifecyclees,
    /// duplicate-guarded), borrowed — plus the visible set for the tail
    /// computation. An entry is visible iff its item exists, its
    /// placement matches the item's authoritative location, and no
    /// earlier entry already claimed the item.
    fn visible_ids<'a>(&'a self, list_id: &str) -> (Vec<&'a str>, HashSet<&'a str>) {
        let mut out = Vec::new();
        let mut seen = HashSet::new();
        for entry in self
            .raw_orders
            .get(list_id)
            .map(Vec::as_slice)
            .unwrap_or(&[])
            .iter()
            .flatten()
        {
            let Some(m) = self.meta.get(&entry.item_id) else {
                continue;
            };
            if m.list_id == list_id
                && m.placement_id == entry.placement_id
                && seen.insert(entry.item_id.as_str())
            {
                out.push(entry.item_id.as_str());
            }
        }
        (out, seen)
    }

    /// Fallback tail: items located in `list_id` with no visible entry,
    /// sorted by `(created_at, id)` so replicas agree.
    fn tail<'a>(&'a self, list_id: &str, seen: &HashSet<&str>) -> Vec<&'a str> {
        let mut tail: Vec<&'a str> = self
            .members
            .get(list_id)
            .into_iter()
            .flatten()
            .filter(|id| !seen.contains(id.as_str()))
            .map(String::as_str)
            .collect();
        tail.sort_by(|a, b| {
            let (ma, mb) = (&self.meta[*a], &self.meta[*b]);
            ma.created_at.cmp(&mb.created_at).then_with(|| a.cmp(b))
        });
        tail
    }

    /// Resolved order of one list — visible entries in container order,
    /// then the fallback tail; all lifecyclees. Pure — reads only the
    /// in-memory mirrors.
    fn resolved(&self, list_id: &str) -> Vec<String> {
        let (mut ids, seen) = self.visible_ids(list_id);
        ids.extend(self.tail(list_id, &seen));
        ids.into_iter().map(str::to_string).collect()
    }

    /// Recompute and store `list_id`'s open projection (and visible
    /// count); returns the projection. One walk covers both; only the
    /// open ids are cloned, so a 13k-lifetime list with 200 open items
    /// allocates 200 strings, not 2×13k.
    fn refresh_open(&mut self, list_id: &str) -> Vec<String> {
        let (open, visible) = {
            let (ids, seen) = self.visible_ids(list_id);
            let visible = ids.len();
            let is_open = |id: &str| self.meta.get(id).is_some_and(|m| m.open);
            let mut open: Vec<String> = ids
                .into_iter()
                .filter(|id| is_open(id))
                .map(str::to_string)
                .collect();
            open.extend(
                self.tail(list_id, &seen)
                    .into_iter()
                    .filter(|id| is_open(id))
                    .map(str::to_string),
            );
            (open, visible)
        };
        if visible == 0 {
            self.visible_counts.remove(list_id);
        } else {
            self.visible_counts.insert(list_id.to_string(), visible);
        }
        if open.is_empty() {
            self.open_by_list.remove(list_id);
        } else {
            self.open_by_list.insert(list_id.to_string(), open.clone());
        }
        open
    }

    /// True when every item located in `list_id` is entry-backed — no
    /// fallback tail exists, so append positions are exact without a
    /// walk.
    fn tail_is_empty(&self, list_id: &str) -> bool {
        let members = self.members.get(list_id).map(HashSet::len).unwrap_or(0);
        let visible = self.visible_counts.get(list_id).copied().unwrap_or(0);
        members == visible
    }

    fn bump_visible(&mut self, list_id: &str, delta: isize) {
        let cur = self.visible_counts.get(list_id).copied().unwrap_or(0) as isize;
        let next = (cur + delta).max(0) as usize;
        if next == 0 {
            self.visible_counts.remove(list_id);
        } else {
            self.visible_counts.insert(list_id.to_string(), next);
        }
    }

    /// Splice `id` into `list_id`'s open projection at `at`.
    fn splice_open_in(&mut self, list_id: &str, id: &str, at: usize) {
        let arr = self.open_by_list.entry(list_id.to_string()).or_default();
        arr.retain(|x| x != id);
        let at = at.min(arr.len());
        arr.insert(at, id.to_string());
    }

    /// Remove `id` from `list_id`'s open projection.
    fn splice_open_out(&mut self, list_id: &str, id: &str) {
        if let Some(arr) = self.open_by_list.get_mut(list_id) {
            arr.retain(|x| x != id);
            if arr.is_empty() {
                self.open_by_list.remove(list_id);
            }
        }
    }

    /// Resolve a caller's target index against `list_id`'s projection
    /// (open projection for open items, full resolved order for hidden
    /// ones), excluding `exclude` from the anchor count when the item
    /// is being re-placed within its own list.
    ///
    /// `open_pos` is the exact open splice position when it can be
    /// derived without a projection walk: `Some(target)` when the
    /// anchor's canonical entry resolved, `Some(usize::MAX)` (append —
    /// the splice clamps) when inserting past the end of a list with no
    /// fallback tail, and `None` when the caller must `refresh_open`
    /// after mutating (tail-adjacent cases). Hidden-item plans always
    /// carry `open_pos: Some(usize::MAX)` — the open array is untouched
    /// by hidden mutations, so no refresh is needed on their account.
    fn plan_target(
        &self,
        list_id: &str,
        target_index: usize,
        is_open: bool,
        exclude: Option<&str>,
    ) -> TargetPlan {
        let raw_len = self.raw_orders.get(list_id).map(Vec::len).unwrap_or(0);
        if !is_open {
            let seq = self.resolved(list_id);
            let anchor_index = match exclude.and_then(|e| seq.iter().position(|x| x == e)) {
                Some(c) if c <= target_index => target_index.saturating_add(1),
                _ => target_index,
            };
            let raw_pos = seq
                .get(anchor_index)
                .and_then(|a| self.canonical_raw_pos(list_id, a))
                .unwrap_or(raw_len);
            return TargetPlan {
                raw_pos,
                open_pos: Some(usize::MAX),
            };
        }
        static EMPTY: Vec<String> = Vec::new();
        let seq = self.open_by_list.get(list_id).unwrap_or(&EMPTY);
        let anchor_index = match exclude.and_then(|e| seq.iter().position(|x| x == e)) {
            Some(c) if c <= target_index => target_index.saturating_add(1),
            _ => target_index,
        };
        match seq.get(anchor_index) {
            Some(anchor) => match self.canonical_raw_pos(list_id, anchor) {
                // Inserting immediately before the anchor puts the item
                // at exactly `target_index` (the exclude-skip above is
                // what makes that hold for same-list re-placement too).
                Some(ap) => TargetPlan {
                    raw_pos: ap,
                    open_pos: Some(target_index),
                },
                // Anchor lives in the fallback tail — position within
                // the open projection needs a walk.
                None => TargetPlan {
                    raw_pos: raw_len,
                    open_pos: None,
                },
            },
            None => TargetPlan {
                raw_pos: raw_len,
                open_pos: if self.tail_is_empty(list_id) {
                    Some(usize::MAX)
                } else {
                    None
                },
            },
        }
    }

    /// Raw container position of the item's canonical (visible) entry.
    fn canonical_raw_pos(&self, list_id: &str, item_id: &str) -> Option<usize> {
        let m = self.meta.get(item_id)?;
        if m.list_id != list_id {
            return None;
        }
        self.raw_orders.get(list_id)?.iter().position(|e| {
            e.as_ref()
                .is_some_and(|e| e.item_id == item_id && e.placement_id == m.placement_id)
        })
    }

    /// Every raw position holding an entry for `item_id` in `list_id`
    /// (canonical, stale, and duplicates alike), ascending.
    fn entry_positions(&self, list_id: &str, item_id: &str) -> Vec<usize> {
        self.raw_orders
            .get(list_id)
            .map(Vec::as_slice)
            .unwrap_or(&[])
            .iter()
            .enumerate()
            .filter_map(|(i, e)| {
                e.as_ref()
                    .is_some_and(|e| e.item_id == item_id)
                    .then_some(i)
            })
            .collect()
    }

    fn set_meta(&mut self, id: &str, meta: ItemMeta) {
        if let Some(old) = self.meta.get(id)
            && old.list_id != meta.list_id
            && let Some(s) = self.members.get_mut(&old.list_id)
        {
            s.remove(id);
            if s.is_empty() {
                self.members.remove(&old.list_id);
            }
        }
        self.members
            .entry(meta.list_id.clone())
            .or_default()
            .insert(id.to_string());
        self.meta.insert(id.to_string(), meta);
    }

    fn remove_item(&mut self, id: &str) {
        if let Some(old) = self.meta.remove(id)
            && let Some(s) = self.members.get_mut(&old.list_id)
        {
            s.remove(id);
            if s.is_empty() {
                self.members.remove(&old.list_id);
            }
        }
    }
}

// ---------- gated diff capture (remote import / undo translation) ----------

/// Owned copy of one Loro container diff captured while an import or
/// undo/redo operation is in progress. Ordinary local mutations emit
/// `AppEvent`s directly and are never captured.
enum CapturedDiff {
    /// Root `items` map changed: item containers appeared / vanished.
    ItemsRoot {
        upserted: Vec<String>,
        removed: Vec<String>,
    },
    /// One item's map changed; which keys were touched.
    ItemMap {
        container: ContainerID,
        keys: Vec<String>,
    },
    /// A text child of an item map changed (`notes`). Carries the Loro
    /// delta (Unicode-scalar units) so a subscribed editor can receive
    /// it as an `ItemNotesDelta`; the key is also treated as dirty on
    /// the item map so the whole-string event still fires.
    ItemText {
        container: ContainerID,
        key: String,
        delta: Vec<TextDelta>,
    },
    /// One `order/<list-id>` container changed (insert/delete/move).
    Order {
        list_id: String,
        ops: Vec<CapturedListItem>,
    },
    /// Root `lists` MovableList or one of its list maps changed. Lists
    /// are few, so translation just re-diffs them wholesale.
    Lists,
    /// Doc-level settings map changed.
    Settings,
    /// The `focus` container changed (`spec/focus.md`). Consumers
    /// re-derive the Focus projection; no positional translation needed.
    Focus,
    /// A diff shape we don't translate — forces the full-resync
    /// fallback for the frame.
    Opaque,
}

enum CapturedListItem {
    Retain(usize),
    Delete(usize),
    /// Inserted (or move-target) scalar entries. `None` for a
    /// non-string value, which we never produce — it lands as an
    /// unparseable (invisible) slot rather than aborting the frame.
    Insert(Vec<Option<String>>),
}

#[derive(Clone, Copy, Default, PartialEq, Eq)]
enum DiffCaptureMode {
    #[default]
    None,
    Import,
    Undo,
}

#[derive(Default)]
struct DiffCapture {
    mode: DiffCaptureMode,
    diffs: Vec<CapturedDiff>,
}

fn classify_captured_diff(
    target: &ContainerID,
    path: &[(ContainerID, Index)],
    diff: &LoroDiff,
) -> CapturedDiff {
    let root_name = |cid: &ContainerID| match cid {
        ContainerID::Root { name, .. } => Some(name.to_string()),
        ContainerID::Normal { .. } => None,
    };
    // A text child of an item map (`notes`). Loro realises a mergeable
    // child as a *root* container with a derived name, so the target
    // never says which item it belongs to; the event path does. Each
    // entry pairs a container with its index in its parent, so the
    // shape is `[(items, _), (item map, Key(id)), (text, Key(key))]`.
    // Translate to the item map dirty at that key; consumers re-read
    // the whole string from the view.
    if let LoroDiff::Text(delta) = diff {
        return match path {
            [(root, _), (item_map, _), (_, Index::Key(key))]
                if root_name(root).as_deref() == Some(ROOT_ITEMS) =>
            {
                CapturedDiff::ItemText {
                    container: item_map.clone(),
                    key: key.to_string(),
                    delta: delta.clone(),
                }
            }
            _ => CapturedDiff::Opaque,
        };
    }
    let path_root = path.first().map(|(cid, _)| cid);
    match root_name(target) {
        Some(name) if name == ROOT_ITEMS => {
            let LoroDiff::Map(m) = diff else {
                return CapturedDiff::Opaque;
            };
            let mut upserted = Vec::new();
            let mut removed = Vec::new();
            for (k, v) in m.updated.iter() {
                match v {
                    Some(_) => upserted.push(k.to_string()),
                    None => removed.push(k.to_string()),
                }
            }
            CapturedDiff::ItemsRoot { upserted, removed }
        }
        Some(name) if name == ROOT_LISTS => CapturedDiff::Lists,
        Some(name) if name == ROOT_SETTINGS => CapturedDiff::Settings,
        Some(name) if name == FOCUS_CONTAINER => CapturedDiff::Focus,
        Some(name) => {
            let Some(list_id) = name.strip_prefix(ORDER_PREFIX) else {
                return CapturedDiff::Opaque;
            };
            let LoroDiff::List(items) = diff else {
                return CapturedDiff::Opaque;
            };
            CapturedDiff::Order {
                list_id: list_id.to_string(),
                ops: items
                    .iter()
                    .map(|it| match it {
                        ListDiffItem::Retain { retain } => CapturedListItem::Retain(*retain),
                        ListDiffItem::Delete { delete } => CapturedListItem::Delete(*delete),
                        ListDiffItem::Insert { insert, .. } => CapturedListItem::Insert(
                            insert
                                .iter()
                                .map(|v| match v {
                                    ValueOrContainer::Value(LoroValue::String(s)) => {
                                        Some(s.to_string())
                                    }
                                    _ => None,
                                })
                                .collect(),
                        ),
                    })
                    .collect(),
            }
        }
        None => {
            // Nested container: an item map or a list map. Route by the
            // root container at the head of its path.
            let Some(root) = path_root.and_then(root_name) else {
                return CapturedDiff::Opaque;
            };
            if root == ROOT_LISTS {
                return CapturedDiff::Lists;
            }
            if root != ROOT_ITEMS {
                return CapturedDiff::Opaque;
            }
            let LoroDiff::Map(m) = diff else {
                return CapturedDiff::Opaque;
            };
            CapturedDiff::ItemMap {
                container: target.clone(),
                keys: m.updated.keys().map(|k| k.to_string()).collect(),
            }
        }
    }
}

fn make_diff_subscriber(
    capture: Arc<Mutex<DiffCapture>>,
) -> Arc<dyn for<'a> Fn(DiffEvent<'a>) + Send + Sync> {
    Arc::new(move |e: DiffEvent<'_>| {
        // NOTE: Loro invokes subscribers re-entrantly from inside
        // import/commit — do not touch the doc here, only stash owned
        // data.
        let Ok(mut capture) = capture.lock() else {
            return;
        };
        let should_capture = matches!(
            (capture.mode, e.triggered_by),
            (DiffCaptureMode::Import, EventTriggerKind::Import)
                | (DiffCaptureMode::Undo, EventTriggerKind::Local)
        );
        if !should_capture {
            return;
        }
        for cd in &e.events {
            capture
                .diffs
                .push(classify_captured_diff(cd.target, cd.path, &cd.diff));
        }
    })
}

pub struct Doc {
    inner: LoroDoc,
    last_persisted_vv: VersionVector,
    /// Domain-level change events. Mutation methods push directly;
    /// `apply_remote` does diff translation and pushes a batch. Drain
    /// via `pop_event` / `drain_events`. Wrapped in `Mutex` so mutation
    /// methods can stay `&self` (Loro's interior-mutability shape).
    events: Mutex<VecDeque<AppEvent>>,
    /// Per-session undo/redo. Bound to the local peer at construction;
    /// only records local commits. Remote ops imported by
    /// `apply_remote` carry origin `"remote"` and are filtered out by
    /// prefix — see `spec/sync-protocol.md` "Commit origin tagging".
    undo: Mutex<UndoManager>,
    /// Disposable projection index — see [`ProjectionIndex`]. Rebuilt
    /// from `inner` after boot replay / fallback and maintained
    /// incrementally otherwise.
    item_index: Mutex<ProjectionIndex>,
    /// Gated Loro diff capture used by remote import and undo/redo. The
    /// gate prevents ordinary local mutations from accumulating diffs;
    /// callers enable it only around the operation they will translate.
    diff_capture: Arc<Mutex<DiffCapture>>,
    /// Pre-change text of every item with a subscribed notes editor
    /// (`subscribe_notes`). Loro's text diffs count Unicode scalars and
    /// a delete carries only a count, so converting a remote delta to
    /// UTF-16 needs the text as it was before the change; this is it.
    /// Updated on every local delta and every translated remote delta.
    notes_shadows: Mutex<HashMap<String, String>>,
    /// Root diff subscription feeding `diff_capture`. Dropping it
    /// unsubscribes, so it lives exactly as long as the doc.
    _diff_sub: Subscription,
}

/// Configure an UndoManager bound to `inner`, excluding remote-tagged
/// commits. Construct *after* any seeding/snapshot import so those
/// operations aren't eligible for undo.
fn make_undo_manager(inner: &LoroDoc) -> UndoManager {
    let mut um = UndoManager::new(inner);
    um.add_exclude_origin_prefix("remote");
    um.add_exclude_origin_prefix(NOTES_ORIGIN_PREFIX);
    um
}

impl Doc {
    /// New doc with built-in state initialised. There are no persisted
    /// user-list seeds; only the virtual built-in `inbox` exists at
    /// first open. Device-2 bootstrap via snapshot bypasses this path
    /// entirely.
    pub fn new() -> Result<Self, DocError> {
        Self::new_inner(LoroDoc::new())
    }

    /// As [`new`](Self::new), but with an explicit Loro peer id instead
    /// of the default random one. The peer is set before builtin
    /// seeding (so any seeded op would carry it) and before the
    /// `UndoManager` binds to the local peer. The caller must guarantee
    /// single ownership of `peer` across live docs — see
    /// `spec/peer-id-plan.md`. `u64::MAX` is reserved by Loro.
    pub fn new_with_peer(peer: u64) -> Result<Self, DocError> {
        let inner = LoroDoc::new();
        inner.set_peer_id(peer)?;
        Self::new_inner(inner)
    }

    fn new_inner(inner: LoroDoc) -> Result<Self, DocError> {
        if seed_builtins(&inner)? {
            inner.commit();
        }
        let undo = Mutex::new(make_undo_manager(&inner));
        let item_index = Mutex::new(ProjectionIndex::default());
        let diff_capture = Arc::new(Mutex::new(DiffCapture::default()));
        let _diff_sub = inner.subscribe_root(make_diff_subscriber(diff_capture.clone()));
        Ok(Self {
            inner,
            last_persisted_vv: VersionVector::default(),
            events: Mutex::new(VecDeque::new()),
            undo,
            item_index,
            diff_capture,
            notes_shadows: Mutex::new(HashMap::new()),
            _diff_sub,
        })
    }

    /// Empty doc — used by device 2 before snapshot import.
    pub fn empty() -> Self {
        Self::empty_inner(LoroDoc::new())
    }

    /// As [`empty`](Self::empty), but with an explicit Loro peer id —
    /// same contract as [`new_with_peer`](Self::new_with_peer). Used by
    /// `boot_doc` so replayed history and subsequent local commits share
    /// the device's leased peer.
    pub fn empty_with_peer(peer: u64) -> Result<Self, DocError> {
        let inner = LoroDoc::new();
        inner.set_peer_id(peer)?;
        Ok(Self::empty_inner(inner))
    }

    fn empty_inner(inner: LoroDoc) -> Self {
        let undo = Mutex::new(make_undo_manager(&inner));
        let item_index = Mutex::new(ProjectionIndex::default());
        let diff_capture = Arc::new(Mutex::new(DiffCapture::default()));
        let _diff_sub = inner.subscribe_root(make_diff_subscriber(diff_capture.clone()));
        Self {
            last_persisted_vv: inner.oplog_vv(),
            inner,
            events: Mutex::new(VecDeque::new()),
            undo,
            item_index,
            diff_capture,
            notes_shadows: Mutex::new(HashMap::new()),
            _diff_sub,
        }
    }

    /// Oplog counter end of the local peer — how many ops this peer id
    /// has ever committed, imported history included.
    fn local_peer_counter(&self) -> i32 {
        self.inner
            .oplog_vv()
            .get(&self.inner.peer_id())
            .copied()
            .unwrap_or(0)
    }

    /// Re-bind the per-session UndoManager at the current oplog frontier.
    ///
    /// Loro's UndoManager advances its internal "next counter" only on
    /// Local-triggered events; an *import* that carries ops for the
    /// local peer (boot replay under a stable leased peer id, or a
    /// snapshot minted under a reused peer slot — see
    /// `spec/peer-id-plan.md`) leaves it stale, so the next local commit
    /// would record one undo span stretching back through the imported
    /// history. Recreating the manager clears both stacks, which is
    /// sound: same-peer ops arriving by import were never produced by
    /// this session and must not be undoable here.
    fn rearm_undo(&self) {
        let mut um = self.undo.lock().expect("undo mutex poisoned");
        *um = make_undo_manager(&self.inner);
    }

    fn begin_diff_capture(&self, mode: DiffCaptureMode) {
        let mut capture = self.diff_capture.lock().expect("diff capture poisoned");
        capture.diffs.clear();
        capture.mode = mode;
    }

    fn finish_diff_capture(&self) -> Vec<CapturedDiff> {
        let mut capture = self.diff_capture.lock().expect("diff capture poisoned");
        capture.mode = DiffCaptureMode::None;
        std::mem::take(&mut capture.diffs)
    }

    /// Recompute the whole [`ProjectionIndex`] from the doc. O(items +
    /// order entries); used on boot, after bulk operations, and by the
    /// translation fallback.
    fn rebuild_index(&self) {
        *self.item_index.lock().expect("item index mutex poisoned") = self.compute_index();
    }

    /// Build a fresh [`ProjectionIndex`] straight from the Loro
    /// containers, without installing it. Shared by `rebuild_index` and
    /// the test-side "does the incremental index match the doc" check.
    fn compute_index(&self) -> ProjectionIndex {
        let items = self.items();
        let mut idx = ProjectionIndex::default();
        let keys: Vec<String> = items.keys().map(|k| k.to_string()).collect();
        for id in keys {
            let Some(map) = item_map_of(&items, &id) else {
                continue;
            };
            let meta = item_meta(&map);
            idx.members
                .entry(meta.list_id.clone())
                .or_default()
                .insert(id.clone());
            idx.meta.insert(id, meta);
        }
        let mut candidates: HashSet<String> = idx.members.keys().cloned().collect();
        candidates.insert(LIST_INBOX.to_string());
        for list in self.all_lists() {
            candidates.insert(list.id);
        }
        for list_id in &candidates {
            let order = self.order_list(list_id);
            let mut raw = Vec::with_capacity(order.len());
            for i in 0..order.len() {
                raw.push(scalar_entry_at(&order, i));
            }
            idx.raw_orders.insert(list_id.clone(), raw);
        }
        for list_id in &candidates {
            idx.refresh_open(list_id);
        }
        idx
    }

    pub fn last_persisted_vv(&self) -> &VersionVector {
        &self.last_persisted_vv
    }

    /// Snapshot of the oplog VV — every commit currently in the log,
    /// across every peer we've seen. The engine captures this at the
    /// moment of an export and feeds it back via `mark_persisted_at`
    /// once the exported bytes are durably in the WAL, so a mutation
    /// committed *during* the append isn't silently skipped.
    pub fn oplog_vv(&self) -> VersionVector {
        self.inner.oplog_vv()
    }

    /// The Loro peer id local commits are minted under.
    pub fn peer_id(&self) -> u64 {
        self.inner.peer_id()
    }

    /// True iff there are commits not yet captured into the local WAL.
    pub fn has_uncaptured_ops(&self) -> bool {
        // `Updates { from: oplog_vv }` is empty; `Updates { from: VV<oplog }` isn't.
        self.inner.oplog_vv() != self.last_persisted_vv
    }

    // ---------- mutations: items ----------

    pub fn add_item(&self, list_id: &str, text: &str) -> Result<String, DocError> {
        self.add_item_at(list_id, text, usize::MAX)
    }

    /// Insert a new item as the `target_index`-th open entry of
    /// `list_id`, in a single Loro commit. `target_index` past the end
    /// of the visible open items appends.
    pub fn add_item_at(
        &self,
        list_id: &str,
        text: &str,
        target_index: usize,
    ) -> Result<String, DocError> {
        let ids = self.add_items_at(list_id, &[text], target_index)?;
        Ok(ids.into_iter().next().expect("one text yields one id"))
    }

    /// Bulk-insert `texts` as a contiguous run of open items starting
    /// at the `target_index`-th visible position of `list_id`. All
    /// inserts land in a single Loro commit (one outbound op group).
    /// Validation is upfront: any empty-after-trim entry rejects the
    /// whole batch so callers don't see partial state.
    pub fn add_items_at(
        &self,
        list_id: &str,
        texts: &[&str],
        target_index: usize,
    ) -> Result<Vec<String>, DocError> {
        self.add_items_at_impl(list_id, texts, target_index, WorkflowState::Backlog)
    }

    /// Board open-lane quick-capture: append a new item directly in the
    /// open workflow state `state`, one commit (`spec/board.md`
    /// "Capture"). Rejects `Done` — the Done lane logs completions via
    /// create-then-transition, not direct capture.
    pub fn add_item_in_state(
        &self,
        list_id: &str,
        text: &str,
        state: WorkflowState,
    ) -> Result<String, DocError> {
        self.add_item_in_state_at(list_id, text, state, usize::MAX)
    }

    /// Board open-lane quick-capture at a position: insert a new item in
    /// state `state` as the `target_index`-th open entry of `list_id`,
    /// one commit. `target_index` addresses the list's Open projection —
    /// the same index space as `add_item_at`.
    pub fn add_item_in_state_at(
        &self,
        list_id: &str,
        text: &str,
        state: WorkflowState,
        target_index: usize,
    ) -> Result<String, DocError> {
        if !state.is_open() {
            return Err(DocError::Invalid(
                "can only capture directly into an open workflow state".into(),
            ));
        }
        let ids = self.add_items_at_impl(list_id, &[text], target_index, state)?;
        Ok(ids.into_iter().next().expect("one text yields one id"))
    }

    fn add_items_at_impl(
        &self,
        list_id: &str,
        texts: &[&str],
        target_index: usize,
        state: WorkflowState,
    ) -> Result<Vec<String>, DocError> {
        let trimmed: Vec<&str> = texts.iter().map(|t| t.trim()).collect();
        if trimmed.iter().any(|t| t.is_empty()) {
            return Err(DocError::Invalid("item text is empty".into()));
        }
        self.assert_list_exists(list_id)?;
        if trimmed.is_empty() {
            return Ok(Vec::new());
        }
        let plan = self.plan_target(list_id, target_index, true, None);
        let raw_pos = plan.raw_pos;
        let items = self.items();
        let order = self.order_list(list_id);
        let mut ids = Vec::with_capacity(trimmed.len());
        let mut pending: Vec<(String, String, String, i64)> = Vec::with_capacity(trimmed.len());
        for (i, text) in trimmed.iter().enumerate() {
            let id = new_id();
            let placement = new_id();
            let now = now_millis();
            let map = items.insert_container(&id, LoroMap::new())?;
            map.insert(KEY_ID, id.as_str())?;
            map.insert(KEY_TEXT, *text)?;
            map.insert(KEY_CREATED_AT, now)?;
            map.insert(
                KEY_LOCATION,
                Location {
                    list_id: list_id.to_string(),
                    placement_id: placement.clone(),
                }
                .encode()
                .as_str(),
            )?;
            if state != WorkflowState::Backlog {
                // Direct capture into a non-default open lane: write the
                // register (and the started_at stamp when the lane is
                // In Progress) in the same commit (`spec/board.md`).
                write_workflow(&map, state, now)?;
                if state == WorkflowState::InProgress {
                    map.insert(KEY_STARTED_AT, now)?;
                }
            }
            let entry = OrderEntry {
                item_id: id.clone(),
                placement_id: placement.clone(),
            }
            .encode();
            let at = raw_pos + i;
            if at >= order.len() {
                order.push(entry.as_str())?;
            } else {
                order.insert(at, entry.as_str())?;
            }
            pending.push((id.clone(), placement, (*text).to_string(), now));
            ids.push(id);
        }
        self.inner.commit();
        let open = {
            let mut guard = self.item_index.lock().expect("item index mutex poisoned");
            for (i, (id, placement, _, now)) in pending.iter().enumerate() {
                guard.set_meta(
                    id,
                    ItemMeta {
                        list_id: list_id.to_string(),
                        placement_id: placement.clone(),
                        open: true,
                        created_at: *now,
                    },
                );
                let raw = guard.raw_orders.entry(list_id.to_string()).or_default();
                let at = (raw_pos + i).min(raw.len());
                raw.insert(
                    at,
                    Some(OrderEntry {
                        item_id: id.clone(),
                        placement_id: placement.clone(),
                    }),
                );
                guard.bump_visible(list_id, 1);
            }
            match plan.open_pos {
                Some(pos) => {
                    for (i, (id, ..)) in pending.iter().enumerate() {
                        guard.splice_open_in(list_id, id, pos.saturating_add(i));
                    }
                    guard.open_by_list.get(list_id).cloned().unwrap_or_default()
                }
                None => guard.refresh_open(list_id),
            }
        };
        for (id, _, text, now) in pending {
            let open_index = open.iter().position(|x| x == &id);
            self.push_event(AppEvent::ItemAdded {
                id,
                list_id: list_id.to_string(),
                text,
                notes: String::new(),
                created_at: now,
                state,
                lifecycle_at: now,
                started_at: (state == WorkflowState::InProgress).then_some(now),
                done_at: None,
                binned_at: None,
                deadline: None,
                when: None,
                duration: None,
                open_index,
            });
        }
        Ok(ids)
    }

    pub fn edit_item_text(&self, item_id: &str, text: &str) -> Result<(), DocError> {
        let text = text.trim();
        if text.is_empty() {
            return Err(DocError::Invalid("item text is empty".into()));
        }
        let map = self.find_item(item_id)?;
        map.insert(KEY_TEXT, text)?;
        self.inner.commit();
        self.push_event(AppEvent::ItemTextChanged {
            id: item_id.to_string(),
            text: text.to_string(),
        });
        Ok(())
    }

    /// Set an item's free-form notes. Empty is allowed (clears the
    /// note); leading/trailing whitespace is preserved verbatim because
    /// notes are intentionally a freeform plain-text field.
    ///
    /// Notes live in a mergeable `LoroText` child of the item map
    /// (`spec/notes-plan.md`), created on the first write. The new string
    /// is applied as a character diff against the current text, so
    /// concurrent edits from two devices merge instead of one overwriting
    /// the other. Clearing deletes the text's content and keeps the map
    /// key: deleting the key would hide the child, and a later
    /// `ensure_mergeable_text` would resurface the old content.
    pub fn edit_item_notes(&self, item_id: &str, notes: &str) -> Result<(), DocError> {
        let map = self.find_item(item_id)?;
        let text = map.ensure_mergeable_text(KEY_NOTES)?;
        if text.to_string() == notes {
            return Ok(());
        }
        update_text(&text, notes)?;
        self.inner.commit();
        self.refresh_notes_shadow(item_id, notes);
        self.push_event(AppEvent::ItemNotesChanged {
            id: item_id.to_string(),
            notes: notes.to_string(),
        });
        Ok(())
    }

    /// Start streaming an item's notes as deltas. Returns the current
    /// plain text, which the caller's editor loads; from here on every
    /// remote (or other-tab, or undo) change to this item's notes is
    /// emitted as an `ItemNotesDelta` in UTF-16 units alongside the
    /// whole-string `ItemNotesChanged`. Call `unsubscribe_notes` when the
    /// editor closes. Subscribing again re-syncs (returns the text and
    /// resets the shadow), which is the recovery step after `FullResync`.
    pub fn subscribe_notes(&self, item_id: &str) -> Result<String, DocError> {
        let map = self.find_item(item_id)?;
        let notes = read_text(&map, KEY_NOTES).unwrap_or_default();
        self.notes_shadows
            .lock()
            .expect("notes shadows mutex poisoned")
            .insert(item_id.to_string(), notes.clone());
        Ok(notes)
    }

    /// Stop streaming an item's notes. Unknown ids are ignored.
    pub fn unsubscribe_notes(&self, item_id: &str) {
        self.notes_shadows
            .lock()
            .expect("notes shadows mutex poisoned")
            .remove(item_id);
    }

    /// Apply an editor delta (UTF-16 units) to an item's notes. One
    /// commit, origin `notes:<item id>`, excluded from workspace undo.
    /// The whole delta is validated against the current text before
    /// anything is written: an out-of-range retain / delete, or a
    /// position that would split a surrogate pair, rejects with
    /// `Invalid` and leaves the doc untouched. Emits `ItemNotesChanged`
    /// with the resulting plain text; no `ItemNotesDelta` (the caller
    /// already has the delta).
    pub fn apply_notes_delta(&self, item_id: &str, delta: &[NotesDeltaOp]) -> Result<(), DocError> {
        let map = self.find_item(item_id)?;
        let text = map.ensure_mergeable_text(KEY_NOTES)?;
        let current = text.to_string();
        validate_utf16_delta(&current, delta)?;
        let mut pos = 0usize;
        for op in delta {
            match op {
                NotesDeltaOp::Retain { retain } => pos += retain,
                NotesDeltaOp::Delete { delete } => text.delete_utf16(pos, *delete)?,
                NotesDeltaOp::Insert { insert } => {
                    text.insert_utf16(pos, insert)?;
                    pos += insert.encode_utf16().count();
                }
            }
        }
        self.inner
            .commit_with(CommitOptions::new().origin(&format!("{NOTES_ORIGIN_PREFIX}{item_id}")));
        let notes = text.to_string();
        self.refresh_notes_shadow(item_id, &notes);
        self.push_event(AppEvent::ItemNotesChanged {
            id: item_id.to_string(),
            notes,
        });
        Ok(())
    }

    /// View-diff paths (`emit_item_diffs` / `emit_state_diff`) know only
    /// the whole strings; give subscribed editors a replace delta for
    /// each `ItemNotesChanged` they produced.
    fn emit_notes_deltas_for(&self, emitted: &[AppEvent]) {
        for ev in emitted {
            if let AppEvent::ItemNotesChanged { id, notes } = ev {
                self.emit_notes_delta(id, &[], notes);
            }
        }
    }

    /// Point a subscribed item's shadow at `notes`; no-op when the item
    /// is not subscribed.
    fn refresh_notes_shadow(&self, item_id: &str, notes: &str) {
        let mut shadows = self
            .notes_shadows
            .lock()
            .expect("notes shadows mutex poisoned");
        if let Some(shadow) = shadows.get_mut(item_id) {
            *shadow = notes.to_string();
        }
    }

    /// Emit `ItemNotesDelta` for a subscribed item after a translated
    /// change. `deltas` are the captured Loro text diffs for this frame
    /// in order (scalar units); an empty slice means the change was
    /// wholesale (a container re-set), so the delta is a full replace.
    /// If a delta does not fit the shadow, the shadow was out of step
    /// and a full replace is emitted instead; either way the shadow
    /// ends equal to `post`.
    fn emit_notes_delta(&self, item_id: &str, deltas: &[Vec<TextDelta>], post: &str) {
        let mut shadows = self
            .notes_shadows
            .lock()
            .expect("notes shadows mutex poisoned");
        let Some(shadow) = shadows.get_mut(item_id) else {
            return;
        };
        let mut out: Vec<NotesDeltaOp> = Vec::new();
        let mut cur = shadow.clone();
        let mut ok = !deltas.is_empty();
        if ok {
            for d in deltas {
                match utf16_delta_from_scalar(&cur, d) {
                    Some((ops, next)) => {
                        out = compose_utf16_deltas(&out, &ops);
                        cur = next;
                    }
                    None => {
                        ok = false;
                        break;
                    }
                }
            }
            if ok && cur != post {
                ok = false;
            }
        }
        if !ok {
            out = replace_delta(shadow, post);
        }
        *shadow = post.to_string();
        drop(shadows);
        if !out.is_empty() {
            self.push_event(AppEvent::ItemNotesDelta {
                id: item_id.to_string(),
                delta: out,
            });
        }
    }

    /// Set or clear an item's date-only deadline. `Some(date)` validates
    /// a `YYYY-MM-DD` calendar date and writes the `deadline` register;
    /// `None` deletes the key. One Loro commit. Malformed dates are
    /// rejected with `Invalid` and never touch the doc. The stored value
    /// is a floating local calendar date — the core never converts it to
    /// a timestamp.
    pub fn set_item_deadline(&self, item_id: &str, deadline: Option<&str>) -> Result<(), DocError> {
        let normalized = match deadline {
            Some(raw) => Some(parse_deadline(raw)?),
            None => None,
        };
        let map = self.find_item(item_id)?;
        match &normalized {
            Some(date) => map.insert(KEY_DEADLINE, date.as_str())?,
            None => {
                let _ = map.delete(KEY_DEADLINE);
            }
        }
        self.inner.commit();
        self.push_event(AppEvent::ItemDeadlineChanged {
            id: item_id.to_string(),
            deadline: normalized,
        });
        Ok(())
    }

    /// Set or clear an item's planned date. `Some(value)` validates a
    /// floating `YYYY-MM-DD` (all-day) or `YYYY-MM-DDTHH:MM` (timed)
    /// value and writes the `when` register; `None` deletes the key. One
    /// Loro commit. Malformed values — seconds, offsets, a bracketed
    /// zone suffix — are rejected with `Invalid` and never touch the doc.
    /// Mirrors `set_item_deadline`. Clearing the `when` also clears any
    /// `duration` in the same commit: a length without a start is
    /// meaningless, and leaving it would resurface on the next date set.
    /// Going timed → all-day keeps it, so re-adding a time restores the
    /// end.
    pub fn set_item_when(&self, item_id: &str, when: Option<&str>) -> Result<(), DocError> {
        let normalized = match when {
            Some(raw) => Some(parse_when(raw)?),
            None => None,
        };
        let map = self.find_item(item_id)?;
        let mut duration_cleared = false;
        match &normalized {
            Some(value) => map.insert(KEY_WHEN, value.as_str())?,
            None => {
                let _ = map.delete(KEY_WHEN);
                if read_duration(&map).is_some() {
                    let _ = map.delete(KEY_DURATION);
                    duration_cleared = true;
                }
            }
        }
        self.inner.commit();
        self.push_event(AppEvent::ItemWhenChanged {
            id: item_id.to_string(),
            when: normalized,
        });
        if duration_cleared {
            self.push_event(AppEvent::ItemDurationChanged {
                id: item_id.to_string(),
                duration: None,
            });
        }
        Ok(())
    }

    /// Set or clear an item's duration in whole minutes. `Some(n)` must
    /// be in `1..=MAX_DURATION_MINUTES` or the call rejects with
    /// `Invalid`; `None` deletes the key. One Loro commit. Independent of
    /// `when` at the register level (no cross-field check, so concurrent
    /// edits can never leave the doc invalid); a duration beside an
    /// all-day or absent `when` is simply ignored by views.
    pub fn set_item_duration(&self, item_id: &str, duration: Option<u32>) -> Result<(), DocError> {
        if let Some(n) = duration
            && (n == 0 || n > MAX_DURATION_MINUTES)
        {
            return Err(DocError::Invalid(format!(
                "duration must be 1..={MAX_DURATION_MINUTES} minutes: {n}"
            )));
        }
        let map = self.find_item(item_id)?;
        match duration {
            Some(n) => map.insert(KEY_DURATION, i64::from(n))?,
            None => {
                let _ = map.delete(KEY_DURATION);
            }
        }
        self.inner.commit();
        self.push_event(AppEvent::ItemDurationChanged {
            id: item_id.to_string(),
            duration,
        });
        Ok(())
    }

    /// Move an item. Same-list: a `mov` on the list's order container
    /// (placement preserved). Cross-list: fresh placement, one atomic
    /// `location` write, entry insert in the target order, best-effort
    /// entry removal from the source order — all in one commit (one
    /// undo step). `target_index` addresses the open projection for
    /// open items and the full resolved order for done/binned items.
    pub fn move_item(
        &self,
        item_id: &str,
        target_list_id: &str,
        target_index: usize,
    ) -> Result<(), DocError> {
        self.assert_list_exists(target_list_id)?;
        let map = self.find_item(item_id)?;
        let (cur_list, is_open) = {
            let guard = self.item_index.lock().expect("item index mutex poisoned");
            let m = guard
                .meta
                .get(item_id)
                .ok_or_else(|| DocError::ItemNotFound(item_id.to_string()))?;
            (m.list_id.clone(), m.open)
        };
        if cur_list == target_list_id {
            self.reorder_in_list(item_id, &map, target_list_id, target_index, is_open)
        } else {
            self.move_across_lists(
                item_id,
                &map,
                &cur_list,
                target_list_id,
                target_index,
                is_open,
            )
        }
    }

    fn reorder_in_list(
        &self,
        item_id: &str,
        map: &LoroMap,
        list_id: &str,
        target_index: usize,
        is_open: bool,
    ) -> Result<(), DocError> {
        let (from, to, open_plan) = {
            let guard = self.item_index.lock().expect("item index mutex poisoned");
            let Some(from) = guard.canonical_raw_pos(list_id, item_id) else {
                // Fallback-tail item (no visible entry): re-place it
                // with a fresh entry instead of a mov.
                drop(guard);
                return self.replace_entry(item_id, map, list_id, target_index, is_open);
            };
            // Anchor over the projection the caller's index speaks
            // about (open for open items, full resolved order for
            // hidden ones), excluding the moving item.
            let plan = guard.plan_target(list_id, target_index, is_open, Some(item_id));
            // `raw_pos` is an insert-before position; a `mov` of an
            // earlier element shifts everything after it left by one.
            let to = if plan.raw_pos > from {
                plan.raw_pos.saturating_sub(1)
            } else {
                plan.raw_pos
            };
            (from, to, plan.open_pos)
        };
        if from == to {
            return Ok(());
        }
        self.order_list(list_id).mov(from, to)?;
        self.inner.commit();
        let open_index = {
            let mut guard = self.item_index.lock().expect("item index mutex poisoned");
            if let Some(raw) = guard.raw_orders.get_mut(list_id) {
                let e = raw.remove(from);
                raw.insert(to.min(raw.len()), e);
            }
            if !is_open {
                None
            } else {
                match open_plan {
                    Some(pos) => {
                        guard.splice_open_in(list_id, item_id, pos);
                        guard
                            .open_by_list
                            .get(list_id)
                            .and_then(|arr| arr.iter().position(|x| x == item_id))
                    }
                    None => {
                        let open = guard.refresh_open(list_id);
                        open.iter().position(|x| x == item_id)
                    }
                }
            }
        };
        self.push_event(AppEvent::ItemMoved {
            id: item_id.to_string(),
            open_index,
        });
        Ok(())
    }

    /// Re-place an item inside its own list with a fresh placement +
    /// entry. Used when a same-list move targets a fallback-tail item
    /// (its canonical entry was lost); functionally a cross-list move
    /// whose source and target coincide.
    fn replace_entry(
        &self,
        item_id: &str,
        map: &LoroMap,
        list_id: &str,
        target_index: usize,
        is_open: bool,
    ) -> Result<(), DocError> {
        let placement = new_id();
        map.insert(
            KEY_LOCATION,
            Location {
                list_id: list_id.to_string(),
                placement_id: placement.clone(),
            }
            .encode()
            .as_str(),
        )?;
        let raw_pos = self
            .plan_target(list_id, target_index, is_open, Some(item_id))
            .raw_pos;
        let order = self.order_list(list_id);
        let entry = OrderEntry {
            item_id: item_id.to_string(),
            placement_id: placement.clone(),
        };
        if raw_pos >= order.len() {
            order.push(entry.encode().as_str())?;
        } else {
            order.insert(raw_pos, entry.encode().as_str())?;
        }
        self.inner.commit();
        let open_index = {
            let mut guard = self.item_index.lock().expect("item index mutex poisoned");
            if let Some(m) = guard.meta.get_mut(item_id) {
                m.placement_id = placement;
            }
            let raw = guard.raw_orders.entry(list_id.to_string()).or_default();
            let at = raw_pos.min(raw.len());
            raw.insert(at, Some(entry));
            // Rare path — a full refresh also recomputes the visible
            // count now that a tail item became entry-backed.
            let open = guard.refresh_open(list_id);
            if is_open {
                open.iter().position(|x| x == item_id)
            } else {
                None
            }
        };
        self.push_event(AppEvent::ItemMoved {
            id: item_id.to_string(),
            open_index,
        });
        Ok(())
    }

    fn move_across_lists(
        &self,
        item_id: &str,
        map: &LoroMap,
        cur_list: &str,
        target_list_id: &str,
        target_index: usize,
        is_open: bool,
    ) -> Result<(), DocError> {
        let placement = new_id();
        let created_at = read_i64(map, KEY_CREATED_AT).unwrap_or(0);
        map.insert(
            KEY_LOCATION,
            Location {
                list_id: target_list_id.to_string(),
                placement_id: placement.clone(),
            }
            .encode()
            .as_str(),
        )?;
        // The workflow register is list-agnostic and rides along with
        // the item across lists (`spec/data-model.md`); nothing to clear.
        let (plan, src_positions, was_visible) = {
            let guard = self.item_index.lock().expect("item index mutex poisoned");
            (
                guard.plan_target(target_list_id, target_index, is_open, None),
                guard.entry_positions(cur_list, item_id),
                guard.canonical_raw_pos(cur_list, item_id).is_some(),
            )
        };
        let raw_pos = plan.raw_pos;
        let target_order = self.order_list(target_list_id);
        let entry = OrderEntry {
            item_id: item_id.to_string(),
            placement_id: placement.clone(),
        };
        if raw_pos >= target_order.len() {
            target_order.push(entry.encode().as_str())?;
        } else {
            target_order.insert(raw_pos, entry.encode().as_str())?;
        }
        // Best-effort cleanup: drop every entry for this item from the
        // source order (canonical + any stale duplicates).
        let src_order = self.order_list(cur_list);
        for p in src_positions.iter().rev() {
            src_order.delete(*p, 1)?;
        }
        self.inner.commit();
        let open_index = {
            let mut guard = self.item_index.lock().expect("item index mutex poisoned");
            guard.set_meta(
                item_id,
                ItemMeta {
                    list_id: target_list_id.to_string(),
                    placement_id: placement,
                    open: is_open,
                    created_at,
                },
            );
            if let Some(raw) = guard.raw_orders.get_mut(cur_list) {
                for p in src_positions.iter().rev() {
                    if *p < raw.len() {
                        raw.remove(*p);
                    }
                }
            }
            if was_visible {
                guard.bump_visible(cur_list, -1);
            }
            let raw = guard
                .raw_orders
                .entry(target_list_id.to_string())
                .or_default();
            let at = raw_pos.min(raw.len());
            raw.insert(at, Some(entry));
            guard.bump_visible(target_list_id, 1);
            if !is_open {
                None
            } else {
                guard.splice_open_out(cur_list, item_id);
                match plan.open_pos {
                    Some(pos) => {
                        guard.splice_open_in(target_list_id, item_id, pos);
                        guard
                            .open_by_list
                            .get(target_list_id)
                            .and_then(|arr| arr.iter().position(|x| x == item_id))
                    }
                    None => {
                        let open = guard.refresh_open(target_list_id);
                        open.iter().position(|x| x == item_id)
                    }
                }
            }
        };
        self.push_event(AppEvent::ItemListChanged {
            id: item_id.to_string(),
            list_id: target_list_id.to_string(),
            open_index,
        });
        Ok(())
    }

    /// Resolve a target index against `list_id`'s projection: the raw
    /// container position for the entry write, plus — when derivable
    /// without a walk — the exact open splice position. See
    /// [`ProjectionIndex::plan_target`].
    fn plan_target(
        &self,
        list_id: &str,
        target_index: usize,
        is_open: bool,
        exclude: Option<&str>,
    ) -> TargetPlan {
        let guard = self.item_index.lock().expect("item index mutex poisoned");
        guard.plan_target(list_id, target_index, is_open, exclude)
    }

    /// Convenience toggle over the workflow ladder: `done == true` is the
    /// Done transition; `done == false` is **un-done** — a plain write to
    /// Backlog (`spec/data-model.md` "Set lifecycle"), applied only to
    /// items whose resolved lifecycle is currently Done (so it never
    /// yanks a Todo/In Progress/Review item back to Backlog).
    pub fn set_item_done(&self, item_id: &str, done: bool) -> Result<(), DocError> {
        self.set_items_done(&[item_id], done)
    }

    /// Bulk [`set_item_done`](Self::set_item_done): one commit, one shared
    /// `now`. Surgical below `BULK_LIFECYCLE_EVENT_THRESHOLD`: work is
    /// proportional to the touched items' lists, never total doc size.
    /// At/above the threshold it falls back to one rebuild + diff.
    pub fn set_items_done(&self, item_ids: &[&str], done: bool) -> Result<(), DocError> {
        let write = if done {
            LifecycleWrite::Set(ItemLifecycle::Done)
        } else {
            LifecycleWrite::UnDone
        };
        self.set_items_lifecycle_impl(item_ids, write)
    }

    /// Convenience toggle over the bin mask: `binned == true` is the
    /// Binned transition (mask set, workflow register preserved for
    /// restore); `binned == false` is **restore** — clear the mask only,
    /// revealing the preserved workflow state (which may itself be Done).
    pub fn set_item_binned(&self, item_id: &str, binned: bool) -> Result<(), DocError> {
        self.set_items_binned(&[item_id], binned)
    }

    /// Bulk [`set_item_binned`](Self::set_item_binned): one commit, one
    /// shared `now`. Surgical / bulk-fallback split as `set_items_done`.
    pub fn set_items_binned(&self, item_ids: &[&str], binned: bool) -> Result<(), DocError> {
        let write = if binned {
            LifecycleWrite::Set(ItemLifecycle::Binned)
        } else {
            LifecycleWrite::Restore
        };
        self.set_items_lifecycle_impl(item_ids, write)
    }

    /// Move one item to a target [`ItemLifecycle`] in a single commit,
    /// writing the workflow register / bin mask (plus reflection stamps)
    /// per the transition table in `spec/data-model.md`. This is the
    /// primitive the board uses; the `done`/`bin`/`restore` helpers are
    /// convenience wrappers over it.
    pub fn set_item_lifecycle(
        &self,
        item_id: &str,
        lifecycle: ItemLifecycle,
    ) -> Result<(), DocError> {
        self.set_items_lifecycle(&[item_id], lifecycle)
    }

    /// Bulk [`set_item_lifecycle`]: move many items to the same target
    /// lifecycle in one commit (one shared `now`). Surgical below
    /// `BULK_LIFECYCLE_EVENT_THRESHOLD`, rebuild+diff at/above it.
    pub fn set_items_lifecycle(
        &self,
        item_ids: &[&str],
        lifecycle: ItemLifecycle,
    ) -> Result<(), DocError> {
        self.set_items_lifecycle_impl(item_ids, LifecycleWrite::Set(lifecycle))
    }

    /// Shared driver behind every lifecycle mutation: resolve all ids up
    /// front (an unknown id aborts before any write), apply the
    /// transition with one shared `now`, prune Focus on Done, commit
    /// once, and emit per-item events (or the bulk rebuild + diff).
    fn set_items_lifecycle_impl(
        &self,
        item_ids: &[&str],
        write: LifecycleWrite,
    ) -> Result<(), DocError> {
        if item_ids.is_empty() {
            return Ok(());
        }
        assert_unique_item_ids(item_ids)?;
        let pre_items = (item_ids.len() >= BULK_LIFECYCLE_EVENT_THRESHOLD)
            .then(|| self.iter_items().collect::<Vec<ItemView>>());
        let maps: Vec<(&str, LoroMap)> = item_ids
            .iter()
            .map(|id| self.find_item(id).map(|map| (*id, map)))
            .collect::<Result<_, _>>()?;
        let now = now_millis();
        let mut changed: Vec<(&str, LoroMap)> = Vec::new();
        for (item_id, map) in maps {
            if apply_lifecycle_write(&map, write, now)? {
                changed.push((item_id, map));
            }
        }
        if changed.is_empty() {
            return Ok(());
        }
        // Focus self-compacts on completion: a Done transition removes the
        // item's focus ref(s) in the *same* commit (`spec/focus.md`). This
        // is the one lifecycle transition that touches a second container —
        // a Done focus ref renders nothing, so Focus stays finite without
        // relying on the unwired `reconcile()`. Binned is left to the sweep.
        let focus_removed = if write == LifecycleWrite::Set(ItemLifecycle::Done) {
            let done_ids: HashSet<String> = changed.iter().map(|(id, _)| id.to_string()).collect();
            self.prune_focus_refs(&done_ids)
        } else {
            0
        };
        self.inner.commit();
        if focus_removed > 0 {
            self.push_event(AppEvent::FocusChanged);
        }
        if let Some(pre) = pre_items {
            self.rebuild_index();
            self.emit_item_diffs(&pre);
            return Ok(());
        }
        for (item_id, map) in changed {
            let open_index = self.sync_item_openness(item_id, &map);
            let (state, lifecycle_at) = workflow_of(&map);
            self.push_event(AppEvent::ItemLifecycleChanged {
                id: item_id.to_string(),
                state,
                lifecycle_at,
                started_at: read_i64(&map, KEY_STARTED_AT),
                done_at: read_i64(&map, KEY_DONE_AT),
                binned_at: read_i64(&map, KEY_BINNED_AT),
                open_index,
            });
        }
        Ok(())
    }

    // ---------- mutations: focus (`spec/focus.md`) ----------

    /// Add a reference to `item_id` in the Focus lens at visible position
    /// `index` (`usize::MAX` appends), one commit. No-ops if the item is
    /// already focused (does *not* move-to-top) or is not Open (a Done /
    /// binned item cannot be focused). Errors if the item is unknown.
    /// Folds a dead-ref sweep into the same commit.
    pub fn add_to_focus(&self, item_id: &str, index: usize) -> Result<(), DocError> {
        let open = {
            let guard = self.item_index.lock().expect("item index mutex poisoned");
            match guard.meta.get(item_id) {
                None => return Err(DocError::ItemNotFound(item_id.to_string())),
                Some(m) => m.open,
            }
        };
        let scan = self.scan_focus();
        if scan.visible_item_ids.iter().any(|id| id == item_id) {
            return Ok(());
        }
        if !open {
            return Ok(());
        }
        let focus = self.focus_list();
        for &i in scan.dead_idx.iter().rev() {
            focus.delete(i, 1)?;
        }
        // After the sweep the container holds exactly the visible refs, so
        // raw position == visible position.
        let new_len = scan.visible_item_ids.len();
        let at = index.min(new_len);
        let encoded = FocusRef::local(item_id).encode();
        if at >= new_len {
            focus.push(encoded.as_str())?;
        } else {
            focus.insert(at, encoded.as_str())?;
        }
        self.inner.commit();
        self.push_event(AppEvent::FocusChanged);
        Ok(())
    }

    /// Batch form of [`add_to_focus`](Self::add_to_focus): prepend a
    /// FocusRef for each of `item_ids` to the **top** of the Focus lens,
    /// in the given order, in a **single commit** — newly-focused items
    /// surface first ("what am I working on now", ordered top-down). Items
    /// that are unknown, not Open, already focused, or repeated within
    /// `item_ids` are skipped (each a no-op, mirroring the single-item
    /// form — unlike the bulk lifecycle paths, an unknown id does not abort
    /// the batch). Folds one dead-ref sweep into the same commit and emits
    /// at most one `FocusChanged`. Backs multi-select "add to focus".
    pub fn add_to_focus_many(&self, item_ids: &[&str]) -> Result<(), DocError> {
        if item_ids.is_empty() {
            return Ok(());
        }
        let scan = self.scan_focus();
        // Seed the "already present" set with the currently-visible refs so
        // the filter dedups against Focus *and* against repeats within
        // `item_ids` (first occurrence wins) in one pass.
        let mut present: HashSet<String> = scan.visible_item_ids.iter().cloned().collect();
        let to_add: Vec<&str> = {
            let guard = self.item_index.lock().expect("item index mutex poisoned");
            item_ids
                .iter()
                .copied()
                .filter(|id| {
                    guard.meta.get(*id).is_some_and(|m| m.open) && present.insert((*id).to_string())
                })
                .collect()
        };
        let swept = !scan.dead_idx.is_empty();
        if to_add.is_empty() && !swept {
            return Ok(());
        }
        let focus = self.focus_list();
        for &i in scan.dead_idx.iter().rev() {
            focus.delete(i, 1)?;
        }
        // After the sweep the container holds exactly the visible refs, so
        // inserting from index 0 upward prepends the batch to the top while
        // preserving `item_ids` order (to_add[0] ends up first).
        for (offset, id) in to_add.iter().enumerate() {
            focus.insert(offset, FocusRef::local(id).encode().as_str())?;
        }
        self.inner.commit();
        self.push_event(AppEvent::FocusChanged);
        Ok(())
    }

    /// Remove `item_id`'s reference(s) from the Focus lens, one commit.
    /// The item itself is untouched. No-op (no commit) when the item has
    /// no ref and there is no garbage to sweep. Folds a dead-ref sweep in.
    pub fn remove_from_focus(&self, item_id: &str) -> Result<(), DocError> {
        let target: HashSet<String> = std::iter::once(item_id.to_string()).collect();
        let removed = self.prune_focus_refs(&target);
        if removed == 0 {
            return Ok(());
        }
        self.inner.commit();
        self.push_event(AppEvent::FocusChanged);
        Ok(())
    }

    /// Batch form of [`remove_from_focus`](Self::remove_from_focus):
    /// remove the ref(s) of every id in `item_ids` from the Focus lens in
    /// a **single commit**. The items themselves are untouched. No-op (no
    /// commit) when nothing matched and there was no garbage to sweep.
    /// Folds the dead-ref sweep in.
    pub fn remove_from_focus_many(&self, item_ids: &[&str]) -> Result<(), DocError> {
        if item_ids.is_empty() {
            return Ok(());
        }
        let target: HashSet<String> = item_ids.iter().map(|s| s.to_string()).collect();
        let removed = self.prune_focus_refs(&target);
        if removed == 0 {
            return Ok(());
        }
        self.inner.commit();
        self.push_event(AppEvent::FocusChanged);
        Ok(())
    }

    /// Reorder `item_id`'s reference to visible position `index` within
    /// the Focus lens, one commit. No-op when the item is not focused (and
    /// nothing was swept). Folds a dead-ref sweep in first.
    pub fn move_in_focus(&self, item_id: &str, index: usize) -> Result<(), DocError> {
        let scan = self.scan_focus();
        let focus = self.focus_list();
        for &i in scan.dead_idx.iter().rev() {
            focus.delete(i, 1)?;
        }
        // After the sweep, positions align with `visible_item_ids`.
        let swept = !scan.dead_idx.is_empty();
        let commit_sweep = |doc: &Self| {
            doc.inner.commit();
            doc.push_event(AppEvent::FocusChanged);
        };
        let Some(from) = scan.visible_item_ids.iter().position(|id| id == item_id) else {
            if swept {
                commit_sweep(self);
            }
            return Ok(());
        };
        let to = index.min(scan.visible_item_ids.len().saturating_sub(1));
        if from == to {
            if swept {
                commit_sweep(self);
            }
            return Ok(());
        }
        focus.mov(from, to)?;
        self.inner.commit();
        self.push_event(AppEvent::FocusChanged);
        Ok(())
    }

    // ---------- reads: focus ----------

    /// Visible Focus item ids in curated order (Open, local, deduped).
    pub fn focus_refs(&self) -> Vec<String> {
        self.scan_focus().visible_item_ids
    }

    /// The Focus lens as an ordered `Vec<ItemView>` — the projection in
    /// `spec/focus.md`. Pure; never mutates.
    pub fn focus_view(&self) -> Vec<ItemView> {
        self.focus_refs()
            .into_iter()
            .filter_map(|id| self.get_item(&id))
            .collect()
    }

    /// Refresh the index after an item's lifecycle changed.
    /// Returns the item's open index within its list (`None` when
    /// hidden). Hiding is an O(open) splice; a restore recomputes the
    /// list's projection to find the re-entry position.
    fn sync_item_openness(&self, item_id: &str, map: &LoroMap) -> Option<usize> {
        let open_now = is_open(map);
        let mut guard = self.item_index.lock().expect("item index mutex poisoned");
        let (list_id, was_open) = {
            let m = guard.meta.get_mut(item_id)?;
            let was = m.open;
            m.open = open_now;
            (m.list_id.clone(), was)
        };
        if !open_now {
            guard.splice_open_out(&list_id, item_id);
            return None;
        }
        if was_open {
            // open → open (no transition): position unchanged.
            return guard
                .open_by_list
                .get(&list_id)
                .and_then(|arr| arr.iter().position(|x| x == item_id));
        }
        let open = guard.refresh_open(&list_id);
        open.iter().position(|x| x == item_id)
    }

    pub fn delete_binned(&self, item_id: &str) -> Result<(), DocError> {
        self.delete_binned_items(&[item_id])
    }

    /// Hard-delete the subset of binned items identified by `item_ids`
    /// in one commit. Errors if any id is not currently binned.
    /// Deletes the item map from `items` and best-effort removes the
    /// item's entries from its located order container.
    pub fn delete_binned_items(&self, item_ids: &[&str]) -> Result<(), DocError> {
        if item_ids.is_empty() {
            return Ok(());
        }
        assert_unique_item_ids(item_ids)?;
        for item_id in item_ids {
            let map = self.find_item(item_id)?;
            if read_i64(&map, KEY_BINNED_AT).is_none() {
                return Err(DocError::NotBinned);
            }
        }
        self.hard_delete_items(item_ids)
    }

    /// Hard-deletes every binned item. Returns how many were removed.
    pub fn empty_bin(&self) -> Result<usize, DocError> {
        let binned: Vec<String> = self
            .iter_items()
            .filter(|i| i.is_binned())
            .map(|i| i.id)
            .collect();
        if binned.is_empty() {
            return Ok(0);
        }
        let refs: Vec<&str> = binned.iter().map(String::as_str).collect();
        self.hard_delete_items(&refs)?;
        Ok(binned.len())
    }

    /// Shared hard-delete path: remove item maps + their order entries
    /// in one commit, then update the index and emit `ItemRemoved`s.
    /// Callers have already validated the ids.
    fn hard_delete_items(&self, item_ids: &[&str]) -> Result<(), DocError> {
        // Entry positions + visibility per item, gathered before any
        // mutation. `per_item` rows are (item_id, list_id, was_visible).
        type PerItem = Vec<(String, String, bool)>;
        let (per_list, per_item): (HashMap<String, Vec<usize>>, PerItem) = {
            let guard = self.item_index.lock().expect("item index mutex poisoned");
            let mut acc: HashMap<String, Vec<usize>> = HashMap::new();
            let mut per_item = Vec::with_capacity(item_ids.len());
            for item_id in item_ids {
                let Some(m) = guard.meta.get(*item_id) else {
                    continue;
                };
                acc.entry(m.list_id.clone())
                    .or_default()
                    .extend(guard.entry_positions(&m.list_id, item_id));
                per_item.push((
                    (*item_id).to_string(),
                    m.list_id.clone(),
                    guard.canonical_raw_pos(&m.list_id, item_id).is_some(),
                ));
            }
            for positions in acc.values_mut() {
                positions.sort_unstable();
                positions.dedup();
            }
            (acc, per_item)
        };
        let items = self.items();
        for item_id in item_ids {
            items.delete(item_id)?;
        }
        for (list_id, positions) in &per_list {
            let order = self.order_list(list_id);
            for p in positions.iter().rev() {
                order.delete(*p, 1)?;
            }
        }
        self.inner.commit();
        {
            let mut guard = self.item_index.lock().expect("item index mutex poisoned");
            for item_id in item_ids {
                guard.remove_item(item_id);
            }
            for (list_id, positions) in &per_list {
                if let Some(raw) = guard.raw_orders.get_mut(list_id) {
                    for p in positions.iter().rev() {
                        if *p < raw.len() {
                            raw.remove(*p);
                        }
                    }
                }
            }
            // Removals are exact: splice out of the open arrays and
            // drop visible counts — no projection walk needed.
            for (item_id, list_id, was_visible) in &per_item {
                guard.splice_open_out(list_id, item_id);
                if *was_visible {
                    guard.bump_visible(list_id, -1);
                }
            }
        }
        for item_id in item_ids {
            self.push_event(AppEvent::ItemRemoved {
                id: (*item_id).to_string(),
            });
        }
        Ok(())
    }

    // ---------- mutations: lists ----------

    pub fn add_list(&self, name: &str) -> Result<String, DocError> {
        let name = name.trim();
        if name.is_empty() {
            return Err(DocError::Invalid("list name is empty".into()));
        }
        let id = new_id();
        let lists = self.lists();
        let map = lists.push_container(LoroMap::new())?;
        let now = now_millis();
        map.insert(KEY_ID, id.as_str())?;
        map.insert(KEY_NAME, name)?;
        map.insert(KEY_CREATED_AT, now)?;
        self.inner.commit();
        let index = self
            .visible_list_index(&id)
            .ok_or_else(|| DocError::ListNotFound(id.clone()))?;
        self.push_event(AppEvent::ListAdded {
            id: id.clone(),
            name: name.to_string(),
            created_at: now,
            archived_at: None,
            index,
        });
        Ok(id)
    }

    /// Toggle the global "show counts on non-Inbox lists" setting. Inbox
    /// is unaffected — its count is always visible (subject to count >
    /// 0) and is not gated by this flag. No-op when the value is
    /// unchanged, so flicking the menu twice doesn't emit phantom
    /// events or undo steps.
    pub fn set_show_list_counts(&self, show: bool) -> Result<(), DocError> {
        let settings = self.settings_map();
        let current = read_bool(&settings, KEY_SHOW_LIST_COUNTS).unwrap_or(true);
        if current == show {
            return Ok(());
        }
        if show {
            // Drop the key entirely on the on path so the default state
            // leaves no trace — on-disk state matches a never-toggled doc.
            settings.delete(KEY_SHOW_LIST_COUNTS)?;
        } else {
            settings.insert(KEY_SHOW_LIST_COUNTS, false)?;
        }
        self.inner.commit();
        let post = settings_view(&settings);
        self.push_event(AppEvent::SettingsChanged {
            show_list_counts: post.show_list_counts,
            inbox_view: post.inbox_view,
        });
        Ok(())
    }

    pub fn rename_list(&self, list_id: &str, name: &str) -> Result<(), DocError> {
        if list_id == LIST_INBOX {
            return Err(DocError::CannotRenameBuiltin(LIST_INBOX.into()));
        }
        let name = name.trim();
        let (_, map) = self.find_list(list_id)?;
        map.insert(KEY_NAME, name)?;
        self.inner.commit();
        self.push_event(AppEvent::ListRenamed {
            id: list_id.to_string(),
            name: name.to_string(),
        });
        Ok(())
    }

    /// Set or clear a user-created list's display icon. `icon` is stored
    /// verbatim as a whole grapheme string (an emoji); a trimmed-empty
    /// value clears it (so clients fall back to the built-in glyph).
    /// No-op when the resulting value matches the current one, so repeat
    /// saves don't emit phantom events or undo steps. Reserved `inbox`
    /// (Inbox) has no ListMeta row, so it cannot carry an icon.
    pub fn set_list_icon(&self, list_id: &str, icon: &str) -> Result<(), DocError> {
        if list_id == LIST_INBOX {
            return Err(DocError::CannotRenameBuiltin(LIST_INBOX.into()));
        }
        let (_, map) = self.find_list(list_id)?;
        let trimmed = icon.trim();
        let next: Option<String> = if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        };
        let current = read_string(&map, KEY_ICON).filter(|s| !s.is_empty());
        if current.as_deref() == next.as_deref() {
            return Ok(());
        }
        match &next {
            Some(s) => map.insert(KEY_ICON, s.as_str())?,
            None => {
                map.delete(KEY_ICON)?;
            }
        }
        self.inner.commit();
        self.push_event(AppEvent::ListIconChanged {
            id: list_id.to_string(),
            icon: next,
        });
        Ok(())
    }

    /// Save (`Some`) or clear (`None`) a list's default view — the lens
    /// a client renders it in when it has no local override of its own
    /// (`spec/board.md`). The whole view is one encoded scalar register,
    /// so a concurrent save on another device replaces it atomically.
    ///
    /// Works for the reserved `inbox` too: it has no ListMeta row, so its
    /// default is written to the doc-level settings map instead and
    /// surfaces as a `SettingsChanged` event rather than
    /// `ListDefaultViewChanged`. No-op when the value is unchanged, so
    /// re-saving the same view emits no phantom event or undo step.
    pub fn set_default_view(
        &self,
        list_id: &str,
        view: Option<DefaultView>,
    ) -> Result<(), DocError> {
        if list_id == LIST_INBOX {
            let settings = self.settings_map();
            if read_default_view(&settings, KEY_INBOX_VIEW) == view {
                return Ok(());
            }
            match view {
                Some(v) => settings.insert(KEY_INBOX_VIEW, v.encode())?,
                None => {
                    settings.delete(KEY_INBOX_VIEW)?;
                }
            }
            self.inner.commit();
            let post = settings_view(&settings);
            self.push_event(AppEvent::SettingsChanged {
                show_list_counts: post.show_list_counts,
                inbox_view: post.inbox_view,
            });
            return Ok(());
        }
        let (_, map) = self.find_list(list_id)?;
        if read_default_view(&map, KEY_VIEW) == view {
            return Ok(());
        }
        match view {
            Some(v) => map.insert(KEY_VIEW, v.encode())?,
            None => {
                map.delete(KEY_VIEW)?;
            }
        }
        self.inner.commit();
        self.push_event(AppEvent::ListDefaultViewChanged {
            id: list_id.to_string(),
            view,
        });
        Ok(())
    }

    /// Archive (`true`) or unarchive (`false`) a user-created list —
    /// the user-facing removal from the active workspace
    /// (`spec/data-model.md` "Archived lists"). Metadata-only: writes or
    /// deletes the ListMeta `archived_at` register and touches nothing
    /// else — no item, order, lifecycle, or Focus mutation. Re-applying
    /// the current state is a no-op (no commit, no event). Refuses for
    /// the reserved `inbox`.
    pub fn set_list_archived(&self, list_id: &str, archived: bool) -> Result<(), DocError> {
        if list_id == LIST_INBOX {
            return Err(DocError::CannotDeleteBuiltin(LIST_INBOX.into()));
        }
        let (_, map) = self.find_list(list_id)?;
        let current = read_i64(&map, KEY_ARCHIVED_AT);
        if current.is_some() == archived {
            return Ok(());
        }
        let next = if archived {
            let now = now_millis();
            map.insert(KEY_ARCHIVED_AT, now)?;
            Some(now)
        } else {
            map.delete(KEY_ARCHIVED_AT)?;
            None
        };
        self.inner.commit();
        self.push_event(AppEvent::ListArchivedChanged {
            id: list_id.to_string(),
            archived_at: next,
        });
        Ok(())
    }

    /// For an **active** list, `target_index` addresses the active-list
    /// projection (archived rows excluded — the index space clients'
    /// draggable navs actually render); it is resolved to the raw CRDT
    /// index here. Moving an archived list keeps raw-index semantics
    /// (no client UI reorders archived lists).
    pub fn move_list(&self, list_id: &str, target_index: usize) -> Result<(), DocError> {
        if list_id == LIST_INBOX {
            return Err(DocError::CannotMoveBuiltin(LIST_INBOX.into()));
        }
        let lists = self.lists();
        let (from, map) = self.find_list(list_id)?;
        let len = lists.len();
        let to = if read_i64(&map, KEY_ARCHIVED_AT).is_none() {
            self.resolve_active_move_target(list_id, target_index)?
        } else {
            target_index.min(len.saturating_sub(1))
        };
        if from == to {
            return Ok(());
        }
        lists.mov(from, to)?;
        self.inner.commit();
        let index = self
            .visible_list_index(list_id)
            .ok_or_else(|| DocError::ListNotFound(list_id.to_string()))?;
        self.push_event(AppEvent::ListMoved {
            id: list_id.to_string(),
            index,
        });
        Ok(())
    }

    /// Refuses for the always-on `inbox` list. Every item locating to
    /// the deleted list (open, done and binned) is moved to `inbox` with
    /// a fresh placement, appended to `order/main` in the deleted
    /// list's resolved order. The abandoned order container remains as
    /// unreachable history.
    pub fn delete_list(&self, list_id: &str) -> Result<(), DocError> {
        if list_id == LIST_INBOX {
            return Err(DocError::CannotDeleteBuiltin(LIST_INBOX.into()));
        }
        let (idx, _) = self.find_list(list_id)?;
        let resolved = {
            let guard = self.item_index.lock().expect("item index mutex poisoned");
            guard.resolved(list_id)
        };
        let pre_items: Vec<ItemView> = self.iter_items().collect();
        let pre_lists: Vec<ListView> = self.all_lists();
        let pre_settings = self.get_settings();
        // Deleting a list discards its contents to the bin rather than
        // dumping them live into Home: every item is relocated to `inbox`
        // (so it has a real home list if later restored) and, unless it
        // was already binned, marked binned with a shared timestamp. The
        // location move keeps `order/main` and restore-target semantics
        // valid without leaning on orphan fallback.
        let binned_at = now_millis();
        let main_order = self.order_list(LIST_INBOX);
        for item_id in &resolved {
            let map = self.find_item(item_id)?;
            let placement = new_id();
            map.insert(
                KEY_LOCATION,
                Location {
                    list_id: LIST_INBOX.to_string(),
                    placement_id: placement.clone(),
                }
                .encode()
                .as_str(),
            )?;
            if read_i64(&map, KEY_BINNED_AT).is_none() {
                map.insert(KEY_BINNED_AT, binned_at)?;
            }
            main_order.push(
                OrderEntry {
                    item_id: item_id.clone(),
                    placement_id: placement,
                }
                .encode()
                .as_str(),
            )?;
        }
        self.lists().delete(idx, 1)?;
        self.inner.commit();
        // Rare operation; a wholesale rebuild is simpler than patching
        // two lists' shadows plus membership.
        self.rebuild_index();
        // Emit the full pre/post state diff: each item picks up an
        // `ItemLifecycleChanged` (now binned) and `ItemListChanged` (now
        // `inbox`), plus the `ListRemoved` for the gone list.
        self.emit_state_diff(&pre_items, &pre_lists, &pre_settings);
        Ok(())
    }

    /// Explicit, idempotent order-entry repair (never run implicitly by
    /// reads): removes stale and duplicate entries and materializes
    /// real entries for fallback-tail items, in one commit. Returns the
    /// number of repairs; `0` means the doc was clean and nothing was
    /// committed. Visible projections are unchanged by construction, so
    /// no events are emitted.
    pub fn reconcile(&self) -> Result<usize, DocError> {
        struct ListPlan {
            stale: Vec<usize>,
            /// (item_id, placement_id) entries to append for
            /// fallback-tail items, in deterministic tail order.
            append: Vec<OrderEntry>,
        }
        // Items with a missing/unparseable location get a fresh one
        // written (main + new placement) plus a matching entry.
        let mut fix_location: Vec<(String, String)> = Vec::new();
        let mut plans: HashMap<String, ListPlan> = HashMap::new();
        {
            let guard = self.item_index.lock().expect("item index mutex poisoned");
            let mut lists: HashSet<&String> = guard.raw_orders.keys().collect();
            lists.extend(guard.members.keys());
            for list_id in lists {
                let mut seen = HashSet::new();
                let mut stale = Vec::new();
                for (i, entry) in guard
                    .raw_orders
                    .get(list_id)
                    .map(Vec::as_slice)
                    .unwrap_or(&[])
                    .iter()
                    .enumerate()
                {
                    let visible = entry.as_ref().is_some_and(|e| {
                        guard.meta.get(&e.item_id).is_some_and(|m| {
                            m.list_id == *list_id && m.placement_id == e.placement_id
                        }) && seen.insert(e.item_id.clone())
                    });
                    if !visible {
                        stale.push(i);
                    }
                }
                // Fallback tail in its deterministic order.
                let mut tail: Vec<&String> = guard
                    .members
                    .get(list_id)
                    .into_iter()
                    .flatten()
                    .filter(|id| !seen.contains(*id))
                    .collect();
                tail.sort_by(|a, b| {
                    let (ma, mb) = (&guard.meta[*a], &guard.meta[*b]);
                    ma.created_at.cmp(&mb.created_at).then_with(|| a.cmp(b))
                });
                let mut append = Vec::new();
                for id in tail {
                    let m = &guard.meta[id];
                    if m.placement_id.is_empty() {
                        let placement = new_id();
                        fix_location.push((id.clone(), placement.clone()));
                        append.push(OrderEntry {
                            item_id: id.clone(),
                            placement_id: placement,
                        });
                    } else {
                        append.push(OrderEntry {
                            item_id: id.clone(),
                            placement_id: m.placement_id.clone(),
                        });
                    }
                }
                if !stale.is_empty() || !append.is_empty() {
                    plans.insert(list_id.clone(), ListPlan { stale, append });
                }
            }
        }
        // Focus backstop: prune dead refs (missing / done / binned /
        // foreign / duplicate) from the focus container. Idempotent; the
        // primary compaction paths are auto-remove-on-Done and the sweep
        // folded into each focus mutation (`spec/focus.md`).
        let focus_dead = self.scan_focus().dead_idx;
        if plans.is_empty() && focus_dead.is_empty() {
            return Ok(0);
        }
        let mut repairs = 0usize;
        for (item_id, placement) in &fix_location {
            let map = self.find_item(item_id)?;
            map.insert(
                KEY_LOCATION,
                Location {
                    list_id: LIST_INBOX.to_string(),
                    placement_id: placement.clone(),
                }
                .encode()
                .as_str(),
            )?;
        }
        for (list_id, plan) in &plans {
            let order = self.order_list(list_id);
            for p in plan.stale.iter().rev() {
                order.delete(*p, 1)?;
                repairs += 1;
            }
            for entry in &plan.append {
                order.push(entry.encode().as_str())?;
                repairs += 1;
            }
        }
        let focus = self.focus_list();
        for p in focus_dead.iter().rev() {
            focus.delete(*p, 1)?;
            repairs += 1;
        }
        self.inner.commit();
        self.rebuild_index();
        Ok(repairs)
    }

    // ---------- reads ----------

    /// Items of one list in resolved order, all lifecyclees except binned
    /// unless `include_binned`.
    pub fn items_in_list(&self, list_id: &str, include_binned: bool) -> Vec<ItemView> {
        let resolved = {
            let guard = self.item_index.lock().expect("item index mutex poisoned");
            guard.resolved(list_id)
        };
        resolved
            .iter()
            .filter_map(|id| self.get_item(id))
            .filter(|i| include_binned || !i.is_binned())
            .collect()
    }

    pub fn binned_items(&self) -> Vec<ItemView> {
        self.iter_items().filter(|i| i.is_binned()).collect()
    }

    /// Every ListMeta row in CRDT order — the canonical projection.
    /// Includes archived lists; archiving never removes a list from
    /// here (`spec/data-model.md` "Archived lists"). Use
    /// [`Self::active_lists`] for the active-workspace view.
    pub fn all_lists(&self) -> Vec<ListView> {
        let lists = self.lists();
        let mut out = Vec::with_capacity(lists.len());
        for i in 0..lists.len() {
            if let Some(map) = list_map_at(&lists, i)
                && let Some(view) = list_view(&map)
            {
                out.push(view);
            }
        }
        out
    }

    /// [`Self::all_lists`] filtered to active (non-archived) lists, in
    /// the same CRDT order.
    pub fn active_lists(&self) -> Vec<ListView> {
        self.all_lists()
            .into_iter()
            .filter(|l| l.archived_at.is_none())
            .collect()
    }

    pub fn get_settings(&self) -> SettingsView {
        settings_view(&self.settings_map())
    }

    /// Semantic full-account export. This is intentionally a compact,
    /// human-readable data dump rather than a CRDT/state backup. Items
    /// are emitted grouped per list in resolved order, so array order
    /// carries the ordering across the export/import boundary.
    pub fn export_json(&self) -> JsonExport {
        let s = self.get_settings();
        let mut lists = Vec::with_capacity(self.lists().len() + 1);
        lists.push(ExportList {
            id: LIST_INBOX.to_string(),
            name: INBOX_NAME.to_string(),
            icon: None,
            // Inbox's default view rides on `settings.inbox_view`, not
            // on its (nonexistent) ListMeta row.
            view: None,
            archived_at: None,
            created_at: None,
            builtin: true,
        });
        lists.extend(self.all_lists().into_iter().map(|list| ExportList {
            id: list.id,
            name: list.name,
            icon: list.icon,
            view: list.default_view.map(|v| v.encode()),
            archived_at: list.archived_at,
            created_at: Some(list.created_at),
            builtin: false,
        }));

        let items = self
            .iter_items()
            .map(|item| ExportItem {
                id: item.id,
                text: item.text,
                notes: item.notes,
                list_id: item.list_id,
                lifecycle: ExportLifecycle {
                    state: item.state.name().to_string(),
                    at: item.lifecycle_at,
                },
                deadline: item.deadline,
                when: item.when,
                duration: item.duration,
                created_at: item.created_at,
                started_at: item.started_at,
                done_at: item.done_at,
                binned_at: item.binned_at,
            })
            .collect();

        JsonExport {
            version: 1,
            settings: ExportSettings {
                show_list_counts: s.show_list_counts,
                inbox_view: s.inbox_view.map(|v| v.encode()),
            },
            lists,
            items,
            // Visible refs only, in Focus order — dead garbage (foreign,
            // missing, non-open, duplicate) never rides an export.
            focus: self.scan_focus().visible_item_ids,
        }
    }

    /// Pretty-printed JSON dump of `export_json` — what the web client
    /// hands the user as `monoplan-*.json`. Pretty by default because the
    /// file is meant to be opened in a text editor; consumers wanting a
    /// compact form can re-serialize. Serialization of `JsonExport` is
    /// statically infallible (only strings, ints, bools, vecs, options),
    /// so the `expect` is a structural invariant, not user-reachable.
    pub fn export_json_string(&self) -> String {
        serde_json::to_string_pretty(&self.export_json())
            .expect("JsonExport contains only primitives — serialization is infallible")
    }

    /// Additive JSON import. Source lists are created as fresh user
    /// lists (new IDs); source items keep their text / notes /
    /// timestamps / done-binned state but get fresh IDs and placements
    /// and follow the id-map: anything in the source's `inbox` lands in
    /// the local `inbox`, builtin entries are not duplicated, and items
    /// whose `list_id` references a list not present in the export fall
    /// back to `inbox` (same orphan handling as `delete_list`).
    /// Focus membership (`spec/focus.md`) rides `export.focus`: refs are
    /// remapped onto the fresh item ids and **appended** after any
    /// existing local Focus (additive import doesn't disturb local
    /// curation); refs to items that weren't imported or aren't Open are
    /// dropped silently, like malformed `deadline` values.
    /// Existing local content is untouched. All writes land in a
    /// single Loro commit; UI events are emitted via the standard
    /// pre/post state-diff.
    pub fn import_json(&self, export: &JsonExport) -> Result<ImportSummary, DocError> {
        if export.version != 1 {
            return Err(DocError::Invalid(format!(
                "unsupported export version: {} (expected 1)",
                export.version
            )));
        }

        let pre_items: Vec<ItemView> = self.iter_items().collect();
        let pre_lists: Vec<ListView> = self.all_lists();
        let pre_settings = self.get_settings();

        let mut id_map: HashMap<String, String> = HashMap::new();
        id_map.insert(LIST_INBOX.to_string(), LIST_INBOX.to_string());

        let lists = self.lists();
        let mut lists_added: usize = 0;
        for src_list in &export.lists {
            if src_list.builtin || src_list.id == LIST_INBOX {
                id_map.insert(src_list.id.clone(), LIST_INBOX.to_string());
                continue;
            }
            let name = src_list.name.trim();
            if name.is_empty() {
                continue;
            }
            let new_list_id = new_id();
            let map = lists.push_container(LoroMap::new())?;
            let created_at = src_list.created_at.unwrap_or_else(now_millis);
            map.insert(KEY_ID, new_list_id.as_str())?;
            map.insert(KEY_NAME, name)?;
            map.insert(KEY_CREATED_AT, created_at)?;
            // Carry the display icon through; a trimmed-empty value (or none)
            // leaves the key unset so clients fall back to the built-in glyph.
            if let Some(icon) = src_list
                .icon
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
            {
                map.insert(KEY_ICON, icon)?;
            }
            // Carry the saved default view through. An unrecognized value
            // in a hand-edited export is dropped rather than stored, so
            // the doc never holds a view string this build can't read.
            if let Some(view) = src_list.view.as_deref().and_then(DefaultView::parse) {
                map.insert(KEY_VIEW, view.encode())?;
            }
            // Archive state round-trips; exports written before the field
            // existed carry no `archivedAt` and import as active.
            if let Some(t) = src_list.archived_at {
                map.insert(KEY_ARCHIVED_AT, t)?;
            }
            id_map.insert(src_list.id.clone(), new_list_id);
            lists_added += 1;
        }

        let items = self.items();
        let mut items_added: usize = 0;
        let mut items_skipped: usize = 0;
        // Source item id → (fresh id, was-Open) — feeds the focus remap
        // below. Skipped items never enter it, so their refs drop out.
        let mut item_id_map: HashMap<String, (String, bool)> = HashMap::new();
        for src_item in &export.items {
            let text = src_item.text.trim();
            if text.is_empty() {
                items_skipped += 1;
                continue;
            }
            let target_list_id = id_map
                .get(&src_item.list_id)
                .cloned()
                .unwrap_or_else(|| LIST_INBOX.to_string());
            let new_item_id = new_id();
            let placement = new_id();
            let map = items.insert_container(&new_item_id, LoroMap::new())?;
            map.insert(KEY_ID, new_item_id.as_str())?;
            map.insert(KEY_TEXT, text)?;
            map.insert(
                KEY_LOCATION,
                Location {
                    list_id: target_list_id.clone(),
                    placement_id: placement.clone(),
                }
                .encode()
                .as_str(),
            )?;
            map.insert(KEY_CREATED_AT, src_item.created_at)?;
            // Workflow register + reflection stamps. An unrecognized
            // state name in a hand-edited export degrades to Backlog:
            // visible and open.
            let state = WorkflowState::parse_name(&src_item.lifecycle.state)
                .unwrap_or(WorkflowState::Backlog);
            let at = src_item.lifecycle.at;
            // The exact fallback value stays unwritten so a plain
            // Backlog item round-trips to a keyless map.
            if state != WorkflowState::Backlog || at != src_item.created_at {
                write_workflow(&map, state, at)?;
            }
            if let Some(t) = src_item.started_at {
                map.insert(KEY_STARTED_AT, t)?;
            }
            // Carry a well-formed deadline through; a malformed one in a
            // hand-edited export is silently dropped rather than aborting
            // the whole import.
            if let Some(date) = src_item
                .deadline
                .as_deref()
                .and_then(|d| parse_deadline(d).ok())
            {
                map.insert(KEY_DEADLINE, date.as_str())?;
            }
            if let Some(value) = src_item.when.as_deref().and_then(|w| parse_when(w).ok()) {
                map.insert(KEY_WHEN, value.as_str())?;
            }
            // Same leniency for an out-of-range duration.
            if let Some(n) = src_item
                .duration
                .filter(|n| (1..=MAX_DURATION_MINUTES).contains(n))
            {
                map.insert(KEY_DURATION, i64::from(n))?;
            }
            let notes = src_item.notes.trim();
            if !notes.is_empty() {
                let text = map.ensure_mergeable_text(KEY_NOTES)?;
                text.insert(0, notes)?;
            }
            if let Some(t) = src_item.done_at {
                map.insert(KEY_DONE_AT, t)?;
            }
            if let Some(t) = src_item.binned_at {
                map.insert(KEY_BINNED_AT, t)?;
            }
            self.order_list(&target_list_id).push(
                OrderEntry {
                    item_id: new_item_id.clone(),
                    placement_id: placement,
                }
                .encode()
                .as_str(),
            )?;
            let open = src_item.binned_at.is_none() && state.is_open();
            item_id_map.insert(src_item.id.clone(), (new_item_id, open));
            items_added += 1;
        }

        // Re-establish Focus membership over the fresh ids, appended after
        // any existing local refs. Imported ids are freshly minted, so
        // they can't collide with local Focus — only in-export dedup is
        // needed. Foreign-doc refs (never emitted, but a hand-edited file
        // could carry one) and refs to skipped / non-Open items drop out.
        let mut focus_added: usize = 0;
        let focus = self.focus_list();
        let mut focus_seen: HashSet<String> = HashSet::new();
        for raw in &export.focus {
            let Some(r) = FocusRef::parse(raw).filter(FocusRef::is_local) else {
                continue;
            };
            let Some((new_item_id, true)) = item_id_map.get(&r.item_id) else {
                continue;
            };
            if !focus_seen.insert(new_item_id.clone()) {
                continue;
            }
            focus.push(FocusRef::local(new_item_id).encode().as_str())?;
            focus_added += 1;
        }

        self.inner.commit();
        self.rebuild_index();
        self.emit_state_diff(&pre_items, &pre_lists, &pre_settings);
        // Focus isn't covered by the state diff; mirror the mutators'
        // manual signal so the lens re-renders after an import.
        if focus_added > 0 {
            self.push_event(AppEvent::FocusChanged);
        }

        Ok(ImportSummary {
            lists_added,
            items_added,
            items_skipped,
            focus_added,
        })
    }

    /// JSON-string convenience wrapper around `import_json` — what the
    /// web client hands the user-selected file contents to.
    pub fn import_json_str(&self, json: &str) -> Result<ImportSummary, DocError> {
        let export: JsonExport = serde_json::from_str(json)
            .map_err(|e| DocError::Invalid(format!("invalid JSON export: {e}")))?;
        self.import_json(&export)
    }

    pub fn get_item(&self, item_id: &str) -> Option<ItemView> {
        let map = self.find_item(item_id).ok()?;
        item_view(&map)
    }

    pub fn get_list_meta(&self, list_id: &str) -> Option<ListView> {
        let (_, map) = self.find_list(list_id).ok()?;
        list_view(&map)
    }

    /// Per-list nav view: ids of items in this list that are neither
    /// done nor binned, in resolved order.
    pub fn open_item_ids(&self, list_id: &str) -> Vec<String> {
        let guard = self.item_index.lock().expect("item index mutex poisoned");
        guard.open_by_list.get(list_id).cloned().unwrap_or_default()
    }

    /// Cross-list "Done" view: ids of done-but-not-binned items, sorted
    /// by the workflow register's `at` descending. Ties broken by id
    /// ascending so the order is deterministic across devices despite
    /// client-clock skew. Binned items are excluded — Bin owns them in
    /// the UI even if their preserved state is Done.
    pub fn done_item_ids(&self) -> Vec<String> {
        let mut items: Vec<ItemView> = self
            .iter_items()
            .filter(|i| i.is_done() && !i.is_binned())
            .collect();
        items.sort_by(|a, b| {
            b.lifecycle_at
                .cmp(&a.lifecycle_at)
                .then_with(|| a.id.cmp(&b.id))
        });
        items.into_iter().map(|i| i.id).collect()
    }

    /// Cross-list "Bin" view: ids sorted by `binned_at` descending.
    /// Includes items that are also done — done-ness is preserved when
    /// binning. Same tiebreaker as `done_item_ids`.
    pub fn binned_item_ids(&self) -> Vec<String> {
        let mut items: Vec<ItemView> = self.iter_items().filter(|i| i.is_binned()).collect();
        items.sort_by(|a, b| {
            let at = a.binned_at.unwrap_or(0);
            let bt = b.binned_at.unwrap_or(0);
            bt.cmp(&at).then_with(|| a.id.cmp(&b.id))
        });
        items.into_iter().map(|i| i.id).collect()
    }

    /// Materialize every item in canonical walk order: `inbox` first,
    /// then user lists in `lists` container order, then any orphan list
    /// ids (sorted) — each list in its resolved order. This grouped
    /// order is the deterministic replacement for the v1 global CRDT
    /// order; per-list consumers reconstruct their arrays from it.
    /// Intended for one-shot client attachment/resync snapshots; open
    /// mutation paths should use `AppEvent`s instead.
    pub fn all_items(&self) -> Vec<ItemView> {
        self.iter_items().collect()
    }

    /// Canonical list-id walk order backing `iter_items` — see
    /// [`Doc::all_items`].
    fn ordered_list_ids(&self) -> Vec<String> {
        let mut out = vec![LIST_INBOX.to_string()];
        let mut known: HashSet<String> = out.iter().cloned().collect();
        for list in self.all_lists() {
            if known.insert(list.id.clone()) {
                out.push(list.id);
            }
        }
        let mut orphans: Vec<String> = {
            let guard = self.item_index.lock().expect("item index mutex poisoned");
            guard
                .members
                .keys()
                .filter(|l| !known.contains(*l))
                .cloned()
                .collect()
        };
        orphans.sort();
        out.extend(orphans);
        out
    }

    fn iter_items(&self) -> impl Iterator<Item = ItemView> + '_ {
        let mut ids: Vec<String> = Vec::new();
        for list_id in self.ordered_list_ids() {
            let guard = self.item_index.lock().expect("item index mutex poisoned");
            ids.extend(guard.resolved(&list_id));
        }
        ids.into_iter().filter_map(move |id| self.get_item(&id))
    }

    // ---------- op stream ----------

    /// Encrypted full-state snapshot at the doc's current frontier.
    /// Used by the sync engine to satisfy a server `SnapshotRequest`.
    /// Independent of `last_persisted_vv` — the snapshot covers the whole
    /// doc, not a delta — so this does not advance any push-state
    /// bookkeeping; producing a snapshot is side-effect-free locally.
    pub fn snapshot_blob(&self, dek: &Dek) -> Result<EncryptedBlob, DocError> {
        let plaintext = self.export_snapshot_bytes()?;
        let (ciphertext, nonce) = dek.seal(&plaintext)?;
        Ok(EncryptedBlob {
            nonce: nonce.to_vec(),
            ciphertext,
        })
    }

    /// Plaintext full-state Loro snapshot. Same bytes that
    /// `snapshot_blob` seals for the server, but unencrypted — for
    /// user-driven backup / interop. Loro's import on a fresh doc
    /// reconstructs identical state from this blob.
    pub fn export_snapshot_bytes(&self) -> Result<Vec<u8>, DocError> {
        Ok(self.inner.export(ExportMode::Snapshot)?)
    }

    /// Encrypted blob containing every commit since `last_persisted_vv`
    /// — the delta the WAL capture path appends. Returns `None` if
    /// there's nothing new to capture.
    pub fn pending_export(&self, dek: &Dek) -> Result<Option<EncryptedBlob>, DocError> {
        if !self.has_uncaptured_ops() {
            return Ok(None);
        }
        let plaintext = self
            .inner
            .export(ExportMode::updates(&self.last_persisted_vv))?;
        let (ciphertext, nonce) = dek.seal(&plaintext)?;
        Ok(Some(EncryptedBlob {
            nonce: nonce.to_vec(),
            ciphertext,
        }))
    }

    /// Encrypted blob containing every commit strictly after `from` —
    /// the outbound-sync export. The engine passes its `server_known_vv`
    /// here so the delta is derived from Loro history, independent of
    /// which WAL rows still exist. Returns `None` when `from` already
    /// covers the whole oplog.
    pub fn export_updates_since(
        &self,
        dek: &Dek,
        from: &VersionVector,
    ) -> Result<Option<EncryptedBlob>, DocError> {
        if from.includes_vv(&self.inner.oplog_vv()) {
            return Ok(None);
        }
        let plaintext = self.inner.export(ExportMode::updates(from))?;
        if plaintext.is_empty() {
            return Ok(None);
        }
        let (ciphertext, nonce) = dek.seal(&plaintext)?;
        Ok(Some(EncryptedBlob {
            nonce: nonce.to_vec(),
            ciphertext,
        }))
    }

    /// Encrypted full-history snapshot at the state described by `vv`,
    /// excluding everything past it. Backs server-snapshot production:
    /// the producer must snapshot exactly the server-known frontier
    /// (`server_known_vv`), never its own unsent local operations, or a
    /// bootstrapping device would receive ops the server op stream
    /// doesn't contain. Implemented as a `fork_at` of the frontiers
    /// corresponding to `vv`.
    pub fn snapshot_blob_at(
        &self,
        dek: &Dek,
        vv: &VersionVector,
    ) -> Result<EncryptedBlob, DocError> {
        let frontiers = self.inner.vv_to_frontiers(vv);
        let fork = self.inner.fork_at(&frontiers)?;
        let plaintext = fork.export(ExportMode::Snapshot)?;
        let (ciphertext, nonce) = dek.seal(&plaintext)?;
        Ok(EncryptedBlob {
            nonce: nonce.to_vec(),
            ciphertext,
        })
    }

    /// Mark the local view as "everything currently in oplog is now
    /// durably captured in the WAL." Caller-friendly shortcut for the
    /// common synchronous case (CLI tests). The sync engine uses
    /// `mark_persisted_at` instead so a mutation committed between the
    /// export and the durable append isn't silently included in the
    /// advance.
    pub fn mark_persisted(&mut self) {
        let vv = self.inner.oplog_vv();
        self.last_persisted_vv.merge(&vv);
    }

    /// Encoded current oplog VersionVector. The browser oplog adapter
    /// (`spec/local-storage.md`) keeps the VV captured after the previous
    /// commit and asks for everything strictly after it on the next
    /// commit — that delta is what the oplog row stores.
    pub fn oplog_vv_bytes(&self) -> Vec<u8> {
        self.inner.oplog_vv().encode()
    }

    /// Export Loro updates strictly after `from_vv_bytes`. Returns the
    /// raw plaintext update blob (the JS layer encrypts it before
    /// writing to IndexedDB). Decoupled from `pending_export` because
    /// the oplog frontier is independent of the sync push frontier — a
    /// freshly committed local op needs to land in the oplog before it's
    /// considered durable, regardless of whether the server has it.
    pub fn export_updates_after_bytes(&self, from_vv_bytes: &[u8]) -> Result<Vec<u8>, DocError> {
        // Empty input means "from genesis" — convenient cursor for the
        // first oplog append on a fresh-signup boot, before any
        // `oplog_vv_bytes()` has been captured. (Loro's wire encoding of
        // an empty VV is one byte `[0]`; an empty slice would otherwise
        // fail decode.)
        let vv = if from_vv_bytes.is_empty() {
            VersionVector::default()
        } else {
            VersionVector::decode(from_vv_bytes).map_err(|e| DocError::Loro(e.to_string()))?
        };
        Ok(self.inner.export(ExportMode::updates(&vv))?)
    }

    /// Apply one oplog replay blob without rebuilding disposable indexes.
    /// Boot callers replay every stored blob through this method and call
    /// [`Doc::finish_oplog_replay`] exactly once afterward, avoiding an
    /// O(items × replay_rows) index rebuild.
    ///
    /// Tagged `"remote"` so the per-session UndoManager skips historical
    /// operations; reloading a tab must not resurrect undoable steps.
    pub fn replay_oplog_update(&mut self, plaintext: &[u8]) -> Result<(), DocError> {
        self.inner.import_with(plaintext, "remote")?;
        Ok(())
    }

    /// Finalize a silent boot replay. Rebuilds the disposable item index
    /// once after every snapshot/tail blob has landed and discards any
    /// domain events: historical state is materialized explicitly by the
    /// attaching UI, not presented as live mutations.
    pub fn finish_oplog_replay(&self) {
        // Replayed rows may carry the local leased peer's own history;
        // re-bind the UndoManager at the post-replay frontier so the
        // first local commit's undo span starts here, not at counter 0.
        self.rearm_undo();
        self.rebuild_index();
        if let Ok(mut capture) = self.diff_capture.lock() {
            capture.mode = DiffCaptureMode::None;
            capture.diffs.clear();
        }
        if let Ok(mut events) = self.events.lock() {
            events.clear();
        }
    }

    /// Convenience for a one-blob replay. Multi-row boot paths should use
    /// [`Doc::replay_oplog_update`] + [`Doc::finish_oplog_replay`] instead.
    ///
    /// Does *not* advance `last_persisted_vv`. Whether the original local
    /// commit reached the server before the crash is unknowable from
    /// disk; the next push retries, and Loro / the server dedupe.
    pub fn import_oplog_updates(&mut self, plaintext: &[u8]) -> Result<(), DocError> {
        self.replay_oplog_update(plaintext)?;
        self.finish_oplog_replay();
        Ok(())
    }

    /// Merge `vv` into `last_persisted_vv`. Pair with `oplog_vv()`
    /// captured at the moment of the export — once the exported bytes
    /// are durably appended to the WAL, this advances only past the
    /// ops the append actually covered, leaving any concurrently
    /// committed local mutations in the uncaptured set.
    pub fn mark_persisted_at(&mut self, vv: VersionVector) {
        self.last_persisted_vv.merge(&vv);
    }

    /// Decrypt and apply a peer op blob. Returns the blob's *declared*
    /// operation range (its decoded `partial_end_vv`) — the proof of
    /// which Loro ops the blob carries, independent of whether the
    /// import was a no-op duplicate. The engine merges this into its
    /// `server_known_vv`. `last_persisted_vv` advances by the same
    /// range (the engine WAL-logs the very same blob, and boot replay
    /// feeds stored rows back through this path).
    ///
    /// Translates the import's Loro container diffs (captured by the
    /// root subscription) into per-id `AppEvent`s, so a frame's cost is
    /// proportional to the ops it carries — never total doc size. Any
    /// diff shape the translator can't handle (or a bulk frame touching
    /// ≥ `DIFF_TRANSLATE_MAX_DIRTY` items) rebuilds the disposable index
    /// once and emits one `FullResync` control event.
    pub fn apply_remote(
        &mut self,
        dek: &Dek,
        blob: &EncryptedBlob,
    ) -> Result<VersionVector, DocError> {
        self.apply_remote_batch(dek, std::iter::once(blob))
    }

    /// Batch variant of [`Doc::apply_remote`]. Imports all blobs first,
    /// then translates once so catch-up batches don't pay per-op cost.
    /// Returns the union of every blob's declared operation range.
    pub fn apply_remote_batch<'a, I>(
        &mut self,
        dek: &Dek,
        blobs: I,
    ) -> Result<VersionVector, DocError>
    where
        I: IntoIterator<Item = &'a EncryptedBlob>,
    {
        let pre_lists: Vec<ListView> = self.all_lists();
        let pre_settings = self.get_settings();
        let pre_peer_counter = self.local_peer_counter();
        self.begin_diff_capture(DiffCaptureMode::Import);

        let mut imported_vv = VersionVector::new();
        let result = blobs.into_iter().try_for_each(|blob| {
            let vv = self.import_remote_blob(dek, blob)?;
            imported_vv.merge(&vv);
            Ok::<(), DocError>(())
        });
        let diffs = self.finish_diff_capture();
        // A blob advancing the *local* peer's counter is this device's
        // own history arriving by import (boot replay, reused peer
        // slot); the UndoManager must re-bind past it or the next local
        // commit's undo step swallows it. Applies on the error path too:
        // earlier blobs in the batch may have landed.
        if self.local_peer_counter() != pre_peer_counter {
            self.rearm_undo();
        }
        if let Err(e) = result {
            // A batch may have applied earlier blobs before a later one
            // failed — resync events for whatever landed, then surface
            // the error.
            self.emit_diff_fallback();
            return Err(e);
        }
        if self
            .translate_captured_diffs(diffs, &pre_lists, &pre_settings)
            .is_err()
        {
            self.emit_diff_fallback();
        }
        Ok(imported_vv)
    }

    /// Translate captured import or undo diffs into surgical `AppEvent`s
    /// and incremental index updates. Errors are not failures — they
    /// mean "this frame is beyond the fast path" and the caller falls
    /// back to `emit_diff_fallback`. Phase 1 plans everything against
    /// clones (no state touched), so an abort leaves the index coherent
    /// for the fallback's pre/post reasoning.
    fn translate_captured_diffs(
        &self,
        diffs: Vec<CapturedDiff>,
        pre_lists: &[ListView],
        pre_settings: &SettingsView,
    ) -> Result<(), ()> {
        // ---- Phase 1: plan (read-only) ----
        let mut lists_dirty = false;
        let mut settings_dirty = false;
        let mut focus_dirty = false;
        let mut upserts = HashSet::<String>::new();
        let mut removals = HashSet::<String>::new();
        let mut map_dirty = HashMap::<ContainerID, HashSet<String>>::new();
        // list id → shadow clone with this frame's positional ops applied.
        let mut shadows = HashMap::<String, Vec<Option<OrderEntry>>>::new();
        // list id → item ids whose entries were (re)inserted this frame:
        // the actively-moved candidates for minimal event emission.
        let mut active: HashMap<String, HashSet<String>> = HashMap::new();
        let mut entry_churn = 0usize;
        // Text diffs per item container, in frame order, for the
        // subscribed-editor delta stream.
        let mut text_deltas: HashMap<ContainerID, Vec<Vec<TextDelta>>> = HashMap::new();

        {
            let guard = self.item_index.lock().expect("item index mutex poisoned");
            for diff in diffs {
                match diff {
                    CapturedDiff::Opaque => return Err(()),
                    CapturedDiff::Lists => lists_dirty = true,
                    CapturedDiff::Settings => settings_dirty = true,
                    CapturedDiff::Focus => focus_dirty = true,
                    CapturedDiff::ItemsRoot { upserted, removed } => {
                        upserts.extend(upserted);
                        removals.extend(removed);
                    }
                    CapturedDiff::ItemMap { container, keys } => {
                        map_dirty.entry(container).or_default().extend(keys);
                    }
                    CapturedDiff::ItemText {
                        container,
                        key,
                        delta,
                    } => {
                        map_dirty.entry(container.clone()).or_default().insert(key);
                        text_deltas.entry(container).or_default().push(delta);
                    }
                    CapturedDiff::Order { list_id, ops } => {
                        let shadow = shadows.entry(list_id.clone()).or_insert_with(|| {
                            guard.raw_orders.get(&list_id).cloned().unwrap_or_default()
                        });
                        let mut pos = 0usize;
                        for op in ops {
                            match op {
                                CapturedListItem::Retain(n) => {
                                    pos = pos.checked_add(n).ok_or(())?;
                                    if pos > shadow.len() {
                                        return Err(());
                                    }
                                }
                                CapturedListItem::Delete(n) => {
                                    if pos + n > shadow.len() {
                                        return Err(());
                                    }
                                    entry_churn += n;
                                    shadow.drain(pos..pos + n);
                                }
                                CapturedListItem::Insert(vals) => {
                                    for v in vals {
                                        let entry = v.as_deref().and_then(OrderEntry::parse);
                                        if let Some(e) = &entry {
                                            active
                                                .entry(list_id.clone())
                                                .or_default()
                                                .insert(e.item_id.clone());
                                        }
                                        shadow.insert(pos.min(shadow.len()), entry);
                                        pos += 1;
                                        entry_churn += 1;
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
        // A key both deleted and re-set within the frame coalesces to
        // its final state in the map diff; treat re-adds as upserts.
        removals.retain(|id| !upserts.contains(id));

        // The shadows must land exactly on the post-import containers,
        // or our positional reasoning was wrong somewhere — resync.
        for (list_id, shadow) in &shadows {
            if shadow.len() != self.order_list(list_id).len() {
                return Err(());
            }
        }

        // Resolve map-dirty containers to item ids. Upserted ids get
        // full-state events anyway; deleted containers are fine iff the
        // frame also removed them.
        let mut dirty_keys_by_id = HashMap::<String, HashSet<String>>::new();
        let mut text_deltas_by_id = HashMap::<String, Vec<Vec<TextDelta>>>::new();
        for (cid, keys) in map_dirty {
            let map = self.inner.get_map(cid.clone());
            let Some(id) = read_string(&map, KEY_ID) else {
                continue;
            };
            if removals.contains(&id) || upserts.contains(&id) {
                continue;
            }
            if let Some(deltas) = text_deltas.remove(&cid) {
                text_deltas_by_id.insert(id.clone(), deltas);
            }
            dirty_keys_by_id.entry(id).or_default().extend(keys);
        }

        if upserts.len() + removals.len() + dirty_keys_by_id.len() + entry_churn
            >= DIFF_TRANSLATE_MAX_DIRTY
        {
            return Err(());
        }

        // Per-item change plan: old meta from the index, new meta from
        // the post-import doc.
        struct Change {
            old: Option<ItemMeta>,
            new: Option<ItemMeta>,
            keys: HashSet<String>,
            brand_new: bool,
        }
        let mut changes = HashMap::<String, Change>::new();
        {
            let guard = self.item_index.lock().expect("item index mutex poisoned");
            let items = self.items();
            for id in &upserts {
                let Some(map) = item_map_of(&items, id) else {
                    return Err(());
                };
                let old = guard.meta.get(id).cloned();
                let brand_new = old.is_none();
                // A re-set of an existing container (undo/redo shapes)
                // is translated as "everything may have changed".
                let keys: HashSet<String> = if brand_new {
                    HashSet::new()
                } else {
                    [
                        KEY_TEXT,
                        KEY_NOTES,
                        KEY_DEADLINE,
                        KEY_WHEN,
                        KEY_DURATION,
                        KEY_LIFECYCLE,
                        KEY_STARTED_AT,
                        KEY_DONE_AT,
                        KEY_BINNED_AT,
                        KEY_LOCATION,
                    ]
                    .into_iter()
                    .map(str::to_string)
                    .collect()
                };
                changes.insert(
                    id.clone(),
                    Change {
                        old,
                        new: Some(item_meta(&map)),
                        keys,
                        brand_new,
                    },
                );
            }
            for (id, keys) in dirty_keys_by_id {
                let Some(map) = item_map_of(&items, &id) else {
                    return Err(());
                };
                changes.insert(
                    id.clone(),
                    Change {
                        old: guard.meta.get(&id).cloned(),
                        new: Some(item_meta(&map)),
                        keys,
                        brand_new: false,
                    },
                );
            }
            for id in &removals {
                changes.insert(
                    id.clone(),
                    Change {
                        old: guard.meta.get(id).cloned(),
                        new: None,
                        keys: HashSet::new(),
                        brand_new: false,
                    },
                );
            }
        }

        let mut affected: HashSet<String> = shadows.keys().cloned().collect();
        for c in changes.values() {
            if let Some(o) = &c.old {
                affected.insert(o.list_id.clone());
            }
            if let Some(n) = &c.new {
                affected.insert(n.list_id.clone());
            }
        }

        // ---- Phase 2: commit index ----
        let (pre_open, post_open) = {
            let mut guard = self.item_index.lock().expect("item index mutex poisoned");
            let pre: HashMap<String, Vec<String>> = affected
                .iter()
                .map(|l| {
                    (
                        l.clone(),
                        guard.open_by_list.get(l).cloned().unwrap_or_default(),
                    )
                })
                .collect();
            for (id, c) in &changes {
                match &c.new {
                    Some(m) => guard.set_meta(id, m.clone()),
                    None => guard.remove_item(id),
                }
            }
            for (l, shadow) in shadows {
                guard.raw_orders.insert(l, shadow);
            }
            let mut post = HashMap::new();
            for l in &affected {
                post.insert(l.clone(), guard.refresh_open(l));
            }
            (pre, post)
        };

        // ---- Phase 3: events ----
        let view_of = |id: &str| -> Result<ItemView, ()> {
            let map = item_map_of(&self.items(), id).ok_or(())?;
            item_view(&map).ok_or(())
        };

        let mut removals_sorted: Vec<&String> = removals.iter().collect();
        removals_sorted.sort();
        for id in removals_sorted {
            self.push_event(AppEvent::ItemRemoved { id: id.clone() });
        }

        // Content + hidden-side events, in id order so event sequences
        // are deterministic. Live-side positional events are emitted by
        // the per-list walk below.
        let mut change_ids: Vec<&String> = changes.keys().collect();
        change_ids.sort();
        for id in change_ids {
            let c = &changes[id];
            let Some(new) = &c.new else { continue };
            let has = |k: &str| c.keys.contains(k);
            if !c.brand_new {
                let view = view_of(id)?;
                if has(KEY_TEXT) {
                    self.push_event(AppEvent::ItemTextChanged {
                        id: id.clone(),
                        text: view.text.clone(),
                    });
                }
                if has(KEY_NOTES) {
                    self.push_event(AppEvent::ItemNotesChanged {
                        id: id.clone(),
                        notes: view.notes.clone(),
                    });
                    let deltas = text_deltas_by_id.get(id).map(Vec::as_slice).unwrap_or(&[]);
                    self.emit_notes_delta(id, deltas, &view.notes);
                }
                if has(KEY_DEADLINE) {
                    self.push_event(AppEvent::ItemDeadlineChanged {
                        id: id.clone(),
                        deadline: view.deadline.clone(),
                    });
                }
                if has(KEY_WHEN) {
                    self.push_event(AppEvent::ItemWhenChanged {
                        id: id.clone(),
                        when: view.when.clone(),
                    });
                }
                if has(KEY_DURATION) {
                    self.push_event(AppEvent::ItemDurationChanged {
                        id: id.clone(),
                        duration: view.duration,
                    });
                }
                // An open→open workflow flip (the register alone, e.g.
                // Backlog → In Progress) changes the item's lane but not
                // its order, so the per-list walk emits nothing for it —
                // surface the lifecycle here. Hidden↔open transitions are
                // emitted by the walk / hidden paths instead.
                let was_open = c.old.as_ref().is_some_and(|o| o.open);
                if new.open
                    && was_open
                    && (has(KEY_LIFECYCLE)
                        || has(KEY_STARTED_AT)
                        || has(KEY_DONE_AT)
                        || has(KEY_BINNED_AT))
                {
                    let open_index = post_open
                        .get(&new.list_id)
                        .and_then(|arr| arr.iter().position(|x| x == id));
                    self.push_event(AppEvent::ItemLifecycleChanged {
                        id: id.clone(),
                        state: view.state,
                        lifecycle_at: view.lifecycle_at,
                        started_at: view.started_at,
                        done_at: view.done_at,
                        binned_at: view.binned_at,
                        open_index,
                    });
                }
            }
            let left_list = c.old.as_ref().is_some_and(|o| o.list_id != new.list_id);
            if new.open {
                // Live cross-list movers get their leave-signal *before*
                // any per-list walk: the consumer removes the item from
                // the source array now (appending it to the target), so
                // walks over the source list never reposition around a
                // ghost. The target walk repositions the item via an
                // ordinary `ItemMoved` candidate.
                if !c.brand_new && has(KEY_LOCATION) && left_list {
                    self.push_event(AppEvent::ItemListChanged {
                        id: id.clone(),
                        list_id: new.list_id.clone(),
                        open_index: None,
                    });
                }
                continue;
            }
            let view = view_of(id)?;
            if c.brand_new {
                self.push_event(AppEvent::ItemAdded {
                    id: view.id,
                    list_id: view.list_id,
                    text: view.text,
                    notes: view.notes,
                    created_at: view.created_at,
                    state: view.state,
                    lifecycle_at: view.lifecycle_at,
                    started_at: view.started_at,
                    done_at: view.done_at,
                    binned_at: view.binned_at,
                    deadline: view.deadline,
                    when: view.when,
                    duration: view.duration,
                    open_index: None,
                });
                continue;
            }
            if has(KEY_LIFECYCLE) || has(KEY_STARTED_AT) || has(KEY_DONE_AT) || has(KEY_BINNED_AT) {
                self.push_event(AppEvent::ItemLifecycleChanged {
                    id: id.clone(),
                    state: view.state,
                    lifecycle_at: view.lifecycle_at,
                    started_at: view.started_at,
                    done_at: view.done_at,
                    binned_at: view.binned_at,
                    open_index: None,
                });
            }
            if has(KEY_LOCATION) && left_list {
                self.push_event(AppEvent::ItemListChanged {
                    id: id.clone(),
                    list_id: new.list_id.clone(),
                    open_index: None,
                });
            }
        }

        // Per-list open walk: minimal ascending remove+insert event
        // plan, verified by replaying it the way a naive consumer does.
        let mut affected_sorted: Vec<&String> = affected.iter().collect();
        affected_sorted.sort();
        let mut planned: Vec<(usize, String, AppEvent)> = Vec::new();
        for list in affected_sorted {
            let pre = &pre_open[list.as_str()];
            let post = &post_open[list.as_str()];
            let post_set: HashSet<&String> = post.iter().collect();
            let base: Vec<&String> = pre.iter().filter(|id| post_set.contains(id)).collect();
            let post_pos: HashMap<&String, usize> =
                post.iter().enumerate().map(|(i, id)| (id, i)).collect();

            // Insert-type ids: not in the consumer's array yet; their
            // typed event both inserts and positions them.
            let mut insert_type: HashMap<&String, AppEvent> = HashMap::new();
            for (i, id) in post.iter().enumerate() {
                let Some(c) = changes.get(id) else { continue };
                let Some(new) = &c.new else { continue };
                if !new.open || new.list_id != *list {
                    continue;
                }
                if c.brand_new {
                    let view = view_of(id)?;
                    insert_type.insert(
                        id,
                        AppEvent::ItemAdded {
                            id: view.id,
                            list_id: view.list_id,
                            text: view.text,
                            notes: view.notes,
                            created_at: view.created_at,
                            state: view.state,
                            lifecycle_at: view.lifecycle_at,
                            started_at: view.started_at,
                            done_at: view.done_at,
                            binned_at: view.binned_at,
                            deadline: view.deadline,
                            when: view.when,
                            duration: view.duration,
                            open_index: Some(i),
                        },
                    );
                    continue;
                }
                // Hidden→open restores insert here. Cross-list movers
                // are *not* insert-type: their pre-walk leave-signal
                // `ItemListChanged` already appended them to this list
                // on the consumer, so an `ItemMoved` candidate
                // repositions them like any other id.
                let shown = c.old.as_ref().is_some_and(|o| !o.open);
                if shown && (c.keys.contains(KEY_LIFECYCLE) || c.keys.contains(KEY_BINNED_AT)) {
                    let view = view_of(id)?;
                    insert_type.insert(
                        id,
                        AppEvent::ItemLifecycleChanged {
                            id: id.clone(),
                            state: view.state,
                            lifecycle_at: view.lifecycle_at,
                            started_at: view.started_at,
                            done_at: view.done_at,
                            binned_at: view.binned_at,
                            open_index: Some(i),
                        },
                    );
                }
            }

            // Candidate move set: start minimal (this frame's actively
            // re-inserted entries), widen to every position-changed id
            // if the minimal replay doesn't reconstruct the post state.
            // The simulation models the consumer exactly: one
            // remove-then-insert-at-open_index per event, applied
            // sequentially in emission (ascending post) order.
            let simulate = |cands: &HashSet<&String>| -> bool {
                let mut arr: Vec<&String> = base
                    .iter()
                    .filter(|id| !insert_type.contains_key(**id))
                    .copied()
                    .collect();
                let mut ops: Vec<(usize, &String)> = insert_type
                    .keys()
                    .copied()
                    .chain(cands.iter().copied())
                    .filter_map(|id| post_pos.get(id).map(|p| (*p, id)))
                    .collect();
                ops.sort_unstable();
                ops.dedup();
                for (p, id) in ops {
                    arr.retain(|x| *x != id);
                    arr.insert(p.min(arr.len()), id);
                }
                arr.len() == post.len() && arr.iter().zip(post.iter()).all(|(a, b)| *a == b)
            };

            let minimal: HashSet<&String> = active
                .get(list.as_str())
                .into_iter()
                .flatten()
                .filter(|id| post_set.contains(*id) && !insert_type.contains_key(*id))
                .collect();
            let candidates = if simulate(&minimal) {
                minimal
            } else {
                // Widen to every id whose surviving-relative position
                // changed; ascending remove+insert of that whole set is
                // the proven reconstruction. Bail to FullResync when
                // the event volume would exceed the surgical budget or
                // the replay still doesn't line up.
                let base_pos: HashMap<&String, usize> =
                    base.iter().enumerate().map(|(i, id)| (*id, i)).collect();
                let mut widened = HashSet::new();
                for (i, id) in post.iter().enumerate() {
                    if insert_type.contains_key(id) {
                        continue;
                    }
                    if base_pos.get(id).copied() != Some(i) {
                        widened.insert(id);
                    }
                }
                if widened.len() + planned.len() >= DIFF_TRANSLATE_MAX_DIRTY {
                    return Err(());
                }
                if !simulate(&widened) {
                    return Err(());
                }
                widened
            };

            for (i, id) in post.iter().enumerate() {
                if let Some(ev) = insert_type.remove(id) {
                    planned.push((i, list.clone(), ev));
                } else if candidates.contains(id) {
                    planned.push((
                        i,
                        list.clone(),
                        AppEvent::ItemMoved {
                            id: id.clone(),
                            open_index: Some(i),
                        },
                    ));
                }
            }
        }
        if planned.len() + removals.len() >= DIFF_TRANSLATE_MAX_DIRTY {
            return Err(());
        }
        for (_, _, ev) in planned {
            self.push_event(ev);
        }

        if lists_dirty || settings_dirty {
            let mut emitted = Vec::new();
            if settings_dirty {
                diff_settings(pre_settings, &self.get_settings(), &mut emitted);
            }
            if lists_dirty {
                diff_lists(pre_lists, &self.all_lists(), &mut emitted);
            }
            for ev in emitted {
                self.push_event(ev);
            }
        }

        if focus_dirty {
            self.push_event(AppEvent::FocusChanged);
        }
        Ok(())
    }

    /// Whole-doc fallback for frames the translator declined. Rebuild the
    /// disposable index, then emit one control signal. Consumers fetch the
    /// current snapshot once; they are never flooded with N synthetic adds.
    fn emit_diff_fallback(&self) {
        self.rebuild_index();
        // Subscribed editors re-subscribe on `FullResync` (which resets
        // their shadow); refresh here too so a consumer that only
        // re-reads the string still converts later deltas correctly.
        {
            let items = self.items();
            let mut shadows = self
                .notes_shadows
                .lock()
                .expect("notes shadows mutex poisoned");
            for (id, shadow) in shadows.iter_mut() {
                if let Some(map) = item_map_of(&items, id) {
                    *shadow = read_text(&map, KEY_NOTES).unwrap_or_default();
                }
            }
        }
        let mut events = self.events.lock().expect("events mutex poisoned");
        events.clear();
        events.push_back(AppEvent::FullResync);
    }

    // ---------- undo / redo ----------

    /// Undo the most recent eligible local commit. Remote-applied ops
    /// are filtered out by origin prefix and never enter the stack.
    /// Returns `true` if a step was applied. Loro's emitted container
    /// diffs are translated into surgical `AppEvent`s; unusual or bulk
    /// shapes retain the whole-document correctness fallback.
    pub fn undo(&self) -> Result<bool, DocError> {
        self.apply_undo_op(|um| um.undo())
    }

    /// Redo the most recently undone step.
    pub fn redo(&self) -> Result<bool, DocError> {
        self.apply_undo_op(|um| um.redo())
    }

    pub fn can_undo(&self) -> bool {
        self.undo.lock().map(|um| um.can_undo()).unwrap_or(false)
    }

    pub fn can_redo(&self) -> bool {
        self.undo.lock().map(|um| um.can_redo()).unwrap_or(false)
    }

    fn apply_undo_op<F>(&self, op: F) -> Result<bool, DocError>
    where
        F: FnOnce(&mut UndoManager) -> loro::LoroResult<bool>,
    {
        let pre_lists: Vec<ListView> = self.all_lists();
        let pre_settings = self.get_settings();
        self.begin_diff_capture(DiffCaptureMode::Undo);
        let result = {
            let mut um = self.undo.lock().expect("undo mutex poisoned");
            op(&mut um)
        };
        let diffs = self.finish_diff_capture();
        let did = result?;
        if did
            && self
                .translate_captured_diffs(diffs, &pre_lists, &pre_settings)
                .is_err()
        {
            self.emit_diff_fallback();
        }
        Ok(did)
    }

    // ---------- event queue ----------

    /// Pop the next domain event, FIFO. UI consumers drain this on each
    /// engine tick. See `snapshot_events` for the synthetic backfill
    /// emitted on first attach.
    pub fn pop_event(&self) -> Option<AppEvent> {
        self.events.lock().ok()?.pop_front()
    }

    /// Drain everything currently queued.
    pub fn drain_events(&self) -> Vec<AppEvent> {
        self.events
            .lock()
            .map(|mut q| q.drain(..).collect())
            .unwrap_or_default()
    }

    /// Synthetic event burst for current state. A fresh consumer calls
    /// this once on attach to materialize lists + items via the same
    /// dispatcher it uses for live deltas — no separate "load initial"
    /// code path.
    pub fn snapshot_events(&self) -> Vec<AppEvent> {
        let mut out = Vec::new();
        let s = self.get_settings();
        out.push(AppEvent::SettingsChanged {
            show_list_counts: s.show_list_counts,
            inbox_view: s.inbox_view,
        });
        let lists = self.all_lists();
        for (idx, list) in lists.iter().enumerate() {
            out.push(AppEvent::ListAdded {
                id: list.id.clone(),
                name: list.name.clone(),
                created_at: list.created_at,
                archived_at: list.archived_at,
                index: idx,
            });
        }
        // Walking the canonical grouped order means a per-list counter
        // yields each open item's position in its list's open projection.
        let mut open_counters = HashMap::<String, usize>::new();
        for item in self.iter_items() {
            let open_index = if item.is_open() {
                let counter = open_counters.entry(item.list_id.clone()).or_insert(0);
                let i = *counter;
                *counter += 1;
                Some(i)
            } else {
                None
            };
            out.push(AppEvent::ItemAdded {
                id: item.id,
                list_id: item.list_id,
                text: item.text,
                notes: item.notes,
                created_at: item.created_at,
                state: item.state,
                lifecycle_at: item.lifecycle_at,
                started_at: item.started_at,
                done_at: item.done_at,
                binned_at: item.binned_at,
                deadline: item.deadline,
                when: item.when,
                duration: item.duration,
                open_index,
            });
        }
        out
    }

    fn push_event(&self, ev: AppEvent) {
        if let Ok(mut q) = self.events.lock() {
            q.push_back(ev);
        }
    }

    // ---------- persistence ----------

    /// Serialize doc state + last-pushed VV. Encoding is msgpack —
    /// small, additively evolvable, and matches the wire format used
    /// everywhere else.
    pub fn save(&self) -> Result<Vec<u8>, DocError> {
        let snapshot = self.inner.export(ExportMode::Snapshot)?;
        let envelope = LocalState {
            version: 1,
            snapshot,
            last_persisted_vv: self.last_persisted_vv.encode(),
        };
        Ok(rmp_serde::to_vec_named(&envelope)?)
    }

    pub fn load(bytes: &[u8]) -> Result<Self, DocError> {
        let envelope: LocalState = rmp_serde::from_slice(bytes)?;
        let inner = LoroDoc::new();
        inner.import(&envelope.snapshot)?;
        let last_persisted_vv = VersionVector::decode(&envelope.last_persisted_vv)
            .map_err(|e| DocError::Loro(e.to_string()))?;
        let undo = Mutex::new(make_undo_manager(&inner));
        let item_index = Mutex::new(ProjectionIndex::default());
        // Subscribe *after* the snapshot import so the boot state
        // doesn't land in the gated diff buffer as one giant frame.
        let diff_capture = Arc::new(Mutex::new(DiffCapture::default()));
        let _diff_sub = inner.subscribe_root(make_diff_subscriber(diff_capture.clone()));
        let doc = Self {
            inner,
            last_persisted_vv,
            events: Mutex::new(VecDeque::new()),
            item_index,
            undo,
            diff_capture,
            notes_shadows: Mutex::new(HashMap::new()),
            _diff_sub,
        };
        doc.rebuild_index();
        Ok(doc)
    }

    // ---------- fingerprint ----------

    /// Logical-state hash. Stable across replicas at logical equality;
    /// used for convergence assertions in tests. Snapshot bytes are
    /// *not* stable (Loro stores per-replica metadata), so we hash a
    /// canonical serialization of the visible item / list state. Items
    /// are hashed in the canonical grouped walk (`all_items`) so each
    /// list's resolved order — including hidden items' restore
    /// positions — is part of the hash.
    pub fn fingerprint(&self) -> [u8; 32] {
        let mut hasher = Sha256::new();
        let settings = self.get_settings();
        hasher.update(b"S");
        hasher.update([settings.show_list_counts as u8]);
        // Lists: walk by stored order because ordering is part of the
        // logical state surfaced to users.
        let lists = self.all_lists();
        hasher.update(b"L");
        hasher.update((lists.len() as u32).to_be_bytes());
        for l in &lists {
            hash_str(&mut hasher, &l.id);
            hash_str(&mut hasher, &l.name);
            // Archive state is logical state (`spec/data-model.md`
            // "Archived lists") — replicas diverging on it must not
            // fingerprint equal.
            hash_opt_i64(&mut hasher, l.archived_at);
            hasher.update(l.created_at.to_be_bytes());
        }
        // Items: canonical grouped walk order.
        let items: Vec<ItemView> = self.iter_items().collect();
        hasher.update(b"I");
        hasher.update((items.len() as u32).to_be_bytes());
        for i in &items {
            hash_str(&mut hasher, &i.id);
            hash_str(&mut hasher, &i.text);
            hash_str(&mut hasher, &i.notes);
            hash_str(&mut hasher, &i.list_id);
            // The resolved workflow register (state + at), the bin mask,
            // and the reflection stamps are all logical state
            // (`spec/data-model.md`).
            hasher.update([i.state as u8]);
            hasher.update(i.lifecycle_at.to_be_bytes());
            hash_opt_str(&mut hasher, i.deadline.as_deref());
            hash_opt_str(&mut hasher, i.when.as_deref());
            hash_opt_i64(&mut hasher, i.duration.map(i64::from));
            hasher.update(i.created_at.to_be_bytes());
            hash_opt_i64(&mut hasher, i.started_at);
            hash_opt_i64(&mut hasher, i.done_at);
            hash_opt_i64(&mut hasher, i.binned_at);
        }
        // Focus: the curated order fixes logical state (`spec/focus.md`),
        // exactly as each list's resolved order does. Hash the raw
        // container contents in order so converged replicas match and a
        // reordered / diverged Focus is caught.
        let focus = self.focus_raw();
        hasher.update(b"F");
        hasher.update((focus.len() as u32).to_be_bytes());
        for s in &focus {
            hash_str(&mut hasher, s);
        }
        hasher.finalize().into()
    }

    // ---------- private ----------

    fn items(&self) -> LoroMap {
        self.inner.get_map(ROOT_ITEMS)
    }

    fn lists(&self) -> LoroMovableList {
        self.inner.get_movable_list(ROOT_LISTS)
    }

    fn settings_map(&self) -> LoroMap {
        self.inner.get_map(ROOT_SETTINGS)
    }

    fn order_list(&self, list_id: &str) -> LoroMovableList {
        self.inner
            .get_movable_list(order_root_name(list_id).as_str())
    }

    fn focus_list(&self) -> LoroMovableList {
        self.inner.get_movable_list(FOCUS_CONTAINER)
    }

    /// Raw encoded elements of the `focus` container in order. A
    /// non-string slot (never emitted) surfaces as an unparseable dead
    /// ref rather than aborting the read.
    fn focus_raw(&self) -> Vec<String> {
        let list = self.focus_list();
        (0..list.len())
            .map(|i| match list.get(i) {
                Some(ValueOrContainer::Value(v)) => v.into_string().ok().map(|s| s.to_string()),
                _ => None,
            })
            .map(|s| s.unwrap_or_default())
            .collect()
    }

    /// Classify the focus container against the item index. Locks the
    /// index once; a ref is *visible* iff it parses, is local, names an
    /// item the index knows to be Open, and is the first ref for that
    /// item. Everything else is dead garbage to sweep.
    fn scan_focus(&self) -> FocusScan {
        let raw = self.focus_raw();
        let guard = self.item_index.lock().expect("item index mutex poisoned");
        let mut seen: HashSet<String> = HashSet::new();
        let mut visible_item_ids = Vec::new();
        let mut dead_idx = Vec::new();
        for (i, s) in raw.iter().enumerate() {
            let item_id = match FocusRef::parse(s) {
                Some(r)
                    if r.is_local()
                        && guard.meta.get(&r.item_id).is_some_and(|m| m.open)
                        && seen.insert(r.item_id.clone()) =>
                {
                    Some(r.item_id)
                }
                _ => None,
            };
            match item_id {
                Some(id) => visible_item_ids.push(id),
                None => dead_idx.push(i),
            }
        }
        FocusScan {
            raw,
            visible_item_ids,
            dead_idx,
        }
    }

    /// Delete, in one uncommitted batch, every focus ref whose item id is
    /// in `remove_items` **plus** all dead refs (the folded sweep).
    /// Returns the number of slots removed; the caller commits.
    fn prune_focus_refs(&self, remove_items: &HashSet<String>) -> usize {
        let scan = self.scan_focus();
        let mut to_delete: Vec<usize> = scan.dead_idx.clone();
        for (i, s) in scan.raw.iter().enumerate() {
            if let Some(r) = FocusRef::parse(s)
                && r.is_local()
                && remove_items.contains(&r.item_id)
            {
                to_delete.push(i);
            }
        }
        to_delete.sort_unstable();
        to_delete.dedup();
        if to_delete.is_empty() {
            return 0;
        }
        let focus = self.focus_list();
        let mut removed = 0;
        for &i in to_delete.iter().rev() {
            if focus.delete(i, 1).is_ok() {
                removed += 1;
            }
        }
        removed
    }

    /// Item container lookup, gated on the disposable index so a doc
    /// mid-boot-replay reports "not found" until `finish_oplog_replay`
    /// materializes state (deliberate: see the deferred-replay test).
    fn find_item(&self, item_id: &str) -> Result<LoroMap, DocError> {
        {
            let guard = self.item_index.lock().expect("item index mutex poisoned");
            if !guard.meta.contains_key(item_id) {
                return Err(DocError::ItemNotFound(item_id.to_string()));
            }
        }
        item_map_of(&self.items(), item_id)
            .ok_or_else(|| DocError::ItemNotFound(item_id.to_string()))
    }

    fn find_list(&self, list_id: &str) -> Result<(usize, LoroMap), DocError> {
        let lists = self.lists();
        for i in 0..lists.len() {
            if let Some(map) = list_map_at(&lists, i)
                && read_string(&map, KEY_ID).as_deref() == Some(list_id)
            {
                return Ok((i, map));
            }
        }
        Err(DocError::ListNotFound(list_id.into()))
    }

    fn visible_list_index(&self, list_id: &str) -> Option<usize> {
        self.all_lists().iter().position(|l| l.id == list_id)
    }

    /// Resolve an active-projection `target_index` for the active list
    /// `list_id` to the raw index in the `lists` container. The mover is
    /// removed from the sequence, the slot is found among the remaining
    /// **active** rows, and the mover lands immediately before the row
    /// at that active position (or right after the last active row when
    /// past-end) — so archived rows interspersed in the container never
    /// skew a drop position computed against the active-only nav.
    fn resolve_active_move_target(
        &self,
        list_id: &str,
        target_index: usize,
    ) -> Result<usize, DocError> {
        let lists = self.lists();
        // Active flag per remaining row (mover excluded), in raw order.
        // Unparseable rows count as occupied-but-inactive slots.
        let mut rest: Vec<bool> = Vec::new();
        for i in 0..lists.len() {
            let Some(map) = list_map_at(&lists, i) else {
                rest.push(false);
                continue;
            };
            if read_string(&map, KEY_ID).as_deref() == Some(list_id) {
                continue;
            }
            rest.push(read_i64(&map, KEY_ARCHIVED_AT).is_none());
        }
        let mut active_seen = 0usize;
        let mut after_last_active = 0usize;
        for (pos, active) in rest.iter().enumerate() {
            if *active {
                if active_seen == target_index {
                    return Ok(pos);
                }
                active_seen += 1;
                after_last_active = pos + 1;
            }
        }
        Ok(after_last_active)
    }

    fn assert_list_exists(&self, list_id: &str) -> Result<(), DocError> {
        if list_id == LIST_INBOX {
            return Ok(());
        }
        self.find_list(list_id).map(|_| ())
    }

    fn emit_item_diffs(&self, pre_items: &[ItemView]) {
        let post_items: Vec<ItemView> = self.iter_items().collect();
        let mut emitted = Vec::new();
        diff_items(pre_items, &post_items, &mut emitted);
        self.emit_notes_deltas_for(&emitted);
        if !emitted.is_empty() {
            let mut q = self.events.lock().expect("events mutex poisoned");
            for ev in emitted {
                q.push_back(ev);
            }
        }
    }

    fn import_remote_blob(
        &mut self,
        dek: &Dek,
        blob: &EncryptedBlob,
    ) -> Result<VersionVector, DocError> {
        if blob.nonce.len() != AEAD_NONCE_LEN {
            return Err(DocError::Invalid(format!(
                "expected {AEAD_NONCE_LEN}-byte nonce, got {}",
                blob.nonce.len()
            )));
        }
        let plaintext = dek.open(&blob.ciphertext, &blob.nonce)?;
        // The blob's *declared* range, not `ImportStatus.success`: a
        // duplicate import is a no-op success-wise but still proves the
        // sender possesses those ops — exactly what `server_known_vv`
        // needs. (`partial_end_vv` covers only peers present in the
        // blob, which is the correct merge shape.) Checksum already
        // guaranteed by the AEAD open above.
        let meta = LoroDoc::decode_import_blob_meta(&plaintext, false)
            .map_err(|e| DocError::Loro(e.to_string()))?;
        self.inner.import_with(&plaintext, "remote")?;
        self.last_persisted_vv.merge(&meta.partial_end_vv);
        Ok(meta.partial_end_vv)
    }

    fn emit_state_diff(
        &self,
        pre_items: &[ItemView],
        pre_lists: &[ListView],
        pre_settings: &SettingsView,
    ) {
        let post_items: Vec<ItemView> = self.iter_items().collect();
        let post_lists: Vec<ListView> = self.all_lists();
        let post_settings = self.get_settings();
        let mut emitted = Vec::new();
        diff_settings(pre_settings, &post_settings, &mut emitted);
        diff_lists(pre_lists, &post_lists, &mut emitted);
        diff_items(pre_items, &post_items, &mut emitted);
        self.emit_notes_deltas_for(&emitted);
        if !emitted.is_empty() {
            let mut q = self.events.lock().expect("events mutex poisoned");
            for ev in emitted {
                q.push_back(ev);
            }
        }
    }
}

#[derive(Serialize, Deserialize)]
struct LocalState {
    version: u8,
    #[serde(with = "serde_bytes")]
    snapshot: Vec<u8>,
    #[serde(with = "serde_bytes")]
    last_persisted_vv: Vec<u8>,
}

fn assert_unique_item_ids(item_ids: &[&str]) -> Result<(), DocError> {
    let mut seen = HashSet::with_capacity(item_ids.len());
    for item_id in item_ids {
        if !seen.insert(*item_id) {
            return Err(DocError::Invalid(format!("duplicate item id: {item_id}")));
        }
    }
    Ok(())
}

fn seed_builtins(doc: &LoroDoc) -> Result<bool, DocError> {
    // `LIST_INBOX` is a *reserved id*, not a ListMeta row — items
    // reference it as the string "inbox" and clients render its label
    // client-side. Touch the root containers so fresh docs keep the
    // same top-level shape; no ops are emitted for untouched roots.
    //
    // Starter content (e.g. a "Welcome" list) is deliberately NOT seeded
    // here: genesis must stay a clean empty baseline so a second replica
    // constructed the same way doesn't mint a duplicate. The
    // account-creating *client* seeds via the public API right after
    // create (see the web boot in App.tsx / the CLI init path); other
    // devices receive it through sync.
    let _ = doc.get_map(ROOT_ITEMS);
    let _ = doc.get_movable_list(ROOT_LISTS);
    let _ = doc.get_map(ROOT_SETTINGS);
    let _ = doc.get_movable_list(order_root_name(LIST_INBOX).as_str());
    Ok(false)
}

fn item_map_of(items: &LoroMap, item_id: &str) -> Option<LoroMap> {
    match items.get(item_id)? {
        ValueOrContainer::Container(Container::Map(m)) => Some(m),
        _ => None,
    }
}

fn list_map_at(lists: &LoroMovableList, idx: usize) -> Option<LoroMap> {
    match lists.get(idx)? {
        ValueOrContainer::Container(Container::Map(m)) => Some(m),
        _ => None,
    }
}

fn scalar_entry_at(order: &LoroMovableList, idx: usize) -> Option<OrderEntry> {
    match order.get(idx)? {
        ValueOrContainer::Value(v) => v
            .into_string()
            .ok()
            .and_then(|s| OrderEntry::parse(s.as_ref())),
        _ => None,
    }
}

fn read_string(map: &LoroMap, key: &str) -> Option<String> {
    let v = map.get(key)?;
    let value = v.as_value()?.clone();
    value.into_string().ok().map(|s| s.to_string())
}

/// Read a text container's content. Anything other than a text container
/// at the key yields `None`, like an absent key.
fn read_text(map: &LoroMap, key: &str) -> Option<String> {
    match map.get(key)? {
        ValueOrContainer::Container(Container::Text(t)) => Some(t.to_string()),
        _ => None,
    }
}

/// Make `text` equal `target` with a minimal character diff. The default
/// options have no timeout, so the timeout error arm cannot fire; it is
/// mapped to `Invalid` for completeness.
fn update_text(text: &LoroText, target: &str) -> Result<(), DocError> {
    text.update(target, UpdateOptions::default())
        .map_err(|e| DocError::Invalid(e.to_string()))
}

/// Check a UTF-16 delta against the text it will be applied to: every
/// retain / delete stays within the text and every position it lands on
/// is a scalar boundary (never inside a surrogate pair). Inserts are
/// only counted. Nothing is written here, so a bad delta is rejected
/// whole rather than half-applied.
fn validate_utf16_delta(current: &str, delta: &[NotesDeltaOp]) -> Result<(), DocError> {
    // UTF-16 offsets that begin a scalar, plus the end offset.
    let mut boundaries = HashSet::new();
    let mut total = 0usize;
    for c in current.chars() {
        boundaries.insert(total);
        total += c.len_utf16();
    }
    boundaries.insert(total);
    let bad = |what: &str| DocError::Invalid(format!("notes delta: {what}"));
    let mut pos = 0usize;
    for op in delta {
        match op {
            NotesDeltaOp::Retain { retain } => {
                pos = pos.checked_add(*retain).ok_or_else(|| bad("overflow"))?;
                if pos > total {
                    return Err(bad("retain past end"));
                }
            }
            NotesDeltaOp::Delete { delete } => {
                let end = pos.checked_add(*delete).ok_or_else(|| bad("overflow"))?;
                if end > total {
                    return Err(bad("delete past end"));
                }
                if !boundaries.contains(&pos) || !boundaries.contains(&end) {
                    return Err(bad("delete splits a surrogate pair"));
                }
                // The text shrinks, but so does every later offset; keep
                // validating against the original coordinates by
                // shifting the boundary set is overkill for short notes:
                // rebuild the remaining boundaries instead.
                let removed = end - pos;
                boundaries = boundaries
                    .into_iter()
                    .filter(|b| *b < pos || *b >= end)
                    .map(|b| if b >= end { b - removed } else { b })
                    .collect();
                total -= removed;
            }
            NotesDeltaOp::Insert { insert } => {
                if !boundaries.contains(&pos) {
                    return Err(bad("insert splits a surrogate pair"));
                }
                let len = insert.encode_utf16().count();
                boundaries = boundaries
                    .into_iter()
                    .map(|b| if b >= pos { b + len } else { b })
                    .collect();
                let mut off = pos;
                for c in insert.chars() {
                    boundaries.insert(off);
                    off += c.len_utf16();
                }
                total += len;
                pos += len;
            }
        }
    }
    Ok(())
}

/// Convert a Loro text delta (Unicode-scalar units) into UTF-16 units
/// against `pre`, the text before the change. Returns the converted
/// delta and the text after the change, or `None` when the delta does
/// not fit `pre` (the shadow was out of step). Attributes are dropped:
/// Phase 2 is plain text.
fn utf16_delta_from_scalar(pre: &str, delta: &[TextDelta]) -> Option<(Vec<NotesDeltaOp>, String)> {
    let chars: Vec<char> = pre.chars().collect();
    let mut cursor = 0usize;
    let mut out = Vec::with_capacity(delta.len());
    let mut post = String::with_capacity(pre.len());
    for op in delta {
        match op {
            TextDelta::Retain { retain, .. } => {
                let end = cursor.checked_add(*retain)?;
                let slice = chars.get(cursor..end)?;
                let n: usize = slice.iter().map(|c| c.len_utf16()).sum();
                post.extend(slice);
                out.push(NotesDeltaOp::Retain { retain: n });
                cursor = end;
            }
            TextDelta::Delete { delete } => {
                let end = cursor.checked_add(*delete)?;
                let slice = chars.get(cursor..end)?;
                let n: usize = slice.iter().map(|c| c.len_utf16()).sum();
                out.push(NotesDeltaOp::Delete { delete: n });
                cursor = end;
            }
            TextDelta::Insert { insert, .. } => {
                post.push_str(insert);
                out.push(NotesDeltaOp::Insert {
                    insert: insert.clone(),
                });
            }
        }
    }
    post.extend(chars.get(cursor..)?);
    Some((out, post))
}

/// Full-replace delta from `pre` to `post` (UTF-16 units): delete
/// everything, insert the new text. Empty when they are equal.
fn replace_delta(pre: &str, post: &str) -> Vec<NotesDeltaOp> {
    if pre == post {
        return Vec::new();
    }
    let mut out = Vec::with_capacity(2);
    let len = pre.encode_utf16().count();
    if len > 0 {
        out.push(NotesDeltaOp::Delete { delete: len });
    }
    if !post.is_empty() {
        out.push(NotesDeltaOp::Insert {
            insert: post.to_string(),
        });
    }
    out
}

/// Compose two UTF-16 deltas so that `compose(a, b)` applied to a text
/// equals applying `a` then `b`. Standard Quill compose over plain
/// text (no attributes). Trailing retains are trimmed.
fn compose_utf16_deltas(a: &[NotesDeltaOp], b: &[NotesDeltaOp]) -> Vec<NotesDeltaOp> {
    if a.is_empty() {
        return trim_delta(b.to_vec());
    }
    if b.is_empty() {
        return trim_delta(a.to_vec());
    }
    // Expand `a` into a per-unit stream is wasteful; instead iterate
    // both with remaining counts.
    #[derive(Clone)]
    enum Cur {
        Retain(usize),
        Insert(Vec<u16>),
        Delete(usize),
    }
    let to_cur = |op: &NotesDeltaOp| match op {
        NotesDeltaOp::Retain { retain } => Cur::Retain(*retain),
        NotesDeltaOp::Insert { insert } => Cur::Insert(insert.encode_utf16().collect()),
        NotesDeltaOp::Delete { delete } => Cur::Delete(*delete),
    };
    let mut ai = a.iter().map(to_cur).peekable();
    let mut bi = b.iter().map(to_cur).peekable();
    let mut out: Vec<Cur> = Vec::new();
    let push = |out: &mut Vec<Cur>, c: Cur| match (out.last_mut(), c) {
        (Some(Cur::Retain(x)), Cur::Retain(y)) => *x += y,
        (Some(Cur::Delete(x)), Cur::Delete(y)) => *x += y,
        (Some(Cur::Insert(x)), Cur::Insert(y)) => x.extend(y),
        (_, c) => out.push(c),
    };
    let mut a_cur: Option<Cur> = ai.next();
    let mut b_cur: Option<Cur> = bi.next();
    loop {
        match (a_cur.take(), b_cur.take()) {
            (None, None) => break,
            (Some(x), None) => {
                push(&mut out, x);
                a_cur = ai.next();
            }
            (None, Some(y)) => {
                push(&mut out, y);
                b_cur = bi.next();
            }
            (Some(Cur::Delete(n)), y) => {
                // Deletes in `a` pass through untouched.
                push(&mut out, Cur::Delete(n));
                a_cur = ai.next();
                b_cur = y;
            }
            (x, Some(Cur::Insert(s))) => {
                // Inserts in `b` pass through untouched.
                push(&mut out, Cur::Insert(s));
                b_cur = bi.next();
                a_cur = x;
            }
            (Some(Cur::Retain(n)), Some(Cur::Retain(m))) => {
                let k = n.min(m);
                push(&mut out, Cur::Retain(k));
                a_cur = (n > k).then_some(Cur::Retain(n - k)).or_else(|| ai.next());
                b_cur = (m > k).then_some(Cur::Retain(m - k)).or_else(|| bi.next());
            }
            (Some(Cur::Retain(n)), Some(Cur::Delete(m))) => {
                let k = n.min(m);
                push(&mut out, Cur::Delete(k));
                a_cur = (n > k).then_some(Cur::Retain(n - k)).or_else(|| ai.next());
                b_cur = (m > k).then_some(Cur::Delete(m - k)).or_else(|| bi.next());
            }
            (Some(Cur::Insert(s)), Some(Cur::Retain(m))) => {
                let k = s.len().min(m);
                push(&mut out, Cur::Insert(s[..k].to_vec()));
                a_cur = (s.len() > k)
                    .then(|| Cur::Insert(s[k..].to_vec()))
                    .or_else(|| ai.next());
                b_cur = (m > k).then_some(Cur::Retain(m - k)).or_else(|| bi.next());
            }
            (Some(Cur::Insert(s)), Some(Cur::Delete(m))) => {
                // `b` deletes what `a` inserted: they cancel.
                let k = s.len().min(m);
                a_cur = (s.len() > k)
                    .then(|| Cur::Insert(s[k..].to_vec()))
                    .or_else(|| ai.next());
                b_cur = (m > k).then_some(Cur::Delete(m - k)).or_else(|| bi.next());
            }
        }
    }
    let ops = out
        .into_iter()
        .map(|c| match c {
            Cur::Retain(n) => NotesDeltaOp::Retain { retain: n },
            Cur::Delete(n) => NotesDeltaOp::Delete { delete: n },
            Cur::Insert(u) => NotesDeltaOp::Insert {
                insert: String::from_utf16_lossy(&u),
            },
        })
        .collect();
    trim_delta(ops)
}

fn trim_delta(mut ops: Vec<NotesDeltaOp>) -> Vec<NotesDeltaOp> {
    while matches!(ops.last(), Some(NotesDeltaOp::Retain { .. })) {
        ops.pop();
    }
    ops
}

/// `duration` register as minutes; an out-of-range value (a newer client
/// with a wider bound, or a stray write) reads as unset.
fn read_duration(map: &LoroMap) -> Option<u32> {
    let n = read_i64(map, KEY_DURATION)?;
    u32::try_from(n)
        .ok()
        .filter(|n| (1..=MAX_DURATION_MINUTES).contains(n))
}

fn read_i64(map: &LoroMap, key: &str) -> Option<i64> {
    let v = map.get(key)?;
    let value = v.as_value()?.clone();
    value.into_i64().ok()
}

fn read_bool(map: &LoroMap, key: &str) -> Option<bool> {
    let v = map.get(key)?;
    let value = v.as_value()?.clone();
    value.into_bool().ok()
}

fn settings_view(map: &LoroMap) -> SettingsView {
    SettingsView {
        // Defaults on: a never-toggled doc shows counts. The key is only
        // persisted on the opt-out path (stored `false`); absence means
        // the default.
        show_list_counts: read_bool(map, KEY_SHOW_LIST_COUNTS).unwrap_or(true),
        // Inbox has no ListMeta row, so its saved default view lives
        // here; same encoding and same absent ≡ no-default reading as a
        // user list's `view` key.
        inbox_view: read_default_view(map, KEY_INBOX_VIEW),
    }
}

/// Location/lifecycle slice used by the projection index. Items with a
/// missing or unparseable `location` deterministically project into
/// `inbox`'s fallback tail (empty placement never matches an entry) so
/// data is never hidden by a bad register.
fn item_meta(map: &LoroMap) -> ItemMeta {
    let (list_id, placement_id) = read_string(map, KEY_LOCATION)
        .and_then(|s| Location::parse(&s))
        .map(|l| (l.list_id, l.placement_id))
        .unwrap_or_else(|| (LIST_INBOX.to_string(), String::new()));
    ItemMeta {
        list_id,
        placement_id,
        open: is_open(map),
        created_at: read_i64(map, KEY_CREATED_AT).unwrap_or(0),
    }
}

/// Validate a date-only deadline and return it normalized. Accepts
/// exactly `YYYY-MM-DD` (4-digit year, 2-digit month, 2-digit day) that
/// names a real calendar day (month 1–12, day within the month's length,
/// leap years honored). Leading/trailing whitespace is trimmed before
/// checking. Any other shape — a time component, unix millis, a
/// single-digit field, an out-of-range day — is rejected. The value is a
/// floating local date; no timezone is applied.
fn parse_deadline(raw: &str) -> Result<String, DocError> {
    let s = raw.trim();
    let bytes = s.as_bytes();
    let invalid = || DocError::Invalid(format!("deadline must be YYYY-MM-DD: {raw:?}"));
    if bytes.len() != 10 || bytes[4] != b'-' || bytes[7] != b'-' {
        return Err(invalid());
    }
    let digits = |from: usize, to: usize| -> Option<u32> {
        let part = &s[from..to];
        if part.bytes().all(|b| b.is_ascii_digit()) {
            part.parse::<u32>().ok()
        } else {
            None
        }
    };
    let (year, month, day) = match (digits(0, 4), digits(5, 7), digits(8, 10)) {
        (Some(y), Some(m), Some(d)) => (y, m, d),
        _ => return Err(invalid()),
    };
    if !(1..=12).contains(&month) || day < 1 {
        return Err(invalid());
    }
    let leap = year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
    let days_in_month = match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if leap => 29,
        2 => 28,
        _ => unreachable!(),
    };
    if day > days_in_month {
        return Err(invalid());
    }
    Ok(s.to_string())
}

/// Validate a planned date and return it normalized (trimmed). Accepts
/// exactly `YYYY-MM-DD` (all-day, 10 chars) or `YYYY-MM-DDTHH:MM` (timed,
/// 16 chars, hour `00..=23`, minute `00..=59`). The date part goes
/// through `parse_deadline`'s calendar check. Seconds, offsets, and the
/// reserved RFC 9557 `[Zone]` suffix are rejected until fixed-instant
/// support lands (`spec/calendar-plan.md`). Floating; no timezone.
fn parse_when(raw: &str) -> Result<String, DocError> {
    let s = raw.trim();
    let invalid = || {
        DocError::Invalid(format!(
            "when must be YYYY-MM-DD or YYYY-MM-DDTHH:MM: {raw:?}"
        ))
    };
    match s.len() {
        10 => parse_deadline(s).map_err(|_| invalid()),
        16 => {
            let bytes = s.as_bytes();
            if bytes[10] != b'T' || bytes[13] != b':' {
                return Err(invalid());
            }
            parse_deadline(&s[..10]).map_err(|_| invalid())?;
            let two = |from: usize| -> Option<u32> {
                let part = &s[from..from + 2];
                if part.bytes().all(|b| b.is_ascii_digit()) {
                    part.parse::<u32>().ok()
                } else {
                    None
                }
            };
            match (two(11), two(14)) {
                (Some(h), Some(m)) if h <= 23 && m <= 59 => Ok(s.to_string()),
                _ => Err(invalid()),
            }
        }
        _ => Err(invalid()),
    }
}

fn item_view(map: &LoroMap) -> Option<ItemView> {
    let location = read_string(map, KEY_LOCATION)
        .and_then(|s| Location::parse(&s))
        .map(|l| l.list_id)
        .unwrap_or_else(|| LIST_INBOX.to_string());
    let (state, lifecycle_at) = workflow_of(map);
    Some(ItemView {
        id: read_string(map, KEY_ID)?,
        text: read_string(map, KEY_TEXT)?,
        notes: read_text(map, KEY_NOTES).unwrap_or_default(),
        list_id: location,
        state,
        lifecycle_at,
        deadline: read_string(map, KEY_DEADLINE).filter(|s| !s.is_empty()),
        when: read_string(map, KEY_WHEN).filter(|s| !s.is_empty()),
        duration: read_duration(map),
        created_at: read_i64(map, KEY_CREATED_AT)?,
        started_at: read_i64(map, KEY_STARTED_AT),
        done_at: read_i64(map, KEY_DONE_AT),
        binned_at: read_i64(map, KEY_BINNED_AT),
    })
}

fn list_view(map: &LoroMap) -> Option<ListView> {
    Some(ListView {
        id: read_string(map, KEY_ID)?,
        name: read_string(map, KEY_NAME)?,
        icon: read_string(map, KEY_ICON).filter(|s| !s.is_empty()),
        default_view: read_default_view(map, KEY_VIEW),
        archived_at: read_i64(map, KEY_ARCHIVED_AT),
        created_at: read_i64(map, KEY_CREATED_AT)?,
    })
}

/// Read an encoded [`DefaultView`] register. Absent, empty, or written
/// by a future client in a form this build doesn't recognize all read as
/// `None` — "no saved default", so the client falls back to its own.
fn read_default_view(map: &LoroMap, key: &str) -> Option<DefaultView> {
    read_string(map, key)
        .as_deref()
        .and_then(DefaultView::parse)
}

/// Decode the item's workflow register: a plain `LoroValue` list
/// `[state, at]`. Wrong shape, non-integer members, or an unrecognized
/// state code (a newer client's state) all read as `None` — the caller
/// applies the `[Backlog, created_at]` fallback so a future state
/// degrades to visible-and-open, never silently hidden.
fn read_workflow(map: &LoroMap) -> Option<(WorkflowState, i64)> {
    let v = map.get(KEY_LIFECYCLE)?;
    let value = v.as_value()?.clone();
    let list = value.into_list().ok()?;
    let [state, at] = list.as_slice() else {
        return None;
    };
    let state = state.clone().into_i64().ok()?;
    let at = at.clone().into_i64().ok()?;
    Some((WorkflowState::from_code(state)?, at))
}

/// The item's workflow register with the absent/unparseable fallback
/// applied: `[Backlog, created_at]`.
fn workflow_of(map: &LoroMap) -> (WorkflowState, i64) {
    read_workflow(map).unwrap_or_else(|| {
        (
            WorkflowState::Backlog,
            read_i64(map, KEY_CREATED_AT).unwrap_or(0),
        )
    })
}

/// Write the workflow register as one atomic plain value — a single op,
/// merged whole by LWW.
fn write_workflow(map: &LoroMap, state: WorkflowState, at: i64) -> Result<(), DocError> {
    let value = LoroValue::List(vec![LoroValue::I64(state.code()), LoroValue::I64(at)].into());
    map.insert(KEY_LIFECYCLE, value)?;
    Ok(())
}

/// One entry in the lifecycle-mutation vocabulary — the target of
/// [`Doc::set_items_lifecycle_impl`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LifecycleWrite {
    /// The transition table's targets (`spec/data-model.md` "Set
    /// lifecycle").
    Set(ItemLifecycle),
    /// Restore from the bin: clear `binned_at` only, revealing the
    /// preserved workflow state. No-op when not binned.
    Restore,
    /// Un-done: write Backlog, but only for items currently resolved
    /// Done (skips the rest, so a bulk un-done never disturbs open items).
    UnDone,
}

/// Apply one lifecycle transition to an item map per the transition
/// table (`spec/data-model.md` "Set lifecycle"). Returns whether
/// anything was written; does not commit. Re-applying the current
/// resolved lifecycle is a no-op.
///
/// - Open / Done targets write the register `[state, now]`, clear the
///   bin mask, and ride the reflection stamps in the same batch:
///   entering In Progress sets `started_at` iff absent; entering Done
///   sets `done_at`.
/// - Binned sets the mask only — the register is preserved for restore.
fn apply_lifecycle(map: &LoroMap, lifecycle: ItemLifecycle, now: i64) -> Result<bool, DocError> {
    let binned = read_i64(map, KEY_BINNED_AT).is_some();
    match lifecycle.workflow_state() {
        None => {
            if binned {
                return Ok(false);
            }
            map.insert(KEY_BINNED_AT, now)?;
            Ok(true)
        }
        Some(want) => {
            let (cur, _) = workflow_of(map);
            if !binned && cur == want {
                return Ok(false);
            }
            if binned {
                let _ = map.delete(KEY_BINNED_AT);
            }
            write_workflow(map, want, now)?;
            if want == WorkflowState::InProgress && read_i64(map, KEY_STARTED_AT).is_none() {
                map.insert(KEY_STARTED_AT, now)?;
            }
            if want == WorkflowState::Done {
                map.insert(KEY_DONE_AT, now)?;
            }
            Ok(true)
        }
    }
}

/// Dispatch a [`LifecycleWrite`] onto an item map. Returns whether
/// anything was written; does not commit.
fn apply_lifecycle_write(map: &LoroMap, write: LifecycleWrite, now: i64) -> Result<bool, DocError> {
    match write {
        LifecycleWrite::Set(lifecycle) => apply_lifecycle(map, lifecycle, now),
        LifecycleWrite::Restore => {
            if read_i64(map, KEY_BINNED_AT).is_none() {
                return Ok(false);
            }
            let _ = map.delete(KEY_BINNED_AT);
            Ok(true)
        }
        LifecycleWrite::UnDone => {
            let binned = read_i64(map, KEY_BINNED_AT).is_some();
            if binned || workflow_of(map).0 != WorkflowState::Done {
                return Ok(false);
            }
            apply_lifecycle(map, ItemLifecycle::Backlog, now)
        }
    }
}

fn is_open(map: &LoroMap) -> bool {
    read_i64(map, KEY_BINNED_AT).is_none() && workflow_of(map).0.is_open()
}

fn hash_str(hasher: &mut Sha256, s: &str) {
    hasher.update((s.len() as u32).to_be_bytes());
    hasher.update(s.as_bytes());
}

fn hash_opt_str(hasher: &mut Sha256, v: Option<&str>) {
    match v {
        Some(s) => {
            hasher.update([1u8]);
            hash_str(hasher, s);
        }
        None => hasher.update([0u8]),
    }
}

fn hash_opt_i64(hasher: &mut Sha256, v: Option<i64>) {
    match v {
        Some(n) => {
            hasher.update([1u8]);
            hasher.update(n.to_be_bytes());
        }
        None => hasher.update([0u8]),
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn now_millis() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

#[cfg(target_arch = "wasm32")]
fn now_millis() -> i64 {
    js_sys::Date::now() as i64
}

fn new_id() -> String {
    Uuid::now_v7().simple().to_string()
}

/// Diff two ordered slices of `ItemView` by id and emit `AppEvent`s for
/// the transitions a UI store needs to mirror. Both slices are in the
/// canonical grouped walk order (`all_items`), so walking `post` with a
/// per-list counter reproduces each list's open projection. Used by
/// bulk local mutations and `import_json`, where the cheapest path to
/// per-id deltas is "snapshot before, snapshot after, walk both once."
fn diff_items(pre: &[ItemView], post: &[ItemView], out: &mut Vec<AppEvent>) {
    let pre_by_id: HashMap<&str, (usize, &ItemView)> = pre
        .iter()
        .enumerate()
        .map(|(i, it)| (it.id.as_str(), (i, it)))
        .collect();
    let post_by_id: HashMap<&str, (usize, &ItemView)> = post
        .iter()
        .enumerate()
        .map(|(i, it)| (it.id.as_str(), (i, it)))
        .collect();
    // Post-state open position per item id. Events emitted below are in
    // ascending post order, so a consumer applying
    // remove-then-insert-at-open_index per event converges on exactly
    // this projection.
    let mut open_counters = HashMap::<&str, usize>::new();
    let mut open_pos = HashMap::<&str, usize>::with_capacity(post.len());
    for it in post {
        if it.is_open() {
            let counter = open_counters.entry(it.list_id.as_str()).or_insert(0);
            open_pos.insert(it.id.as_str(), *counter);
            *counter += 1;
        }
    }

    for it in pre {
        if !post_by_id.contains_key(it.id.as_str()) {
            out.push(AppEvent::ItemRemoved { id: it.id.clone() });
        }
    }
    for (post_idx, post_it) in post.iter().enumerate() {
        let open_index = open_pos.get(post_it.id.as_str()).copied();
        match pre_by_id.get(post_it.id.as_str()) {
            None => {
                out.push(AppEvent::ItemAdded {
                    id: post_it.id.clone(),
                    list_id: post_it.list_id.clone(),
                    text: post_it.text.clone(),
                    notes: post_it.notes.clone(),
                    created_at: post_it.created_at,
                    state: post_it.state,
                    lifecycle_at: post_it.lifecycle_at,
                    started_at: post_it.started_at,
                    done_at: post_it.done_at,
                    binned_at: post_it.binned_at,
                    deadline: post_it.deadline.clone(),
                    when: post_it.when.clone(),
                    duration: post_it.duration,
                    open_index,
                });
            }
            Some(&(pre_idx, pre_it)) => {
                if pre_it.text != post_it.text {
                    out.push(AppEvent::ItemTextChanged {
                        id: post_it.id.clone(),
                        text: post_it.text.clone(),
                    });
                }
                if pre_it.notes != post_it.notes {
                    out.push(AppEvent::ItemNotesChanged {
                        id: post_it.id.clone(),
                        notes: post_it.notes.clone(),
                    });
                }
                if pre_it.deadline != post_it.deadline {
                    out.push(AppEvent::ItemDeadlineChanged {
                        id: post_it.id.clone(),
                        deadline: post_it.deadline.clone(),
                    });
                }
                if pre_it.when != post_it.when {
                    out.push(AppEvent::ItemWhenChanged {
                        id: post_it.id.clone(),
                        when: post_it.when.clone(),
                    });
                }
                if pre_it.duration != post_it.duration {
                    out.push(AppEvent::ItemDurationChanged {
                        id: post_it.id.clone(),
                        duration: post_it.duration,
                    });
                }
                if pre_it.state != post_it.state
                    || pre_it.lifecycle_at != post_it.lifecycle_at
                    || pre_it.started_at != post_it.started_at
                    || pre_it.done_at != post_it.done_at
                    || pre_it.binned_at != post_it.binned_at
                {
                    out.push(AppEvent::ItemLifecycleChanged {
                        id: post_it.id.clone(),
                        state: post_it.state,
                        lifecycle_at: post_it.lifecycle_at,
                        started_at: post_it.started_at,
                        done_at: post_it.done_at,
                        binned_at: post_it.binned_at,
                        open_index,
                    });
                }
                if pre_it.list_id != post_it.list_id {
                    out.push(AppEvent::ItemListChanged {
                        id: post_it.id.clone(),
                        list_id: post_it.list_id.clone(),
                        open_index,
                    });
                }
                if pre_idx != post_idx {
                    out.push(AppEvent::ItemMoved {
                        id: post_it.id.clone(),
                        open_index,
                    });
                }
            }
        }
    }
}

/// Diff two ordered slices of `ListView`. Mirror of `diff_items` for
/// the lists root container.
fn diff_lists(pre: &[ListView], post: &[ListView], out: &mut Vec<AppEvent>) {
    let pre_by_id: HashMap<&str, (usize, &ListView)> = pre
        .iter()
        .enumerate()
        .map(|(i, l)| (l.id.as_str(), (i, l)))
        .collect();
    let post_by_id: HashMap<&str, (usize, &ListView)> = post
        .iter()
        .enumerate()
        .map(|(i, l)| (l.id.as_str(), (i, l)))
        .collect();

    for l in pre {
        if !post_by_id.contains_key(l.id.as_str()) {
            out.push(AppEvent::ListRemoved { id: l.id.clone() });
        }
    }
    for (post_idx, post_l) in post.iter().enumerate() {
        match pre_by_id.get(post_l.id.as_str()) {
            None => {
                out.push(AppEvent::ListAdded {
                    id: post_l.id.clone(),
                    name: post_l.name.clone(),
                    created_at: post_l.created_at,
                    archived_at: post_l.archived_at,
                    index: post_idx,
                });
            }
            Some(&(pre_idx, pre_l)) => {
                if pre_l.name != post_l.name {
                    out.push(AppEvent::ListRenamed {
                        id: post_l.id.clone(),
                        name: post_l.name.clone(),
                    });
                }
                if pre_l.icon != post_l.icon {
                    out.push(AppEvent::ListIconChanged {
                        id: post_l.id.clone(),
                        icon: post_l.icon.clone(),
                    });
                }
                if pre_l.default_view != post_l.default_view {
                    out.push(AppEvent::ListDefaultViewChanged {
                        id: post_l.id.clone(),
                        view: post_l.default_view,
                    });
                }
                if pre_l.archived_at != post_l.archived_at {
                    out.push(AppEvent::ListArchivedChanged {
                        id: post_l.id.clone(),
                        archived_at: post_l.archived_at,
                    });
                }
                if pre_idx != post_idx {
                    out.push(AppEvent::ListMoved {
                        id: post_l.id.clone(),
                        index: post_idx,
                    });
                }
            }
        }
    }
}

fn diff_settings(pre: &SettingsView, post: &SettingsView, out: &mut Vec<AppEvent>) {
    if pre != post {
        out.push(AppEvent::SettingsChanged {
            show_list_counts: post.show_list_counts,
            inbox_view: post.inbox_view,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crypto::Dek;

    /// The incremental index must always agree with a from-scratch
    /// recompute off the Loro containers.
    fn assert_open_projection_matches_doc(doc: &Doc) {
        let fresh = doc.compute_index();
        let guard = doc.item_index.lock().expect("item index mutex poisoned");
        assert_eq!(
            guard.open_by_list, fresh.open_by_list,
            "open_by_list out of sync with doc"
        );
        assert_eq!(
            guard.raw_orders, fresh.raw_orders,
            "raw order shadow out of sync with doc"
        );
        assert_eq!(guard.meta, fresh.meta, "item meta out of sync with doc");
        assert_eq!(
            guard.visible_counts, fresh.visible_counts,
            "visible-entry counts out of sync with doc"
        );
    }

    /// spec/peer-id-plan.md: the `_with_peer` constructors bind the
    /// explicit peer, local commits carry it, and Loro's reserved
    /// `u64::MAX` is rejected.
    #[test]
    fn constructors_bind_explicit_peer() {
        let doc = Doc::new_with_peer(7).unwrap();
        assert_eq!(doc.peer_id(), 7);
        doc.add_item(LIST_INBOX, "carried by peer 7").unwrap();
        assert!(
            doc.oplog_vv().get(&7).copied().unwrap_or(0) > 0,
            "local commits must carry the explicit peer"
        );
        let doc = Doc::empty_with_peer(9).unwrap();
        assert_eq!(doc.peer_id(), 9);
        assert!(Doc::empty_with_peer(u64::MAX).is_err());
    }

    /// Not a correctness test — a quick order-of-magnitude probe for
    /// the per-mutation cost terms at a realistic lifetime-item count.
    /// Run with:
    ///   cargo test -p monoplan-core --release bench_mutation_terms_at_13k -- --ignored --nocapture
    #[test]
    #[ignore]
    fn bench_mutation_terms_at_13k() {
        use std::time::Instant;
        let dek = Dek::generate();
        let doc = Doc::new().unwrap();
        // ~13k lifetime items: 200 open in main, the rest done — the
        // user-reported shape (many small lists, large Done history).
        let texts: Vec<String> = (0..13_000).map(|i| format!("item number {i}")).collect();
        let refs: Vec<&str> = texts.iter().map(String::as_str).collect();
        let ids = doc.add_items_at(LIST_INBOX, &refs, 0).unwrap();
        let done_refs: Vec<&str> = ids[..12_800].iter().map(String::as_str).collect();
        doc.set_items_done(&done_refs, true).unwrap();
        let _ = doc.drain_events();

        let t = Instant::now();
        let id = doc.add_item(LIST_INBOX, "one more").unwrap();
        println!("add_item:            {:?}", t.elapsed());

        let t = Instant::now();
        doc.set_item_done(&id, true).unwrap();
        println!("set_item_done:       {:?}", t.elapsed());

        let t = Instant::now();
        doc.set_items_binned(&[ids[12_900].as_str()], true).unwrap();
        println!("set_items_binned(1): {:?}", t.elapsed());

        let t = Instant::now();
        let blob = doc.pending_export(&dek).unwrap();
        println!(
            "pending_export:      {:?} ({} bytes)",
            t.elapsed(),
            blob.map(|b| b.ciphertext.len()).unwrap_or(0)
        );

        let t = Instant::now();
        let snap = doc.snapshot_blob(&dek).unwrap();
        println!(
            "snapshot_blob:       {:?} ({} bytes)",
            t.elapsed(),
            snap.ciphertext.len()
        );

        let t = Instant::now();
        let all: Vec<ItemView> = doc.iter_items().collect();
        println!(
            "iter_items.collect:  {:?} ({} items)",
            t.elapsed(),
            all.len()
        );

        let t = Instant::now();
        doc.rebuild_index();
        println!("rebuild_index:       {:?}", t.elapsed());
        let _ = doc.drain_events();

        // Match a real browser session: persisted history is loaded before
        // the user performs the move, so the UndoManager only owns the
        // current session's action rather than the 13k-item fixture setup.
        let saved = doc.save().unwrap();
        let doc = Doc::load(&saved).unwrap();
        doc.move_item(&ids[12_999], LIST_INBOX, 0).unwrap();
        let _ = doc.drain_events();
        let t = Instant::now();
        assert!(doc.undo().unwrap());
        println!("undo_move:            {:?}", t.elapsed());
        println!("  events emitted:    {}", doc.drain_events().len());
        let t = Instant::now();
        assert!(doc.redo().unwrap());
        println!("redo_move:            {:?}", t.elapsed());
        println!("  events emitted:    {}", doc.drain_events().len());

        let doc = Doc::load(&saved).unwrap();
        for (index, id) in ids[12_980..13_000].iter().enumerate() {
            doc.move_item(id, LIST_INBOX, index).unwrap();
        }
        let _ = doc.drain_events();
        let t = Instant::now();
        for _ in 0..20 {
            assert!(doc.undo().unwrap());
        }
        println!("undo_20_moves:         {:?}", t.elapsed());
        println!("  events emitted:    {}", doc.drain_events().len());

        // Remote-frame cost on a peer at the same size: one op in, how
        // long to apply + translate?
        let mut a = Doc::new().unwrap();
        let texts2: Vec<String> = (0..13_000).map(|i| format!("item number {i}")).collect();
        let refs2: Vec<&str> = texts2.iter().map(String::as_str).collect();
        let a_ids = a.add_items_at(LIST_INBOX, &refs2, 0).unwrap();
        let seed = a.pending_export(&dek).unwrap().unwrap();
        a.mark_persisted();
        let mut b = Doc::empty();
        b.apply_remote(&dek, &seed).unwrap();
        let _ = a.drain_events();
        let _ = b.drain_events();
        a.set_item_done(&a_ids[6_500], true).unwrap();
        let frame = a.pending_export(&dek).unwrap().unwrap();
        let t = Instant::now();
        b.apply_remote(&dek, &frame).unwrap();
        println!("apply_remote(1 op):  {:?}", t.elapsed());
        let evs = b.drain_events();
        println!("  events emitted:    {}", evs.len());
    }

    #[test]
    fn new_doc_has_no_persisted_lists() {
        // Main is a reserved id with no ListMeta row, and there are no
        // seeded user lists — genesis stays a clean baseline; starter
        // content is seeded client-side (see App.tsx / the CLI init path).
        let doc = Doc::new().unwrap();
        let lists = doc.all_lists();
        assert!(lists.is_empty());
        assert!(!lists.iter().any(|l| l.id == LIST_INBOX));
    }

    #[test]
    fn add_item_to_main_works_without_list_meta_row() {
        // `LIST_INBOX` is virtual — items can address it even though
        // no ListMeta row exists for it.
        let doc = Doc::new().unwrap();
        let id = doc.add_item(LIST_INBOX, "milk").unwrap();
        assert_eq!(doc.get_item(&id).unwrap().list_id, LIST_INBOX);
        assert_eq!(doc.open_item_ids(LIST_INBOX), vec![id]);
    }

    #[test]
    fn no_document_wide_item_movable_list_remains() {
        // Success criterion for the v2 schema: `items` is a map keyed
        // by item id, ordering lives in per-list order containers, and
        // the old global MovableList root never materializes.
        let doc = Doc::new().unwrap();
        let a = doc.add_item(LIST_INBOX, "a").unwrap();
        let other = doc.add_list("Other").unwrap();
        let b = doc.add_item(&other, "b").unwrap();

        assert!(doc.items().get(&a).is_some());
        assert!(doc.items().get(&b).is_some());
        assert_eq!(doc.order_list(LIST_INBOX).len(), 1);
        assert_eq!(doc.order_list(&other).len(), 1);
        // The v1 root (`items` as a MovableList) is a different
        // container type entirely and stays empty.
        assert_eq!(doc.inner.get_movable_list(ROOT_ITEMS).len(), 0);
    }

    #[test]
    fn reordering_one_list_does_not_touch_other_order_containers() {
        let doc = Doc::new().unwrap();
        let other = doc.add_list("Other").unwrap();
        let a = doc.add_item(LIST_INBOX, "a").unwrap();
        let _b = doc.add_item(LIST_INBOX, "b").unwrap();
        let o1 = doc.add_item(&other, "o1").unwrap();
        let o2 = doc.add_item(&other, "o2").unwrap();

        let other_before: Vec<Option<OrderEntry>> = {
            let order = doc.order_list(&other);
            (0..order.len())
                .map(|i| scalar_entry_at(&order, i))
                .collect()
        };
        let vv_before = doc.oplog_vv();

        doc.move_item(&a, LIST_INBOX, 1).unwrap();

        let other_after: Vec<Option<OrderEntry>> = {
            let order = doc.order_list(&other);
            (0..order.len())
                .map(|i| scalar_entry_at(&order, i))
                .collect()
        };
        assert_eq!(other_before, other_after);
        assert_eq!(doc.open_item_ids(&other), vec![o1, o2]);
        assert!(doc.oplog_vv() != vv_before, "the reorder itself committed");
    }

    #[test]
    fn move_list_refuses_main() {
        let doc = Doc::new().unwrap();
        assert!(matches!(
            doc.move_list(LIST_INBOX, 0).unwrap_err(),
            DocError::CannotMoveBuiltin(_)
        ));
    }

    #[test]
    fn rename_list_refuses_main() {
        let doc = Doc::new().unwrap();
        assert!(matches!(
            doc.rename_list(LIST_INBOX, "Today").unwrap_err(),
            DocError::CannotRenameBuiltin(_)
        ));
    }

    #[test]
    fn set_list_icon_round_trips_and_clears() {
        let doc = Doc::new().unwrap();
        let list = doc.add_list("Work").unwrap();
        let _ = doc.drain_events();

        // Set → stored on the view + ListIconChanged emitted.
        doc.set_list_icon(&list, "📥").unwrap();
        assert_eq!(
            doc.get_list_meta(&list).unwrap().icon.as_deref(),
            Some("📥")
        );
        assert!(doc.drain_events().iter().any(|e| matches!(
            e,
            AppEvent::ListIconChanged { id, icon } if id == &list && icon.as_deref() == Some("📥")
        )));

        // No-op when unchanged (no phantom event).
        doc.set_list_icon(&list, "📥").unwrap();
        assert!(doc.drain_events().is_empty());

        // Empty clears → view drops the icon, event carries None.
        doc.set_list_icon(&list, "   ").unwrap();
        assert_eq!(doc.get_list_meta(&list).unwrap().icon, None);
        assert!(doc.drain_events().iter().any(|e| matches!(
            e,
            AppEvent::ListIconChanged { id, icon: None } if id == &list
        )));
    }

    #[test]
    fn set_list_icon_refuses_main() {
        let doc = Doc::new().unwrap();
        assert!(matches!(
            doc.set_list_icon(LIST_INBOX, "📥").unwrap_err(),
            DocError::CannotRenameBuiltin(_)
        ));
    }

    #[test]
    fn set_default_view_round_trips_and_clears() {
        let doc = Doc::new().unwrap();
        let list = doc.add_list("Work").unwrap();
        let _ = doc.drain_events();

        let board_nodone = DefaultView {
            board: true,
            lanes: LaneSet::ALL.with(WorkflowState::Done, false),
        };
        doc.set_default_view(&list, Some(board_nodone)).unwrap();
        assert_eq!(
            doc.get_list_meta(&list).unwrap().default_view,
            Some(board_nodone)
        );
        assert!(doc.drain_events().iter().any(|e| matches!(
            e,
            AppEvent::ListDefaultViewChanged { id, view: Some(v) }
                if id == &list && *v == board_nodone
        )));

        // No-op when unchanged (no phantom event / undo step).
        doc.set_default_view(&list, Some(board_nodone)).unwrap();
        assert!(doc.drain_events().is_empty());

        // Clearing drops the key and reports "no saved default".
        doc.set_default_view(&list, None).unwrap();
        assert_eq!(doc.get_list_meta(&list).unwrap().default_view, None);
        assert!(doc.drain_events().iter().any(|e| matches!(
            e,
            AppEvent::ListDefaultViewChanged { id, view: None } if id == &list
        )));
    }

    #[test]
    fn set_default_view_on_inbox_lands_in_settings() {
        let doc = Doc::new().unwrap();
        let _ = doc.drain_events();

        // Inbox has no ListMeta row, so its default rides on the
        // doc-level settings map and its settings event.
        doc.set_default_view(LIST_INBOX, Some(DefaultView::BOARD))
            .unwrap();
        assert_eq!(doc.get_settings().inbox_view, Some(DefaultView::BOARD));
        assert!(doc.drain_events().iter().any(|e| matches!(
            e,
            AppEvent::SettingsChanged {
                inbox_view: Some(v),
                ..
            } if *v == DefaultView::BOARD
        )));

        doc.set_default_view(LIST_INBOX, Some(DefaultView::BOARD))
            .unwrap();
        assert!(doc.drain_events().is_empty());

        doc.set_default_view(LIST_INBOX, None).unwrap();
        assert_eq!(doc.get_settings().inbox_view, None);
    }

    #[test]
    fn default_view_encoding_round_trips() {
        let no_done = LaneSet::ALL.with(WorkflowState::Done, false);
        let two_lanes: LaneSet = [WorkflowState::InProgress, WorkflowState::Done]
            .into_iter()
            .collect();
        for view in [
            DefaultView::LIST,
            DefaultView::BOARD,
            DefaultView {
                board: true,
                lanes: no_done,
            },
            DefaultView {
                board: true,
                lanes: two_lanes,
            },
        ] {
            assert_eq!(DefaultView::parse(&view.encode()), Some(view));
        }
        // Lanes list in ladder order, and the full set is bare "board".
        assert_eq!(
            DefaultView {
                board: true,
                lanes: two_lanes,
            }
            .encode(),
            "board:in_progress,done"
        );
        assert_eq!(
            DefaultView {
                board: true,
                lanes: no_done,
            }
            .encode(),
            "board:backlog,todo,in_progress,review"
        );
        assert_eq!(
            DefaultView::parse("board:backlog,todo,in_progress,review,done"),
            Some(DefaultView::BOARD)
        );
        // Parsing is set-like: order and repeats don't matter.
        assert_eq!(
            DefaultView::parse("board:done,in_progress,done"),
            Some(DefaultView {
                board: true,
                lanes: two_lanes,
            })
        );
        // The list lens carries no lane state.
        assert_eq!(
            DefaultView {
                board: false,
                lanes: no_done,
            }
            .encode(),
            "list"
        );
        // An empty set never reaches the register.
        assert_eq!(
            DefaultView {
                board: true,
                lanes: LaneSet::NONE,
            }
            .encode(),
            "board"
        );
        // A value written by a future client reads as "no saved
        // default" rather than something wrong.
        assert_eq!(DefaultView::parse("calendar"), None);
        assert_eq!(DefaultView::parse("board:"), None);
        assert_eq!(DefaultView::parse("board:blocked"), None);
        assert_eq!(DefaultView::parse("board:done,"), None);
        assert_eq!(DefaultView::parse(""), None);
    }

    #[test]
    fn remote_default_view_change_emits_event() {
        let dek = Dek::generate();
        let mut a = Doc::new().unwrap();
        let list = a.add_list("Work").unwrap();
        let seed = a.pending_export(&dek).unwrap().unwrap();
        a.mark_persisted();
        let mut b = Doc::empty();
        b.apply_remote(&dek, &seed).unwrap();
        let _ = b.drain_events();

        a.set_default_view(&list, Some(DefaultView::BOARD)).unwrap();
        let blob = a.pending_export(&dek).unwrap().unwrap();
        b.apply_remote(&dek, &blob).unwrap();
        assert_eq!(
            b.get_list_meta(&list).unwrap().default_view,
            Some(DefaultView::BOARD)
        );
        assert!(b.drain_events().iter().any(|e| matches!(
            e,
            AppEvent::ListDefaultViewChanged { id, view: Some(v) }
                if id == &list && *v == DefaultView::BOARD
        )));
    }

    #[test]
    fn export_import_carries_default_view() {
        let src = Doc::new().unwrap();
        let list = src.add_list("Work").unwrap();
        let nodone = DefaultView {
            board: true,
            lanes: LaneSet::ALL.with(WorkflowState::Done, false),
        };
        src.set_default_view(&list, Some(nodone)).unwrap();
        src.set_default_view(LIST_INBOX, Some(DefaultView::BOARD))
            .unwrap();
        src.add_item(&list, "a").unwrap();

        let export = src.export_json();
        assert_eq!(export.settings.inbox_view.as_deref(), Some("board"));
        let exported = export
            .lists
            .iter()
            .find(|l| l.id == list)
            .expect("user list exported");
        assert_eq!(
            exported.view.as_deref(),
            Some("board:backlog,todo,in_progress,review")
        );

        let dst = Doc::new().unwrap();
        dst.import_json(&export).unwrap();
        let imported = dst
            .all_lists()
            .into_iter()
            .find(|l| l.name == "Work")
            .expect("user list imported");
        assert_eq!(imported.default_view, Some(nodone));
    }

    #[test]
    fn import_drops_unknown_default_view() {
        let export = JsonExport {
            version: 1,
            settings: ExportSettings {
                show_list_counts: false,
                inbox_view: None,
            },
            lists: vec![ExportList {
                id: "src-list".to_string(),
                name: "Work".to_string(),
                icon: None,
                view: Some("hologram".to_string()),
                archived_at: None,
                created_at: Some(1),
                builtin: false,
            }],
            items: vec![],
            focus: vec![],
        };
        let dst = Doc::new().unwrap();
        dst.import_json(&export).unwrap();
        let imported = dst
            .all_lists()
            .into_iter()
            .find(|l| l.name == "Work")
            .expect("user list imported");
        assert_eq!(imported.default_view, None);
    }

    // ---------- archive (`spec/data-model.md` "Archived lists") ----------

    #[test]
    fn archive_round_trips_and_is_noop_when_unchanged() {
        let doc = Doc::new().unwrap();
        let list = doc.add_list("Work").unwrap();
        let _ = doc.drain_events();

        // Archive → timestamp stored, exactly one archive event.
        doc.set_list_archived(&list, true).unwrap();
        let archived_at = doc.get_list_meta(&list).unwrap().archived_at;
        assert!(archived_at.is_some());
        let evs = doc.drain_events();
        assert_eq!(evs.len(), 1, "exactly one event expected: {evs:?}");
        assert!(matches!(
            &evs[0],
            AppEvent::ListArchivedChanged { id, archived_at: at }
                if id == &list && *at == archived_at
        ));

        // Re-archiving is a no-op: no commit, no event, timestamp kept.
        doc.set_list_archived(&list, true).unwrap();
        assert!(doc.drain_events().is_empty());
        assert_eq!(doc.get_list_meta(&list).unwrap().archived_at, archived_at);

        // Unarchive → key deleted, exactly the inverse event.
        doc.set_list_archived(&list, false).unwrap();
        assert_eq!(doc.get_list_meta(&list).unwrap().archived_at, None);
        let evs = doc.drain_events();
        assert_eq!(evs.len(), 1, "exactly one event expected: {evs:?}");
        assert!(matches!(
            &evs[0],
            AppEvent::ListArchivedChanged {
                id,
                archived_at: None
            } if id == &list
        ));

        // Re-unarchiving is a no-op too.
        doc.set_list_archived(&list, false).unwrap();
        assert!(doc.drain_events().is_empty());
    }

    #[test]
    fn archive_refuses_inbox() {
        let doc = Doc::new().unwrap();
        assert!(matches!(
            doc.set_list_archived(LIST_INBOX, true).unwrap_err(),
            DocError::CannotDeleteBuiltin(_)
        ));
        assert!(matches!(
            doc.set_list_archived(LIST_INBOX, false).unwrap_err(),
            DocError::CannotDeleteBuiltin(_)
        ));
    }

    #[test]
    fn archive_preserves_items_metadata_and_focus() {
        let doc = Doc::new().unwrap();
        let list = doc.add_list("Work").unwrap();
        doc.set_list_icon(&list, "🎯").unwrap();
        doc.set_default_view(&list, Some(DefaultView::BOARD))
            .unwrap();

        // A list with every lifecycle represented, notes, a deadline, a
        // manual reorder, and a Focus ref.
        let a = doc.add_item(&list, "a").unwrap();
        let b = doc.add_item(&list, "b").unwrap();
        let c = doc.add_item(&list, "c").unwrap();
        let d = doc.add_item(&list, "d").unwrap();
        doc.edit_item_notes(&a, "some notes").unwrap();
        doc.set_item_deadline(&a, Some("2026-09-01")).unwrap();
        doc.set_item_lifecycle(&b, ItemLifecycle::InProgress)
            .unwrap();
        doc.set_item_done(&c, true).unwrap();
        doc.set_item_binned(&d, true).unwrap();
        doc.move_item(&a, &list, 1).unwrap();
        doc.add_to_focus(&a, 0).unwrap();

        let pre_meta = doc.get_list_meta(&list).unwrap();
        let pre_items: Vec<ItemView> = doc.items_in_list(&list, true);
        let pre_resolved: Vec<String> = pre_items.iter().map(|i| i.id.clone()).collect();
        let pre_focus = doc.focus_refs();
        let pre_locations: Vec<String> = pre_resolved
            .iter()
            .map(|id| read_string(&doc.find_item(id).unwrap(), KEY_LOCATION).unwrap())
            .collect();
        let _ = doc.drain_events();

        doc.set_list_archived(&list, true).unwrap();

        // Exactly one event — no item / order / lifecycle / focus events.
        let evs = doc.drain_events();
        assert_eq!(evs.len(), 1, "archive must be metadata-only: {evs:?}");
        assert!(matches!(&evs[0], AppEvent::ListArchivedChanged { .. }));

        // Every item field, location (list + placement), and the
        // resolved order survive untouched.
        let post_items: Vec<ItemView> = doc.items_in_list(&list, true);
        assert_eq!(post_items, pre_items);
        let post_locations: Vec<String> = pre_resolved
            .iter()
            .map(|id| read_string(&doc.find_item(id).unwrap(), KEY_LOCATION).unwrap())
            .collect();
        assert_eq!(post_locations, pre_locations);
        assert_eq!(doc.focus_refs(), pre_focus);

        // List metadata survives; only `archived_at` changed.
        let post_meta = doc.get_list_meta(&list).unwrap();
        assert_eq!(post_meta.name, pre_meta.name);
        assert_eq!(post_meta.icon, pre_meta.icon);
        assert_eq!(post_meta.default_view, pre_meta.default_view);
        assert_eq!(post_meta.created_at, pre_meta.created_at);
        assert!(post_meta.archived_at.is_some());

        // The canonical projection still contains the archived list.
        assert!(doc.all_lists().iter().any(|l| l.id == list));
        assert!(doc.active_lists().iter().all(|l| l.id != list));

        // Unarchive restores the exact pre-archive metadata.
        doc.set_list_archived(&list, false).unwrap();
        assert_eq!(doc.get_list_meta(&list).unwrap(), pre_meta);
        assert_eq!(doc.items_in_list(&list, true), pre_items);
        assert_eq!(doc.focus_refs(), pre_focus);
    }

    #[test]
    fn archive_undo_redo_round_trips() {
        let doc = Doc::new().unwrap();
        let list = doc.add_list("Work").unwrap();
        doc.set_list_archived(&list, true).unwrap();
        let archived_at = doc.get_list_meta(&list).unwrap().archived_at;
        let _ = doc.drain_events();

        assert!(doc.undo().unwrap());
        assert_eq!(doc.get_list_meta(&list).unwrap().archived_at, None);
        assert!(doc.drain_events().iter().any(|e| matches!(
            e,
            AppEvent::ListArchivedChanged {
                id,
                archived_at: None
            } if id == &list
        )));

        assert!(doc.redo().unwrap());
        assert_eq!(doc.get_list_meta(&list).unwrap().archived_at, archived_at);
        assert!(doc.drain_events().iter().any(|e| matches!(
            e,
            AppEvent::ListArchivedChanged { id, archived_at: at }
                if id == &list && *at == archived_at
        )));
    }

    #[test]
    fn remote_archive_change_emits_event() {
        let dek = Dek::generate();
        let mut a = Doc::new().unwrap();
        let list = a.add_list("Work").unwrap();
        let seed = a.pending_export(&dek).unwrap().unwrap();
        a.mark_persisted();
        let mut b = Doc::empty();
        b.apply_remote(&dek, &seed).unwrap();
        let _ = b.drain_events();

        a.set_list_archived(&list, true).unwrap();
        let expected = a.get_list_meta(&list).unwrap().archived_at;
        let blob = a.pending_export(&dek).unwrap().unwrap();
        b.apply_remote(&dek, &blob).unwrap();
        assert_eq!(b.get_list_meta(&list).unwrap().archived_at, expected);
        assert!(b.drain_events().iter().any(|e| matches!(
            e,
            AppEvent::ListArchivedChanged { id, archived_at: at }
                if id == &list && *at == expected
        )));
    }

    #[test]
    fn snapshot_events_carry_archive_state() {
        let doc = Doc::new().unwrap();
        let active = doc.add_list("Active").unwrap();
        let archived = doc.add_list("Old").unwrap();
        doc.set_list_archived(&archived, true).unwrap();
        let expected = doc.get_list_meta(&archived).unwrap().archived_at;

        let evs = doc.snapshot_events();
        assert!(evs.iter().any(|e| matches!(
            e,
            AppEvent::ListAdded {
                id,
                archived_at: None,
                ..
            } if id == &active
        )));
        assert!(evs.iter().any(|e| matches!(
            e,
            AppEvent::ListAdded { id, archived_at: at, .. }
                if id == &archived && *at == expected
        )));
    }

    #[test]
    fn export_import_round_trips_archived_at() {
        let src = Doc::new().unwrap();
        let list = src.add_list("Old").unwrap();
        src.set_list_archived(&list, true).unwrap();
        let expected = src.get_list_meta(&list).unwrap().archived_at;

        let export = src.export_json();
        let exported = export
            .lists
            .iter()
            .find(|l| l.id == list)
            .expect("user list exported");
        assert_eq!(exported.archived_at, expected);

        let dst = Doc::new().unwrap();
        dst.import_json(&export).unwrap();
        let imported = dst
            .all_lists()
            .into_iter()
            .find(|l| l.name == "Old")
            .expect("user list imported");
        assert_eq!(imported.archived_at, expected);
    }

    #[test]
    fn old_exports_without_archived_at_import_as_active() {
        // A pre-archive export (no `archivedAt` on any list) must parse
        // and import every list as active.
        let json = r#"{
            "version": 1,
            "settings": { "showListCounts": false },
            "lists": [
                { "id": "inbox", "name": "Inbox", "createdAt": null, "builtin": true },
                { "id": "src", "name": "Work", "createdAt": 1, "builtin": false }
            ],
            "items": []
        }"#;
        let doc = Doc::new().unwrap();
        doc.import_json_str(json).unwrap();
        let imported = doc
            .all_lists()
            .into_iter()
            .find(|l| l.name == "Work")
            .expect("user list imported");
        assert_eq!(imported.archived_at, None);
    }

    #[test]
    fn fingerprint_includes_archive_state() {
        let doc = Doc::new().unwrap();
        let list = doc.add_list("Work").unwrap();
        let before = doc.fingerprint();
        doc.set_list_archived(&list, true).unwrap();
        assert_ne!(doc.fingerprint(), before, "archive must change the hash");
        doc.set_list_archived(&list, false).unwrap();
        assert_eq!(
            doc.fingerprint(),
            before,
            "unarchive deletes the key, restoring the exact logical state"
        );
    }

    #[test]
    fn move_active_list_addresses_active_projection() {
        let doc = Doc::new().unwrap();
        let a = doc.add_list("A").unwrap();
        let b = doc.add_list("B").unwrap();
        let c = doc.add_list("C").unwrap();
        let d = doc.add_list("D").unwrap();
        // Archive B so the raw container holds an archived row *between*
        // active rows: raw [A, B✕, C, D], active [A, C, D].
        doc.set_list_archived(&b, true).unwrap();
        let _ = doc.drain_events();

        let active_ids =
            |doc: &Doc| -> Vec<String> { doc.active_lists().into_iter().map(|l| l.id).collect() };
        assert_eq!(active_ids(&doc), vec![a.clone(), c.clone(), d.clone()]);

        // Active index 0 for D must land it before A, not at raw slot 0
        // of some archived-aware miscount.
        doc.move_list(&d, 0).unwrap();
        assert_eq!(active_ids(&doc), vec![d.clone(), a.clone(), c.clone()]);
        // The archived row keeps its relative place in the raw order.
        let raw: Vec<String> = doc.all_lists().into_iter().map(|l| l.id).collect();
        assert_eq!(raw, vec![d.clone(), a.clone(), b.clone(), c.clone()]);
        // The emitted index addresses the full `all_lists` projection.
        let evs = doc.drain_events();
        assert!(matches!(
            &evs[..],
            [AppEvent::ListMoved { id, index: 0 }] if id == &d
        ));

        // Move D to the middle of the actives (active index 1: between A
        // and C, which straddles the archived B in raw order).
        doc.move_list(&d, 1).unwrap();
        assert_eq!(active_ids(&doc), vec![a.clone(), d.clone(), c.clone()]);

        // Past-end lands after the last active list.
        doc.move_list(&a, 99).unwrap();
        assert_eq!(active_ids(&doc), vec![d.clone(), c.clone(), a.clone()]);

        // Moving to the slot it already occupies is a no-op.
        let _ = doc.drain_events();
        doc.move_list(&a, 2).unwrap();
        assert!(doc.drain_events().is_empty());
    }

    #[test]
    fn remote_icon_change_emits_list_icon_changed() {
        let dek = Dek::generate();
        let mut a = Doc::new().unwrap();
        let list = a.add_list("Work").unwrap();
        let seed = a.pending_export(&dek).unwrap().unwrap();
        a.mark_persisted();
        let mut b = Doc::empty();
        b.apply_remote(&dek, &seed).unwrap();
        let _ = b.drain_events();

        // A sets an icon; the op replays into B as a ListIconChanged.
        a.set_list_icon(&list, "🎯").unwrap();
        let blob = a.pending_export(&dek).unwrap().unwrap();
        b.apply_remote(&dek, &blob).unwrap();
        assert_eq!(b.get_list_meta(&list).unwrap().icon.as_deref(), Some("🎯"));
        assert!(b.drain_events().iter().any(|e| matches!(
            e,
            AppEvent::ListIconChanged { id, icon } if id == &list && icon.as_deref() == Some("🎯")
        )));
    }

    #[test]
    fn add_item_round_trips_through_get() {
        let doc = Doc::new().unwrap();
        let id = doc.add_item(LIST_INBOX, "buy milk").unwrap();
        let view = doc.get_item(&id).unwrap();
        assert_eq!(view.text, "buy milk");
        assert_eq!(view.list_id, LIST_INBOX);
        assert!(!view.is_done());
        assert!(!view.is_binned());
    }

    #[test]
    fn empty_text_rejected() {
        let doc = Doc::new().unwrap();
        let err = doc.add_item(LIST_INBOX, "   ").unwrap_err();
        assert!(matches!(err, DocError::Invalid(_)));
    }

    #[test]
    fn add_to_unknown_list_rejected() {
        let doc = Doc::new().unwrap();
        let err = doc.add_item("does-not-exist", "x").unwrap_err();
        assert!(matches!(err, DocError::ListNotFound(_)));
    }

    #[test]
    fn done_and_binned_are_orthogonal() {
        let doc = Doc::new().unwrap();
        let id = doc.add_item(LIST_INBOX, "thing").unwrap();
        doc.set_item_done(&id, true).unwrap();
        assert!(doc.get_item(&id).unwrap().is_done());
        // Binning a done item must keep it done.
        doc.set_item_binned(&id, true).unwrap();
        let v = doc.get_item(&id).unwrap();
        assert!(v.is_done(), "done state must survive binning");
        assert!(v.is_binned());
        // Restoring (unbinning) must keep it done.
        doc.set_item_binned(&id, false).unwrap();
        let v = doc.get_item(&id).unwrap();
        assert!(v.is_done(), "done state must survive restore");
        assert!(!v.is_binned());
        // Unmarking done leaves binned alone (already false here).
        doc.set_item_done(&id, false).unwrap();
        let v = doc.get_item(&id).unwrap();
        assert!(!v.is_done());
        assert!(!v.is_binned());
    }

    #[test]
    fn set_done_idempotent() {
        let doc = Doc::new().unwrap();
        let id = doc.add_item(LIST_INBOX, "thing").unwrap();
        doc.set_item_done(&id, true).unwrap();
        let first = doc.get_item(&id).unwrap().done_at.unwrap();
        let _ = doc.drain_events();
        doc.set_item_done(&id, true).unwrap();
        assert_eq!(doc.get_item(&id).unwrap().done_at, Some(first));
        assert!(doc.drain_events().is_empty(), "no-op must not emit events");
    }

    #[test]
    fn lifecycle_flips_do_not_touch_order_containers() {
        // Decision pinned by spec/data-model.md: done/binned are pure
        // item-map writes; the entry stays where it is so restore is
        // exact-position for free.
        let doc = Doc::new().unwrap();
        let a = doc.add_item(LIST_INBOX, "a").unwrap();
        let b = doc.add_item(LIST_INBOX, "b").unwrap();
        let c = doc.add_item(LIST_INBOX, "c").unwrap();
        let order_len_before = doc.order_list(LIST_INBOX).len();

        doc.set_item_done(&b, true).unwrap();
        assert_eq!(doc.order_list(LIST_INBOX).len(), order_len_before);
        assert_eq!(doc.open_item_ids(LIST_INBOX), vec![a.clone(), c.clone()]);

        doc.set_item_done(&b, false).unwrap();
        assert_eq!(doc.order_list(LIST_INBOX).len(), order_len_before);
        assert_eq!(doc.open_item_ids(LIST_INBOX), vec![a, b, c]);
        assert_open_projection_matches_doc(&doc);
    }

    #[test]
    fn empty_bin_removes_only_binned() {
        let doc = Doc::new().unwrap();
        let a = doc.add_item(LIST_INBOX, "keep").unwrap();
        let b = doc.add_item(LIST_INBOX, "drop").unwrap();
        doc.set_item_binned(&b, true).unwrap();
        let removed = doc.empty_bin().unwrap();
        assert_eq!(removed, 1);
        assert!(doc.get_item(&a).is_some());
        assert!(doc.get_item(&b).is_none());
    }

    #[test]
    fn delete_binned_only_works_for_binned() {
        let doc = Doc::new().unwrap();
        let id = doc.add_item(LIST_INBOX, "live").unwrap();
        assert!(matches!(
            doc.delete_binned(&id).unwrap_err(),
            DocError::NotBinned
        ));
        doc.set_item_binned(&id, true).unwrap();
        doc.delete_binned(&id).unwrap();
        assert!(doc.get_item(&id).is_none());
    }

    #[test]
    fn hard_delete_removes_order_entries() {
        let doc = Doc::new().unwrap();
        let a = doc.add_item(LIST_INBOX, "a").unwrap();
        let b = doc.add_item(LIST_INBOX, "b").unwrap();
        doc.set_item_binned(&b, true).unwrap();
        assert_eq!(doc.order_list(LIST_INBOX).len(), 2);
        doc.delete_binned(&b).unwrap();
        assert_eq!(doc.order_list(LIST_INBOX).len(), 1);
        assert_eq!(doc.open_item_ids(LIST_INBOX), vec![a]);
        assert_open_projection_matches_doc(&doc);
    }

    #[test]
    fn delete_list_bins_items_and_relocates_to_main() {
        let doc = Doc::new().unwrap();
        let mylist = doc.add_list("Errands").unwrap();
        let id = doc.add_item(&mylist, "milk").unwrap();
        doc.delete_list(&mylist).unwrap();
        let view = doc.get_item(&id).unwrap();
        assert_eq!(view.list_id, LIST_INBOX);
        assert!(view.is_binned(), "deleting a list bins its items");
    }

    #[test]
    fn delete_list_bins_open_done_and_binned_items_with_fresh_placements() {
        let doc = Doc::new().unwrap();
        let mylist = doc.add_list("Errands").unwrap();
        let open = doc.add_item(&mylist, "live").unwrap();
        let done = doc.add_item(&mylist, "done").unwrap();
        let binned = doc.add_item(&mylist, "binned").unwrap();
        doc.set_item_done(&done, true).unwrap();
        doc.set_item_binned(&binned, true).unwrap();
        let main_existing = doc.add_item(LIST_INBOX, "already here").unwrap();

        doc.delete_list(&mylist).unwrap();

        for id in [&open, &done, &binned] {
            assert_eq!(doc.get_item(id).unwrap().list_id, LIST_INBOX);
        }
        // Everything from the deleted list is now binned, so the open
        // projection of main is unchanged (only the pre-existing item).
        assert_eq!(doc.open_item_ids(LIST_INBOX), vec![main_existing]);
        // The formerly-done item is now done *and* binned, so it leaves
        // the done-only view; all three land in the bin.
        assert_eq!(doc.done_item_ids(), Vec::<String>::new());
        let mut in_bin = doc.binned_item_ids();
        in_bin.sort();
        let mut expected = vec![open, done, binned];
        expected.sort();
        assert_eq!(in_bin, expected);
        assert_open_projection_matches_doc(&doc);
    }

    #[test]
    fn delete_main_refused() {
        let doc = Doc::new().unwrap();
        assert!(matches!(
            doc.delete_list(LIST_INBOX).unwrap_err(),
            DocError::CannotDeleteBuiltin(_)
        ));
    }

    #[test]
    fn new_doc_has_show_list_counts_on() {
        let doc = Doc::new().unwrap();
        assert!(doc.get_settings().show_list_counts);
    }

    #[test]
    fn show_list_counts_round_trips() {
        let doc = Doc::new().unwrap();
        // Default is on; the opt-out (`false`) is what gets persisted.
        doc.set_show_list_counts(false).unwrap();
        assert!(!doc.get_settings().show_list_counts);
        let bytes = doc.save().unwrap();
        let restored = Doc::load(&bytes).unwrap();
        assert!(!restored.get_settings().show_list_counts);
        // Toggling back on drops the key — verify the round-trip to the
        // default.
        doc.set_show_list_counts(true).unwrap();
        assert!(doc.get_settings().show_list_counts);
    }

    #[test]
    fn show_list_counts_idempotent() {
        let doc = Doc::new().unwrap();
        let _ = doc.drain_events();
        // On is the default, so setting it on is a no-op.
        doc.set_show_list_counts(true).unwrap();
        assert!(
            doc.drain_events().is_empty(),
            "no-op toggle must not emit events"
        );
        doc.set_show_list_counts(false).unwrap();
        let evs = doc.drain_events();
        assert!(matches!(
            evs.as_slice(),
            [AppEvent::SettingsChanged {
                show_list_counts: false,
                ..
            }]
        ));
        doc.set_show_list_counts(false).unwrap();
        assert!(
            doc.drain_events().is_empty(),
            "second toggle to same value must not re-emit"
        );
    }

    #[test]
    fn save_load_round_trip_preserves_state() {
        let doc = Doc::new().unwrap();
        let id = doc.add_item(LIST_INBOX, "persisted").unwrap();
        let bytes = doc.save().unwrap();
        let restored = Doc::load(&bytes).unwrap();
        assert_eq!(restored.get_item(&id).unwrap().text, "persisted");
        assert_eq!(restored.fingerprint(), doc.fingerprint());
    }

    #[test]
    fn pending_export_is_none_when_clean() {
        let mut doc = Doc::new().unwrap();
        doc.mark_persisted();
        let dek = Dek::generate();
        assert!(doc.pending_export(&dek).unwrap().is_none());
    }

    #[test]
    fn two_replicas_converge_via_op_exchange() {
        let dek = Dek::generate();

        // Replica A is the originator and creates the first item.
        let mut a = Doc::new().unwrap();
        let item_a = a.add_item(LIST_INBOX, "from A").unwrap();

        // Replica B starts empty. Real device-2 bootstrap typically
        // uses snapshot, but the convergence guarantee is what we're
        // testing.
        let mut b = Doc::empty();
        let blob_a1 = a.pending_export(&dek).unwrap().unwrap();
        a.mark_persisted();
        b.apply_remote(&dek, &blob_a1).unwrap();
        assert!(b.get_item(&item_a).is_some());

        // B mutates concurrently, ships back to A.
        let item_b = b.add_item(LIST_INBOX, "from B").unwrap();
        let blob_b1 = b.pending_export(&dek).unwrap().unwrap();
        b.mark_persisted();
        a.apply_remote(&dek, &blob_b1).unwrap();
        assert!(a.get_item(&item_b).is_some());

        assert_eq!(a.fingerprint(), b.fingerprint());
        assert_open_projection_matches_doc(&a);
        assert_open_projection_matches_doc(&b);
    }

    #[test]
    fn fingerprint_diverges_when_state_diverges() {
        let mut a = Doc::new().unwrap();
        let mut b = Doc::new().unwrap();
        let _ = a.add_item(LIST_INBOX, "A only").unwrap();
        let _ = b.add_item(LIST_INBOX, "B only").unwrap();
        a.mark_persisted();
        b.mark_persisted();
        assert_ne!(a.fingerprint(), b.fingerprint());
    }

    #[test]
    fn fingerprint_diverges_when_order_diverges() {
        let a = Doc::new().unwrap();
        let b = Doc::new().unwrap();
        let a1 = a.add_item(LIST_INBOX, "one").unwrap();
        let a2 = a.add_item(LIST_INBOX, "two").unwrap();
        let b1 = b.add_item(LIST_INBOX, "one").unwrap();
        let b2 = b.add_item(LIST_INBOX, "two").unwrap();

        a.move_item(&a1, LIST_INBOX, 1).unwrap();

        assert_eq!(a.open_item_ids(LIST_INBOX), vec![a2, a1]);
        assert_eq!(b.open_item_ids(LIST_INBOX), vec![b1, b2]);
        assert_ne!(a.fingerprint(), b.fingerprint());
    }

    #[test]
    fn repeated_moves_keep_index_in_sync() {
        let doc = Doc::new().unwrap();
        let _a = doc.add_item(LIST_INBOX, "a").unwrap();
        let b = doc.add_item(LIST_INBOX, "b").unwrap();
        let _c = doc.add_item(LIST_INBOX, "c").unwrap();

        for target in [0, 2, 0, 2, 1, 0, 2, 0, 1, 2] {
            doc.move_item(&b, LIST_INBOX, target).unwrap();
            assert_eq!(
                doc.open_item_ids(LIST_INBOX).iter().position(|id| id == &b),
                Some(target)
            );
            assert_open_projection_matches_doc(&doc);
        }
    }

    #[test]
    fn cross_list_move_preserves_identity_and_content() {
        let doc = Doc::new().unwrap();
        let other = doc.add_list("Other").unwrap();
        let id = doc.add_item(LIST_INBOX, "wandering").unwrap();
        doc.edit_item_notes(&id, "some notes").unwrap();
        let before = doc.get_item(&id).unwrap();

        doc.move_item(&id, &other, 0).unwrap();
        let after = doc.get_item(&id).unwrap();

        assert_eq!(after.id, before.id);
        assert_eq!(after.text, before.text);
        assert_eq!(after.notes, before.notes);
        assert_eq!(after.created_at, before.created_at);
        assert_eq!(after.list_id, other);
        assert_eq!(doc.open_item_ids(&other), vec![id]);
        assert!(doc.open_item_ids(LIST_INBOX).is_empty());
        assert_open_projection_matches_doc(&doc);
    }

    #[test]
    fn cross_list_move_removes_source_entry_best_effort() {
        let doc = Doc::new().unwrap();
        let other = doc.add_list("Other").unwrap();
        let id = doc.add_item(LIST_INBOX, "x").unwrap();
        assert_eq!(doc.order_list(LIST_INBOX).len(), 1);
        doc.move_item(&id, &other, 0).unwrap();
        assert_eq!(
            doc.order_list(LIST_INBOX).len(),
            0,
            "source order entry cleaned up"
        );
        assert_eq!(doc.order_list(&other).len(), 1);
    }

    #[test]
    fn open_projection_survives_bulk_archive_then_reorder() {
        let doc = Doc::new().unwrap();
        let historical = doc
            .add_items_at(LIST_INBOX, &["old 1", "old 2", "old 3"], 0)
            .unwrap();
        let historical_refs: Vec<&str> = historical.iter().map(String::as_str).collect();
        doc.set_items_done(&historical_refs, true).unwrap();

        let open = doc
            .add_items_at(LIST_INBOX, &["current 1", "current 2", "current 3"], 0)
            .unwrap();
        doc.move_item(&open[2], LIST_INBOX, 0).unwrap();

        assert_eq!(
            doc.open_item_ids(LIST_INBOX),
            vec![open[2].clone(), open[0].clone(), open[1].clone()]
        );
        assert_open_projection_matches_doc(&doc);
    }

    #[test]
    fn view_helpers_empty_doc() {
        let doc = Doc::new().unwrap();
        assert_eq!(doc.open_item_ids(LIST_INBOX), Vec::<String>::new());
        assert_eq!(doc.done_item_ids(), Vec::<String>::new());
        assert_eq!(doc.binned_item_ids(), Vec::<String>::new());
    }

    #[test]
    fn open_item_ids_match_order_container() {
        let doc = Doc::new().unwrap();
        let other = doc.add_list("Other").unwrap();
        let a = doc.add_item(LIST_INBOX, "a").unwrap();
        let b = doc.add_item(LIST_INBOX, "b").unwrap();
        let c = doc.add_item(LIST_INBOX, "c").unwrap();
        // Items in another list must not leak into main's view.
        let _h = doc.add_item(&other, "h").unwrap();
        // Done/binned items must not leak into the open view.
        doc.set_item_done(&b, true).unwrap();
        let d = doc.add_item(LIST_INBOX, "d").unwrap();
        doc.set_item_binned(&d, true).unwrap();

        assert_eq!(doc.open_item_ids(LIST_INBOX), vec![a, c]);
    }

    #[test]
    fn done_item_ids_sorted_by_done_at_desc() {
        let doc = Doc::new().unwrap();
        let other = doc.add_list("Other").unwrap();
        let first = doc.add_item(LIST_INBOX, "first").unwrap();
        let second = doc.add_item(&other, "second").unwrap();
        let third = doc.add_item(LIST_INBOX, "third").unwrap();
        doc.set_item_done(&first, true).unwrap();
        // tiny gap so the millisecond timestamps definitely differ
        std::thread::sleep(std::time::Duration::from_millis(2));
        doc.set_item_done(&second, true).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(2));
        doc.set_item_done(&third, true).unwrap();

        assert_eq!(doc.done_item_ids(), vec![third, second, first]);
    }

    #[test]
    fn binned_item_ids_sorted_by_binned_at_desc() {
        let doc = Doc::new().unwrap();
        let a = doc.add_item(LIST_INBOX, "a").unwrap();
        let b = doc.add_item(LIST_INBOX, "b").unwrap();
        doc.set_item_binned(&a, true).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(2));
        doc.set_item_binned(&b, true).unwrap();
        assert_eq!(doc.binned_item_ids(), vec![b, a]);
    }

    #[test]
    fn deleted_list_items_land_binned_under_main() {
        let doc = Doc::new().unwrap();
        let mylist = doc.add_list("Errands").unwrap();
        let id = doc.add_item(&mylist, "x").unwrap();
        doc.delete_list(&mylist).unwrap();
        // Items are discarded to the bin: gone from every open view,
        // relocated to main, and present in the cross-list bin view.
        assert!(doc.open_item_ids(&mylist).is_empty());
        assert!(doc.open_item_ids(LIST_INBOX).is_empty());
        assert_eq!(doc.get_item(&id).unwrap().list_id, LIST_INBOX);
        assert_eq!(doc.binned_item_ids(), vec![id]);
    }

    #[test]
    fn json_export_includes_builtin_and_user_lists() {
        let doc = Doc::new().unwrap();
        let errands = doc.add_list("Errands").unwrap();
        doc.set_show_list_counts(true).unwrap();

        let export = doc.export_json();

        assert_eq!(export.version, 1);
        assert!(export.settings.show_list_counts);
        assert_eq!(
            export.lists[0],
            ExportList {
                id: LIST_INBOX.to_string(),
                name: INBOX_NAME.to_string(),
                icon: None,
                view: None,
                archived_at: None,
                created_at: None,
                builtin: true,
            }
        );
        assert_eq!(export.lists[1].id, errands);
        assert_eq!(export.lists[1].name, "Errands");
        assert_eq!(export.lists[1].builtin, false);
        assert!(export.lists[1].created_at.is_some());
    }

    #[test]
    fn json_export_includes_notes_and_lifecycle_timestamps() {
        let doc = Doc::new().unwrap();
        let id = doc.add_item(LIST_INBOX, "buy milk").unwrap();
        doc.edit_item_notes(&id, "whole milk").unwrap();
        doc.set_item_done(&id, true).unwrap();
        doc.set_item_binned(&id, true).unwrap();

        let export = doc.export_json();
        let item = export.items.iter().find(|item| item.id == id).unwrap();

        assert_eq!(item.text, "buy milk");
        assert_eq!(item.notes, "whole milk");
        assert_eq!(item.list_id, LIST_INBOX);
        assert!(item.done_at.is_some());
        assert!(item.binned_at.is_some());
    }

    #[test]
    fn move_open_item_uses_visible_target_index() {
        let doc = Doc::new().unwrap();
        let other = doc.add_list("Other").unwrap();
        let hidden = doc.add_item(&other, "hidden").unwrap();
        doc.set_item_done(&hidden, true).unwrap();
        let anchor = doc.add_item(&other, "anchor").unwrap();
        let moved = doc.add_item(LIST_INBOX, "moved").unwrap();

        doc.move_item(&moved, &other, 1).unwrap();

        assert_eq!(doc.open_item_ids(&other), vec![anchor, moved]);
    }

    #[test]
    fn get_list_meta_returns_view() {
        let doc = Doc::new().unwrap();
        // Main has no ListMeta row, so no metadata in the doc —
        // clients render its label themselves.
        assert!(doc.get_list_meta(LIST_INBOX).is_none());
        assert!(doc.get_list_meta("nope").is_none());
    }

    #[test]
    fn apply_remote_rejects_wrong_dek() {
        let dek1 = Dek::generate();
        let dek2 = Dek::generate();
        let a = Doc::new().unwrap();
        let _ = a.add_item(LIST_INBOX, "x").unwrap();
        let blob = a.pending_export(&dek1).unwrap().unwrap();

        let mut b = Doc::empty();
        let err = b.apply_remote(&dek2, &blob).unwrap_err();
        assert!(matches!(err, DocError::Crypto(_)));
    }

    // ---------- stale / duplicate / missing order entries ----------

    /// Concurrent cross-list moves of the same item: both replicas
    /// insert an entry into a different order container; the item's
    /// atomic location register picks one winner and the loser's entry
    /// goes stale. Exactly one visible item, on both replicas.
    #[test]
    fn concurrent_cross_list_moves_converge_to_one_visible_item() {
        let dek = Dek::generate();
        let mut a = Doc::new().unwrap();
        let list_x = a.add_list("X").unwrap();
        let list_y = a.add_list("Y").unwrap();
        let id = a.add_item(LIST_INBOX, "contested").unwrap();
        let seed = a.pending_export(&dek).unwrap().unwrap();
        a.mark_persisted();
        let mut b = Doc::empty();
        b.apply_remote(&dek, &seed).unwrap();

        // Concurrent: A moves it to X, B moves it to Y.
        a.move_item(&id, &list_x, 0).unwrap();
        b.move_item(&id, &list_y, 0).unwrap();
        let blob_a = a.pending_export(&dek).unwrap().unwrap();
        a.mark_persisted();
        let blob_b = b.pending_export(&dek).unwrap().unwrap();
        b.mark_persisted();
        a.apply_remote(&dek, &blob_b).unwrap();
        b.apply_remote(&dek, &blob_a).unwrap();

        assert_eq!(a.fingerprint(), b.fingerprint());
        let winner = a.get_item(&id).unwrap().list_id;
        assert!(winner == list_x || winner == list_y);
        let visible_in = |d: &Doc| {
            [LIST_INBOX, list_x.as_str(), list_y.as_str()]
                .iter()
                .filter(|l| d.open_item_ids(l).contains(&id))
                .count()
        };
        assert_eq!(visible_in(&a), 1, "exactly one visible copy on A");
        assert_eq!(visible_in(&b), 1, "exactly one visible copy on B");
        assert_open_projection_matches_doc(&a);
        assert_open_projection_matches_doc(&b);
    }

    /// Concurrent reorder on one replica and lifecycle change on another
    /// must merge cleanly: the order mov and the lifecycle flip touch
    /// disjoint containers.
    #[test]
    fn concurrent_reorder_and_lifecycle_change_converge() {
        let dek = Dek::generate();
        let mut a = Doc::new().unwrap();
        let ids: Vec<String> = ["a", "b", "c", "d"]
            .iter()
            .map(|t| a.add_item(LIST_INBOX, t).unwrap())
            .collect();
        let seed = a.pending_export(&dek).unwrap().unwrap();
        a.mark_persisted();
        let mut b = Doc::empty();
        b.apply_remote(&dek, &seed).unwrap();

        a.move_item(&ids[3], LIST_INBOX, 0).unwrap();
        b.set_item_done(&ids[1], true).unwrap();
        let blob_a = a.pending_export(&dek).unwrap().unwrap();
        a.mark_persisted();
        let blob_b = b.pending_export(&dek).unwrap().unwrap();
        b.mark_persisted();
        a.apply_remote(&dek, &blob_b).unwrap();
        b.apply_remote(&dek, &blob_a).unwrap();

        assert_eq!(a.fingerprint(), b.fingerprint());
        assert_eq!(
            a.open_item_ids(LIST_INBOX),
            vec![ids[3].clone(), ids[0].clone(), ids[2].clone()]
        );
        assert!(a.get_item(&ids[1]).unwrap().is_done());
        assert_open_projection_matches_doc(&a);
        assert_open_projection_matches_doc(&b);
    }

    /// A hand-crafted stale entry (placement mismatch) must never make
    /// the item visible, and a duplicate entry must not double it.
    #[test]
    fn stale_and_duplicate_entries_are_invisible() {
        let doc = Doc::new().unwrap();
        let a = doc.add_item(LIST_INBOX, "a").unwrap();
        let b = doc.add_item(LIST_INBOX, "b").unwrap();

        // Duplicate of a's canonical entry + a stale entry with a bogus
        // placement + an entry for a nonexistent item.
        let a_placement = {
            let guard = doc.item_index.lock().unwrap();
            guard.meta[&a].placement_id.clone()
        };
        let order = doc.order_list(LIST_INBOX);
        order
            .push(
                OrderEntry {
                    item_id: a.clone(),
                    placement_id: a_placement,
                }
                .encode()
                .as_str(),
            )
            .unwrap();
        order
            .push(
                OrderEntry {
                    item_id: b.clone(),
                    placement_id: "bogus".to_string(),
                }
                .encode()
                .as_str(),
            )
            .unwrap();
        order.push("no-such-item:whatever").unwrap();
        doc.inner.commit();
        doc.rebuild_index();

        assert_eq!(doc.open_item_ids(LIST_INBOX), vec![a, b]);
        assert_eq!(doc.order_list(LIST_INBOX).len(), 5);
        assert_open_projection_matches_doc(&doc);
    }

    /// An item whose canonical entry is missing entirely still projects
    /// — appended deterministically after entry-backed items — and
    /// `reconcile` materializes a real entry without changing the
    /// visible order.
    #[test]
    fn missing_entry_falls_back_deterministically_and_reconciles() {
        let doc = Doc::new().unwrap();
        let a = doc.add_item(LIST_INBOX, "a").unwrap();
        let b = doc.add_item(LIST_INBOX, "b").unwrap();
        let c = doc.add_item(LIST_INBOX, "c").unwrap();

        // Simulate a lost entry: delete b's canonical entry directly.
        let b_pos = {
            let guard = doc.item_index.lock().unwrap();
            guard.canonical_raw_pos(LIST_INBOX, &b).unwrap()
        };
        doc.order_list(LIST_INBOX).delete(b_pos, 1).unwrap();
        doc.inner.commit();
        doc.rebuild_index();

        // b is not hidden: it lands in the fallback tail (after the
        // entry-backed items, created_at/id order).
        assert_eq!(
            doc.open_item_ids(LIST_INBOX),
            vec![a.clone(), c.clone(), b.clone()]
        );

        // Reads must not have mutated anything.
        assert_eq!(doc.order_list(LIST_INBOX).len(), 2);

        // Reconcile materializes b's entry; visible order unchanged.
        let repairs = doc.reconcile().unwrap();
        assert!(repairs >= 1, "expected at least one repair");
        assert_eq!(doc.order_list(LIST_INBOX).len(), 3);
        assert_eq!(doc.open_item_ids(LIST_INBOX), vec![a, c, b]);
        assert_open_projection_matches_doc(&doc);
        // Second run is a no-op.
        assert_eq!(doc.reconcile().unwrap(), 0);
    }

    #[test]
    fn reconcile_removes_stale_and_duplicate_entries() {
        let doc = Doc::new().unwrap();
        let a = doc.add_item(LIST_INBOX, "a").unwrap();
        let a_placement = {
            let guard = doc.item_index.lock().unwrap();
            guard.meta[&a].placement_id.clone()
        };
        let order = doc.order_list(LIST_INBOX);
        order
            .push(
                OrderEntry {
                    item_id: a.clone(),
                    placement_id: a_placement,
                }
                .encode()
                .as_str(),
            )
            .unwrap();
        order.push("ghost:stale").unwrap();
        doc.inner.commit();
        doc.rebuild_index();
        assert_eq!(doc.order_list(LIST_INBOX).len(), 3);

        let repairs = doc.reconcile().unwrap();
        assert_eq!(repairs, 2);
        assert_eq!(doc.order_list(LIST_INBOX).len(), 1);
        assert_eq!(doc.open_item_ids(LIST_INBOX), vec![a]);
        assert_eq!(doc.reconcile().unwrap(), 0);
        assert_open_projection_matches_doc(&doc);
    }

    // ---------- AppEvent tests ----------

    #[test]
    fn local_add_item_emits_item_added() {
        let doc = Doc::new().unwrap();
        let _ = doc.drain_events();
        let id = doc.add_item(LIST_INBOX, "milk").unwrap();
        let evs = doc.drain_events();
        assert_eq!(evs.len(), 1);
        match &evs[0] {
            AppEvent::ItemAdded {
                id: eid,
                list_id,
                text,
                done_at,
                binned_at,
                open_index,
                ..
            } => {
                assert_eq!(eid, &id);
                assert_eq!(list_id, LIST_INBOX);
                assert_eq!(text, "milk");
                assert!(done_at.is_none());
                assert!(binned_at.is_none());
                assert_eq!(*open_index, Some(0));
            }
            other => panic!("expected ItemAdded, got {other:?}"),
        }
    }

    #[test]
    fn local_edit_text_emits_text_changed() {
        let doc = Doc::new().unwrap();
        let id = doc.add_item(LIST_INBOX, "old").unwrap();
        let _ = doc.drain_events();
        doc.edit_item_text(&id, "new").unwrap();
        let evs = doc.drain_events();
        assert!(matches!(
            evs.as_slice(),
            [AppEvent::ItemTextChanged { id: eid, text }] if eid == &id && text == "new"
        ));
    }

    #[test]
    fn local_set_done_emits_lifecycle_changed_with_timestamps() {
        let doc = Doc::new().unwrap();
        let id = doc.add_item(LIST_INBOX, "task").unwrap();
        let _ = doc.drain_events();
        doc.set_item_done(&id, true).unwrap();
        let evs = doc.drain_events();
        match evs.as_slice() {
            [
                AppEvent::ItemLifecycleChanged {
                    id: eid,
                    done_at,
                    binned_at,
                    open_index,
                    ..
                },
            ] => {
                assert_eq!(eid, &id);
                assert!(done_at.is_some());
                assert!(binned_at.is_none());
                assert_eq!(*open_index, None, "done item leaves the open projection");
            }
            other => panic!("unexpected events: {other:?}"),
        }
    }

    #[test]
    fn local_set_binned_preserves_done_in_event() {
        let doc = Doc::new().unwrap();
        let id = doc.add_item(LIST_INBOX, "task").unwrap();
        doc.set_item_done(&id, true).unwrap();
        let _ = doc.drain_events();
        doc.set_item_binned(&id, true).unwrap();
        let evs = doc.drain_events();
        match evs.as_slice() {
            [
                AppEvent::ItemLifecycleChanged {
                    id: eid,
                    done_at,
                    binned_at,
                    open_index,
                    ..
                },
            ] => {
                assert_eq!(eid, &id);
                assert!(done_at.is_some(), "done state must be preserved");
                assert!(binned_at.is_some());
                assert_eq!(*open_index, None);
            }
            other => panic!("unexpected events: {other:?}"),
        }
    }

    #[test]
    fn local_cross_list_move_emits_list_changed_with_open_index() {
        let doc = Doc::new().unwrap();
        let other = doc.add_list("Other").unwrap();
        let anchor = doc.add_item(&other, "anchor").unwrap();
        let moved = doc.add_item(LIST_INBOX, "moved").unwrap();
        let _ = doc.drain_events();

        doc.move_item(&moved, &other, 1).unwrap();

        let evs = doc.drain_events();
        match evs.as_slice() {
            [
                AppEvent::ItemListChanged {
                    id,
                    list_id,
                    open_index,
                },
            ] => {
                assert_eq!(id, &moved);
                assert_eq!(list_id, &other);
                assert_eq!(*open_index, Some(1));
            }
            other_evs => panic!("expected one ItemListChanged, got {other_evs:?}"),
        }
        assert_eq!(doc.open_item_ids(&other), vec![anchor, moved]);
    }

    #[test]
    fn local_delete_list_bins_items_and_emits_remove() {
        let doc = Doc::new().unwrap();
        let other = doc.add_list("Other").unwrap();
        let id = doc.add_item(&other, "in other").unwrap();
        let _ = doc.drain_events();
        doc.delete_list(&other).unwrap();
        let evs = doc.drain_events();
        // The item is relocated to `inbox` (ItemListChanged) and binned
        // (ItemLifecycleChanged, so it drops out of the open projection),
        // then the list is removed (ListRemoved).
        let mut saw_reassign = false;
        let mut saw_binned = false;
        let mut saw_removed = false;
        for ev in &evs {
            match ev {
                AppEvent::ItemListChanged {
                    id: eid,
                    list_id,
                    open_index,
                } => {
                    assert_eq!(eid, &id);
                    assert_eq!(list_id, LIST_INBOX);
                    assert_eq!(*open_index, None, "binned item is not in a open view");
                    saw_reassign = true;
                }
                AppEvent::ItemLifecycleChanged {
                    id: eid, binned_at, ..
                } => {
                    assert_eq!(eid, &id);
                    assert!(binned_at.is_some(), "item is binned");
                    saw_binned = true;
                }
                AppEvent::ListRemoved { id: lid } => {
                    assert_eq!(lid, &other);
                    saw_removed = true;
                }
                _ => {}
            }
        }
        assert!(saw_reassign && saw_binned && saw_removed, "events: {evs:?}");
    }

    /// Seed a peer doc `b` with `a`'s current state, mark `a` pushed,
    /// and drain both event queues so the next frame's events are the
    /// only thing under test.
    fn sync_fresh_peer(a: &mut Doc, dek: &Dek) -> Doc {
        let blob = a.pending_export(dek).unwrap().expect("seed blob");
        a.mark_persisted();
        let mut b = Doc::empty();
        b.apply_remote(dek, &blob).unwrap();
        let _ = a.drain_events();
        let _ = b.drain_events();
        b
    }

    #[test]
    fn remote_lifecycle_change_translates_to_one_surgical_event() {
        let dek = Dek::generate();
        let mut a = Doc::new().unwrap();
        let first = a.add_item(LIST_INBOX, "first").unwrap();
        let _second = a.add_item(LIST_INBOX, "second").unwrap();
        let mut b = sync_fresh_peer(&mut a, &dek);

        a.set_item_done(&first, true).unwrap();
        let blob = a.pending_export(&dek).unwrap().unwrap();
        b.apply_remote(&dek, &blob).unwrap();

        // Exactly one precise event — a fallback resync would instead
        // emit a FullResync control signal.
        let evs = b.drain_events();
        match evs.as_slice() {
            [
                AppEvent::ItemLifecycleChanged {
                    id,
                    done_at,
                    binned_at,
                    open_index,
                    ..
                },
            ] => {
                assert_eq!(id, &first);
                assert!(done_at.is_some());
                assert!(binned_at.is_none());
                assert_eq!(*open_index, None);
            }
            other => panic!("expected surgical ItemLifecycleChanged, got {other:?}"),
        }
        assert_open_projection_matches_doc(&b);
        assert_eq!(a.fingerprint(), b.fingerprint());
    }

    #[test]
    fn remote_move_translates_to_item_moved_with_open_index() {
        let dek = Dek::generate();
        let mut a = Doc::new().unwrap();
        let _first = a.add_item(LIST_INBOX, "first").unwrap();
        let _second = a.add_item(LIST_INBOX, "second").unwrap();
        let third = a.add_item(LIST_INBOX, "third").unwrap();
        let mut b = sync_fresh_peer(&mut a, &dek);

        a.move_item(&third, LIST_INBOX, 0).unwrap();
        let blob = a.pending_export(&dek).unwrap().unwrap();
        b.apply_remote(&dek, &blob).unwrap();

        let evs = b.drain_events();
        match evs.as_slice() {
            [AppEvent::ItemMoved { id, open_index }] => {
                assert_eq!(id, &third);
                assert_eq!(*open_index, Some(0));
            }
            other => panic!("expected surgical ItemMoved, got {other:?}"),
        }
        assert_open_projection_matches_doc(&b);
        assert_eq!(b.open_item_ids(LIST_INBOX)[0], third);
        assert_eq!(a.fingerprint(), b.fingerprint());
    }

    /// Replay a frame's emitted events the way the JS store does:
    /// naive per-list arrays, remove-then-insert at `open_index`,
    /// applied in emission order. This is the contract the translator
    /// must satisfy — the Rust-internal projection and Loro fingerprint
    /// can be correct while the *emitted events* fail to converge on
    /// the consumer.
    fn replay_open_events(
        pre: &HashMap<String, Vec<String>>,
        evs: &[AppEvent],
    ) -> HashMap<String, Vec<String>> {
        let mut open = pre.clone();
        let remove_everywhere = |open: &mut HashMap<String, Vec<String>>, id: &str| {
            for arr in open.values_mut() {
                arr.retain(|x| x != id);
            }
        };
        let insert_at =
            |open: &mut HashMap<String, Vec<String>>, list: &str, id: &str, at: usize| {
                let arr = open.entry(list.to_string()).or_default();
                arr.insert(at.min(arr.len()), id.to_string());
            };
        for ev in evs {
            match ev {
                AppEvent::ItemAdded {
                    id,
                    list_id,
                    open_index,
                    ..
                } => {
                    remove_everywhere(&mut open, id);
                    if let Some(li) = open_index {
                        insert_at(&mut open, list_id, id, *li);
                    }
                }
                AppEvent::ItemMoved { id, open_index } => {
                    if let Some(li) = open_index {
                        // The store knows the item's list; the test
                        // replayer finds it by membership.
                        let list = open
                            .iter()
                            .find(|(_, arr)| arr.iter().any(|x| x == id))
                            .map(|(l, _)| l.clone());
                        if let Some(list) = list {
                            remove_everywhere(&mut open, id);
                            insert_at(&mut open, &list, id, *li);
                        }
                    }
                }
                AppEvent::ItemListChanged {
                    id,
                    list_id,
                    open_index,
                } => {
                    // Store semantics: a open item moves lists — remove
                    // from the old array and insert into the new one at
                    // `open_index`, *appending* when absent. A hidden
                    // item only changes its list field.
                    let was_open = open.values().any(|arr| arr.iter().any(|x| x == id));
                    remove_everywhere(&mut open, id);
                    if was_open {
                        let at = open_index.unwrap_or(usize::MAX);
                        insert_at(&mut open, list_id, id, at);
                    }
                }
                AppEvent::ItemLifecycleChanged { id, open_index, .. } => {
                    // A lifecycle event either hides the item (None) or
                    // re-inserts it at its list position. The replayer
                    // needs the list; the store reads it off its item
                    // mirror, the test gets it from the final doc — so
                    // lifecycle re-entries are handled by the caller
                    // passing docs whose lists are stable. For pure
                    // reorder/move tests this arm only hides.
                    if open_index.is_none() {
                        remove_everywhere(&mut open, id);
                    }
                }
                AppEvent::ItemRemoved { id } => remove_everywhere(&mut open, id),
                _ => {}
            }
        }
        open.retain(|_, arr| !arr.is_empty());
        open
    }

    fn open_state(doc: &Doc) -> HashMap<String, Vec<String>> {
        let guard = doc.item_index.lock().unwrap();
        guard.open_by_list.clone()
    }

    #[test]
    fn remote_multi_item_move_down_converges_on_consumer() {
        let dek = Dek::generate();
        let mut a = Doc::new().unwrap();
        let ids: Vec<String> = ["a", "b", "c", "d", "e"]
            .iter()
            .map(|t| a.add_item(LIST_INBOX, t).unwrap())
            .collect();
        let mut b = sync_fresh_peer(&mut a, &dek);

        // Select the top two (a, b) and drag them below d — the exact op
        // sequence `planReorderMoves` emits for a two-item downward move:
        // move b to open index 3, then a to open index 2. Final order:
        // [c, d, a, b, e].
        let pre_open = open_state(&b);
        a.move_item(&ids[1], LIST_INBOX, 3).unwrap();
        a.move_item(&ids[0], LIST_INBOX, 2).unwrap();
        let blob = a.pending_export(&dek).unwrap().unwrap();
        b.apply_remote(&dek, &blob).unwrap();

        let evs = b.drain_events();
        let expected = vec![
            ids[2].clone(),
            ids[3].clone(),
            ids[0].clone(),
            ids[1].clone(),
            ids[4].clone(),
        ];
        assert_eq!(a.open_item_ids(LIST_INBOX), expected);
        assert_eq!(a.fingerprint(), b.fingerprint());
        assert_open_projection_matches_doc(&b);
        assert!(
            !evs.contains(&AppEvent::FullResync),
            "expected surgical events, got a resync: {evs:?}"
        );
        assert_eq!(
            replay_open_events(&pre_open, &evs)
                .get(LIST_INBOX)
                .cloned()
                .unwrap_or_default(),
            expected,
            "emitted events interleave on the consumer; got {evs:?}"
        );
    }

    #[test]
    fn remote_multi_item_move_up_converges_on_consumer() {
        let dek = Dek::generate();
        let mut a = Doc::new().unwrap();
        let ids: Vec<String> = ["a", "b", "c", "d", "e"]
            .iter()
            .map(|t| a.add_item(LIST_INBOX, t).unwrap())
            .collect();
        let mut b = sync_fresh_peer(&mut a, &dek);

        // Select c, d and drag them to the top — `planReorderMoves`
        // emits move c to 0, then d to 1. Final order: [c, d, a, b, e].
        let pre_open = open_state(&b);
        a.move_item(&ids[2], LIST_INBOX, 0).unwrap();
        a.move_item(&ids[3], LIST_INBOX, 1).unwrap();
        let blob = a.pending_export(&dek).unwrap().unwrap();
        b.apply_remote(&dek, &blob).unwrap();

        let evs = b.drain_events();
        let expected = vec![
            ids[2].clone(),
            ids[3].clone(),
            ids[0].clone(),
            ids[1].clone(),
            ids[4].clone(),
        ];
        assert_eq!(a.open_item_ids(LIST_INBOX), expected);
        assert_eq!(a.fingerprint(), b.fingerprint());
        assert_open_projection_matches_doc(&b);
        assert!(
            !evs.contains(&AppEvent::FullResync),
            "expected surgical events, got a resync: {evs:?}"
        );
        assert_eq!(
            replay_open_events(&pre_open, &evs)
                .get(LIST_INBOX)
                .cloned()
                .unwrap_or_default(),
            expected,
            "emitted events interleave on the consumer; got {evs:?}"
        );
    }

    #[test]
    fn remote_discontiguous_multi_move_converges_on_consumer() {
        let dek = Dek::generate();
        let mut a = Doc::new().unwrap();
        let ids: Vec<String> = ["a", "b", "c", "d", "e", "f"]
            .iter()
            .map(|t| a.add_item(LIST_INBOX, t).unwrap())
            .collect();
        let mut b = sync_fresh_peer(&mut a, &dek);

        // Discontiguous selection {a, c} dragged down to sit before f:
        // move a below e, then c below e. Exercises a non-adjacent widen.
        let pre_open = open_state(&b);
        a.move_item(&ids[0], LIST_INBOX, 3).unwrap();
        a.move_item(&ids[2], LIST_INBOX, 4).unwrap();
        let blob = a.pending_export(&dek).unwrap().unwrap();
        b.apply_remote(&dek, &blob).unwrap();

        let evs = b.drain_events();
        assert_eq!(a.fingerprint(), b.fingerprint());
        assert_open_projection_matches_doc(&b);
        assert!(
            !evs.contains(&AppEvent::FullResync),
            "expected surgical events, got a resync: {evs:?}"
        );
        assert_eq!(
            replay_open_events(&pre_open, &evs)
                .get(LIST_INBOX)
                .cloned()
                .unwrap_or_default(),
            a.open_item_ids(LIST_INBOX),
            "emitted events interleave on the consumer; got {evs:?}"
        );
    }

    /// Two items changing lists in one frame is surgical in the v2
    /// schema (cross-list moves are ordinary register writes + entry
    /// ops), where v1 had to fall back to a whole-doc resync.
    #[test]
    fn remote_cross_list_multi_move_is_surgical_and_converges() {
        let dek = Dek::generate();
        let mut a = Doc::new().unwrap();
        let other = a.add_list("Other").unwrap();
        let m0 = a.add_item(LIST_INBOX, "m0").unwrap();
        let m1 = a.add_item(LIST_INBOX, "m1").unwrap();
        let _o0 = a.add_item(&other, "o0").unwrap();
        let _o1 = a.add_item(&other, "o1").unwrap();
        let mut b = sync_fresh_peer(&mut a, &dek);

        let pre_open = open_state(&b);
        a.move_item(&m0, &other, 1).unwrap();
        a.move_item(&m1, &other, 2).unwrap();
        let blob = a.pending_export(&dek).unwrap().unwrap();
        b.apply_remote(&dek, &blob).unwrap();

        let evs = b.drain_events();
        assert!(
            !evs.contains(&AppEvent::FullResync),
            "expected surgical events, got a resync: {evs:?}"
        );
        assert_eq!(a.fingerprint(), b.fingerprint());
        assert_open_projection_matches_doc(&b);
        assert_eq!(a.open_item_ids(&other), b.open_item_ids(&other));
        assert_eq!(a.open_item_ids(LIST_INBOX), b.open_item_ids(LIST_INBOX));
        let replayed = replay_open_events(&pre_open, &evs);
        assert_eq!(
            replayed.get(other.as_str()).cloned().unwrap_or_default(),
            b.open_item_ids(&other)
        );
        assert_eq!(
            replayed.get(LIST_INBOX).cloned().unwrap_or_default(),
            b.open_item_ids(LIST_INBOX)
        );
    }

    #[test]
    fn remote_delete_translates_to_item_removed() {
        let dek = Dek::generate();
        let mut a = Doc::new().unwrap();
        let victim = a.add_item(LIST_INBOX, "victim").unwrap();
        let _keeper = a.add_item(LIST_INBOX, "keeper").unwrap();
        a.set_item_binned(&victim, true).unwrap();
        let mut b = sync_fresh_peer(&mut a, &dek);

        a.delete_binned(&victim).unwrap();
        let blob = a.pending_export(&dek).unwrap().unwrap();
        b.apply_remote(&dek, &blob).unwrap();

        let evs = b.drain_events();
        assert!(
            matches!(evs.as_slice(), [AppEvent::ItemRemoved { id }] if id == &victim),
            "expected surgical ItemRemoved, got {evs:?}"
        );
        assert_open_projection_matches_doc(&b);
        assert_eq!(a.fingerprint(), b.fingerprint());
    }

    #[test]
    fn remote_cross_list_move_translates_to_list_changed() {
        let dek = Dek::generate();
        let mut a = Doc::new().unwrap();
        let other = a.add_list("Other").unwrap();
        let roamer = a.add_item(LIST_INBOX, "roamer").unwrap();
        let _anchor1 = a.add_item(&other, "anchor1").unwrap();
        let _anchor2 = a.add_item(&other, "anchor2").unwrap();
        let mut b = sync_fresh_peer(&mut a, &dek);

        a.move_item(&roamer, &other, 1).unwrap();
        let blob = a.pending_export(&dek).unwrap().unwrap();
        b.apply_remote(&dek, &blob).unwrap();

        let evs = b.drain_events();
        // Leave-signal first (consumer moves the item to the target
        // list, appended), then the positional correction.
        match evs.as_slice() {
            [
                AppEvent::ItemListChanged {
                    id: lid,
                    list_id,
                    open_index: li1,
                },
                AppEvent::ItemMoved {
                    id: mid,
                    open_index: li2,
                },
            ] => {
                assert_eq!(lid, &roamer);
                assert_eq!(mid, &roamer);
                assert_eq!(list_id, &other);
                assert_eq!(*li1, None, "leave-signal carries no position");
                assert_eq!(*li2, Some(1));
            }
            other_evs => panic!("expected ItemListChanged + ItemMoved, got {other_evs:?}"),
        }
        assert_open_projection_matches_doc(&b);
        assert_eq!(b.open_item_ids(&other)[1], roamer);
        assert_eq!(a.fingerprint(), b.fingerprint());
    }

    #[test]
    fn remote_bulk_frame_falls_back_and_converges() {
        let dek = Dek::generate();
        let mut a = Doc::new().unwrap();
        let texts: Vec<String> = (0..100).map(|i| format!("bulk {i}")).collect();
        let refs: Vec<&str> = texts.iter().map(String::as_str).collect();
        let ids = a.add_items_at(LIST_INBOX, &refs, 0).unwrap();
        let mut b = sync_fresh_peer(&mut a, &dek);

        // 100 dirty items in one frame → over DIFF_TRANSLATE_MAX_DIRTY.
        let id_refs: Vec<&str> = ids.iter().map(String::as_str).collect();
        a.set_items_done(&id_refs, true).unwrap();
        let blob = a.pending_export(&dek).unwrap().unwrap();
        b.apply_remote(&dek, &blob).unwrap();

        let evs = b.drain_events();
        assert_eq!(evs, vec![AppEvent::FullResync]);
        assert_open_projection_matches_doc(&b);
        assert_eq!(a.fingerprint(), b.fingerprint());
        assert!(b.open_item_ids(LIST_INBOX).is_empty());
    }

    #[test]
    fn apply_remote_emits_item_added_for_peer_inserts() {
        let dek = Dek::generate();
        let a = Doc::new().unwrap();
        let item_id = a.add_item(LIST_INBOX, "from peer").unwrap();
        let blob = a.pending_export(&dek).unwrap().unwrap();

        let mut b = Doc::empty();
        let _ = b.drain_events();
        b.apply_remote(&dek, &blob).unwrap();
        let evs = b.drain_events();

        // Should include ItemAdded for the peer item. Main has no
        // ListMeta row, so no ListAdded is emitted for it.
        assert!(
            !evs.iter()
                .any(|e| matches!(e, AppEvent::ListAdded { id, .. } if id == LIST_INBOX)),
            "main is virtual; no ListAdded should be emitted: {evs:?}"
        );
        assert!(
            evs.iter()
                .any(|e| matches!(e, AppEvent::ItemAdded { id, .. } if id == &item_id)),
            "expected ItemAdded for peer item: {evs:?}"
        );
    }

    #[test]
    fn apply_remote_emits_text_changed_for_peer_edits() {
        let dek = Dek::generate();
        let mut a = Doc::new().unwrap();
        let id = a.add_item(LIST_INBOX, "old").unwrap();
        let setup_blob = a.pending_export(&dek).unwrap().unwrap();
        a.mark_persisted();

        let mut b = Doc::empty();
        b.apply_remote(&dek, &setup_blob).unwrap();
        let _ = b.drain_events();

        a.edit_item_text(&id, "new").unwrap();
        let edit_blob = a.pending_export(&dek).unwrap().unwrap();
        b.apply_remote(&dek, &edit_blob).unwrap();
        let evs = b.drain_events();
        assert!(
            evs.iter()
                .any(|e| matches!(e, AppEvent::ItemTextChanged { id: eid, text } if eid == &id && text == "new")),
            "expected ItemTextChanged in {evs:?}"
        );
    }

    #[test]
    fn apply_remote_batch_emits_final_state_once_for_multi_blob_catchup() {
        let dek = Dek::generate();
        let mut src = Doc::new().unwrap();
        let id = src.add_item(LIST_INBOX, "old").unwrap();
        let setup_blob = src.pending_export(&dek).unwrap().unwrap();
        src.mark_persisted();

        src.edit_item_text(&id, "new").unwrap();
        let edit_blob = src.pending_export(&dek).unwrap().unwrap();

        let mut dst = Doc::empty();
        let _ = dst.drain_events();
        dst.apply_remote_batch(&dek, [&setup_blob, &edit_blob])
            .unwrap();
        let evs = dst.drain_events();

        assert!(
            evs.iter().any(
                |e| matches!(e, AppEvent::ItemAdded { id: eid, text, .. } if eid == &id && text == "new")
            ),
            "expected final ItemAdded for {id} in {evs:?}"
        );
        assert!(
            !evs.iter()
                .any(|e| matches!(e, AppEvent::ItemTextChanged { id: eid, .. } if eid == &id)),
            "batch catch-up should emit final-state delta, not intermediate edit churn: {evs:?}"
        );
        assert_eq!(dst.fingerprint(), src.fingerprint());
    }

    #[test]
    fn snapshot_then_multiple_trailing_deltas_converge_on_fresh_peer() {
        // Mirrors the e2e bootstrap: a producer captures N ops one at a
        // time (pending_export + mark_persisted, the capture-cursor model),
        // a full snapshot is taken mid-stream, then more ops are
        // captured. A fresh peer applies the snapshot followed by the
        // trailing per-op deltas as a batch and must converge.
        let dek = Dek::generate();
        let mut a = Doc::new().unwrap();
        a.mark_persisted(); // cursor at the seed, like Doc.create() on web

        // Five captured ops, one delta each.
        for i in 0..5 {
            a.add_item(LIST_INBOX, &format!("item {i}")).unwrap();
            let _ = a.pending_export(&dek).unwrap().unwrap();
            a.mark_persisted();
        }
        // Snapshot taken here (frontier = 5 items).
        let snapshot = a.snapshot_blob(&dek).unwrap();

        // Three more captured ops, one delta each.
        let mut trailing = Vec::new();
        for i in 5..8 {
            a.add_item(LIST_INBOX, &format!("item {i}")).unwrap();
            trailing.push(a.pending_export(&dek).unwrap().unwrap());
            a.mark_persisted();
        }

        // Fresh peer: snapshot, then the trailing deltas as a batch.
        let mut b = Doc::empty();
        b.apply_remote(&dek, &snapshot).unwrap();
        b.apply_remote_batch(&dek, trailing.iter()).unwrap();

        assert_eq!(b.fingerprint(), a.fingerprint());
        let texts: Vec<String> = b
            .items_in_list(LIST_INBOX, false)
            .into_iter()
            .map(|it| it.text)
            .collect();
        assert_eq!(
            texts.len(),
            8,
            "expected all 8 items on the peer, got {texts:?}"
        );
    }

    #[test]
    fn add_item_at_inserts_at_target_position() {
        let doc = Doc::new().unwrap();
        let a = doc.add_item(LIST_INBOX, "a").unwrap();
        let b = doc.add_item(LIST_INBOX, "b").unwrap();
        let c = doc.add_item(LIST_INBOX, "c").unwrap();
        let mid = doc.add_item_at(LIST_INBOX, "mid", 1).unwrap();
        assert_eq!(doc.open_item_ids(LIST_INBOX), vec![a, mid, b, c]);
    }

    #[test]
    fn add_item_at_appends_when_target_past_end() {
        let doc = Doc::new().unwrap();
        let a = doc.add_item(LIST_INBOX, "a").unwrap();
        let b = doc.add_item_at(LIST_INBOX, "b", 99).unwrap();
        assert_eq!(doc.open_item_ids(LIST_INBOX), vec![a, b]);
    }

    #[test]
    fn add_item_at_skips_other_lists_and_non_open_when_counting() {
        let doc = Doc::new().unwrap();
        let other = doc.add_list("Other").unwrap();
        let a = doc.add_item(LIST_INBOX, "a").unwrap();
        let _hidden = doc.add_item(&other, "hidden").unwrap();
        let b = doc.add_item(LIST_INBOX, "b").unwrap();
        let done = doc.add_item(LIST_INBOX, "done").unwrap();
        doc.set_item_done(&done, true).unwrap();
        // Position 1 in main's open view should land between a and b
        // regardless of the other-list and done items in between.
        let mid = doc.add_item_at(LIST_INBOX, "mid", 1).unwrap();
        assert_eq!(doc.open_item_ids(LIST_INBOX), vec![a, mid, b]);
    }

    #[test]
    fn add_item_at_emits_item_added_with_open_index() {
        let doc = Doc::new().unwrap();
        let _a = doc.add_item(LIST_INBOX, "a").unwrap();
        let _b = doc.add_item(LIST_INBOX, "b").unwrap();
        let _ = doc.drain_events();
        let mid = doc.add_item_at(LIST_INBOX, "mid", 1).unwrap();
        let evs = doc.drain_events();
        assert_eq!(evs.len(), 1);
        match &evs[0] {
            AppEvent::ItemAdded { id, open_index, .. } => {
                assert_eq!(id, &mid);
                assert_eq!(*open_index, Some(1));
            }
            other => panic!("expected ItemAdded, got {other:?}"),
        }
    }

    #[test]
    fn local_move_item_emits_destination_open_index() {
        let doc = Doc::new().unwrap();
        let first = doc.add_item(LIST_INBOX, "first").unwrap();
        let moved = doc.add_item(LIST_INBOX, "moved").unwrap();
        let third = doc.add_item(LIST_INBOX, "third").unwrap();
        let _ = doc.drain_events();

        doc.move_item(&moved, LIST_INBOX, 0).unwrap();

        assert_eq!(
            doc.open_item_ids(LIST_INBOX),
            vec![moved.clone(), first, third]
        );
        let evs = doc.drain_events();
        assert!(
            evs.iter().any(
                |e| matches!(e, AppEvent::ItemMoved { id, open_index } if id == &moved && *open_index == Some(0))
            ),
            "expected ItemMoved to open index 0, got {evs:?}"
        );
    }

    #[test]
    fn set_items_binned_small_batch_is_surgical_and_restores_position() {
        let doc = Doc::new().unwrap();
        let first = doc.add_item(LIST_INBOX, "first").unwrap();
        let second = doc.add_item(LIST_INBOX, "second").unwrap();
        let third = doc.add_item(LIST_INBOX, "third").unwrap();
        let _ = doc.drain_events();

        // Below the bulk threshold: exactly one per-item event, no
        // whole-doc diff artifacts (which would add ItemMoved noise).
        doc.set_items_binned(&[second.as_str()], true).unwrap();
        let evs = doc.drain_events();
        match evs.as_slice() {
            [
                AppEvent::ItemLifecycleChanged {
                    id,
                    binned_at,
                    open_index,
                    ..
                },
            ] => {
                assert_eq!(id, &second);
                assert!(binned_at.is_some());
                assert_eq!(*open_index, None);
            }
            other => panic!("expected one surgical ItemLifecycleChanged, got {other:?}"),
        }
        assert_open_projection_matches_doc(&doc);
        assert_eq!(
            doc.open_item_ids(LIST_INBOX),
            vec![first.clone(), third.clone()]
        );

        // Restore: re-enters the open projection at its former position
        // (between first and third — the entry never moved), and the
        // event says so.
        doc.set_items_binned(&[second.as_str()], false).unwrap();
        let evs = doc.drain_events();
        match evs.as_slice() {
            [
                AppEvent::ItemLifecycleChanged {
                    id,
                    binned_at,
                    open_index,
                    ..
                },
            ] => {
                assert_eq!(id, &second);
                assert!(binned_at.is_none());
                assert_eq!(*open_index, Some(1));
            }
            other => panic!("expected one surgical ItemLifecycleChanged, got {other:?}"),
        }
        assert_open_projection_matches_doc(&doc);
        assert_eq!(doc.open_item_ids(LIST_INBOX), vec![first, second, third]);
    }

    #[test]
    fn set_items_binned_updates_many_in_one_call() {
        let doc = Doc::new().unwrap();
        let first = doc.add_item(LIST_INBOX, "first").unwrap();
        let second = doc.add_item(LIST_INBOX, "second").unwrap();

        doc.set_items_binned(&[first.as_str(), second.as_str()], true)
            .unwrap();

        assert_eq!(doc.open_item_ids(LIST_INBOX), Vec::<String>::new());
        assert_eq!(doc.binned_item_ids().len(), 2);
    }

    #[test]
    fn delete_binned_items_removes_many_in_one_call() {
        let doc = Doc::new().unwrap();
        let keep = doc.add_item(LIST_INBOX, "keep").unwrap();
        let first = doc.add_item(LIST_INBOX, "first").unwrap();
        let second = doc.add_item(LIST_INBOX, "second").unwrap();
        doc.set_items_binned(&[first.as_str(), second.as_str()], true)
            .unwrap();

        doc.delete_binned_items(&[first.as_str(), second.as_str()])
            .unwrap();

        assert_eq!(doc.open_item_ids(LIST_INBOX), vec![keep]);
        assert_eq!(doc.binned_item_ids(), Vec::<String>::new());
    }

    #[test]
    fn add_item_at_rejects_empty_text() {
        let doc = Doc::new().unwrap();
        let err = doc.add_item_at(LIST_INBOX, "  ", 0).unwrap_err();
        assert!(matches!(err, DocError::Invalid(_)));
    }

    #[test]
    fn add_items_at_inserts_in_order() {
        let doc = Doc::new().unwrap();
        let a = doc.add_item(LIST_INBOX, "a").unwrap();
        let b = doc.add_item(LIST_INBOX, "b").unwrap();
        let ids = doc.add_items_at(LIST_INBOX, &["x", "y", "z"], 1).unwrap();
        assert_eq!(ids.len(), 3);
        assert_eq!(
            doc.open_item_ids(LIST_INBOX),
            vec![a, ids[0].clone(), ids[1].clone(), ids[2].clone(), b],
        );
    }

    #[test]
    fn add_items_at_appends_when_target_past_end() {
        let doc = Doc::new().unwrap();
        let a = doc.add_item(LIST_INBOX, "a").unwrap();
        let ids = doc.add_items_at(LIST_INBOX, &["x", "y"], 99).unwrap();
        assert_eq!(
            doc.open_item_ids(LIST_INBOX),
            vec![a, ids[0].clone(), ids[1].clone()]
        );
    }

    #[test]
    fn add_items_at_emits_one_event_per_item_with_increasing_open_indices() {
        let doc = Doc::new().unwrap();
        let _a = doc.add_item(LIST_INBOX, "a").unwrap();
        let _b = doc.add_item(LIST_INBOX, "b").unwrap();
        let _ = doc.drain_events();
        let ids = doc.add_items_at(LIST_INBOX, &["x", "y"], 1).unwrap();
        let evs = doc.drain_events();
        let added: Vec<(String, Option<usize>)> = evs
            .iter()
            .filter_map(|e| match e {
                AppEvent::ItemAdded { id, open_index, .. } => Some((id.clone(), *open_index)),
                _ => None,
            })
            .collect();
        assert_eq!(added.len(), 2);
        assert_eq!(added[0], (ids[0].clone(), Some(1)));
        assert_eq!(added[1], (ids[1].clone(), Some(2)));
    }

    #[test]
    fn add_items_at_rejects_batch_atomically_on_empty_text() {
        let doc = Doc::new().unwrap();
        let _a = doc.add_item(LIST_INBOX, "a").unwrap();
        let _ = doc.drain_events();
        let err = doc
            .add_items_at(LIST_INBOX, &["ok", "  ", "also ok"], 0)
            .unwrap_err();
        assert!(matches!(err, DocError::Invalid(_)));
        // Nothing landed.
        assert_eq!(doc.open_item_ids(LIST_INBOX).len(), 1);
        assert!(doc.drain_events().is_empty());
    }

    #[test]
    fn add_items_at_empty_input_is_a_noop() {
        let doc = Doc::new().unwrap();
        let _ = doc.drain_events();
        let ids = doc.add_items_at(LIST_INBOX, &[], 0).unwrap();
        assert!(ids.is_empty());
        assert!(doc.drain_events().is_empty());
    }

    #[test]
    fn snapshot_events_materializes_current_state() {
        let doc = Doc::new().unwrap();
        let other = doc.add_list("Other").unwrap();
        let a = doc.add_item(LIST_INBOX, "a").unwrap();
        let b = doc.add_item(&other, "b").unwrap();
        doc.set_show_list_counts(true).unwrap();
        let _ = doc.drain_events();

        let snap = doc.snapshot_events();
        assert!(snap.iter().any(|e| matches!(
            e,
            AppEvent::SettingsChanged {
                show_list_counts: true,
                ..
            }
        )));
        // ListAdded events come first, then ItemAdded events.
        let lists: Vec<&str> = snap
            .iter()
            .filter_map(|e| match e {
                AppEvent::ListAdded { id, .. } => Some(id.as_str()),
                _ => None,
            })
            .collect();
        let items: Vec<&str> = snap
            .iter()
            .filter_map(|e| match e {
                AppEvent::ItemAdded { id, .. } => Some(id.as_str()),
                _ => None,
            })
            .collect();
        // Main is virtual; no ListAdded is emitted for it.
        assert!(!lists.contains(&LIST_INBOX));
        assert!(lists.contains(&other.as_str()));
        assert!(items.contains(&a.as_str()));
        assert!(items.contains(&b.as_str()));
    }

    #[test]
    fn export_json_string_is_valid_pretty_json() {
        let doc = Doc::new().unwrap();
        let other = doc.add_list("Other").unwrap();
        let _ = doc.add_item(LIST_INBOX, "a").unwrap();
        let _ = doc.add_item(&other, "b").unwrap();

        let s = doc.export_json_string();
        // Pretty form has at least one newline; validates that the
        // method went through `to_string_pretty`, not a compact dump.
        assert!(s.contains('\n'));
        // Round-trips through serde_json into the same struct.
        let parsed: JsonExport = serde_json::from_str(&s).unwrap();
        assert_eq!(parsed, doc.export_json());
    }

    #[test]
    fn import_json_creates_lists_with_fresh_ids_and_routes_main_locally() {
        let src = Doc::new().unwrap();
        let other = src.add_list("Other").unwrap();
        let _ = src.add_item(LIST_INBOX, "in-main").unwrap();
        let _ = src.add_item(&other, "in-other").unwrap();
        let export = src.export_json();

        let dst = Doc::new().unwrap();
        let summary = dst.import_json(&export).unwrap();

        assert_eq!(summary.lists_added, 1);
        assert_eq!(summary.items_added, 2);
        assert_eq!(summary.items_skipped, 0);

        let dst_lists = dst.all_lists();
        assert_eq!(dst_lists.len(), 1);
        assert_eq!(dst_lists[0].name, "Other");
        // New list got a fresh id — additive means we do *not* reuse the
        // source's list id.
        assert_ne!(dst_lists[0].id, other);

        let texts_main: Vec<String> = dst
            .iter_items()
            .filter(|i| i.list_id == LIST_INBOX)
            .map(|i| i.text)
            .collect();
        assert_eq!(texts_main, vec!["in-main"]);

        let texts_other: Vec<String> = dst
            .iter_items()
            .filter(|i| i.list_id == dst_lists[0].id)
            .map(|i| i.text)
            .collect();
        assert_eq!(texts_other, vec!["in-other"]);
    }

    #[test]
    fn import_json_round_trips_list_icon() {
        let src = Doc::new().unwrap();
        let with_icon = src.add_list("Errands").unwrap();
        src.set_list_icon(&with_icon, "🛒").unwrap();
        let _plain = src.add_list("Notes").unwrap();
        let export = src.export_json();

        // The icon rides on the source list's export entry (and Inbox has
        // none), while an iconless list serializes with the key absent.
        let exported = export
            .lists
            .iter()
            .find(|l| l.id == with_icon)
            .expect("source list in export");
        assert_eq!(exported.icon.as_deref(), Some("🛒"));

        let dst = Doc::new().unwrap();
        dst.import_json(&export).unwrap();

        let icons: Vec<(String, Option<String>)> = dst
            .all_lists()
            .into_iter()
            .map(|l| (l.name, l.icon))
            .collect();
        assert!(icons.contains(&("Errands".to_string(), Some("🛒".to_string()))));
        assert!(icons.contains(&("Notes".to_string(), None)));
    }

    #[test]
    fn export_json_omits_focus_when_empty_and_carries_it_in_order() {
        let doc = Doc::new().unwrap();
        let a = doc.add_item(LIST_INBOX, "a").unwrap();
        let b = doc.add_item(LIST_INBOX, "b").unwrap();

        // No focus refs → the key is absent (pre-focus dump byte-compat).
        assert!(!doc.export_json_string().contains("\"focus\""));

        doc.add_to_focus(&b, usize::MAX).unwrap();
        doc.add_to_focus(&a, usize::MAX).unwrap();
        let export = doc.export_json();
        assert_eq!(export.focus, vec![b.clone(), a.clone()]);
    }

    #[test]
    fn import_json_round_trips_focus_order_onto_fresh_ids() {
        let src = Doc::new().unwrap();
        let a = src.add_item(LIST_INBOX, "alpha").unwrap();
        let b = src.add_item(LIST_INBOX, "beta").unwrap();
        let _ = src.add_item(LIST_INBOX, "gamma").unwrap();
        src.add_to_focus(&b, usize::MAX).unwrap();
        src.add_to_focus(&a, usize::MAX).unwrap();
        let export = src.export_json();

        // The destination already curates its own Focus — imported refs
        // append after it rather than disturbing local order.
        let dst = Doc::new().unwrap();
        let local = dst.add_item(LIST_INBOX, "local").unwrap();
        dst.add_to_focus(&local, usize::MAX).unwrap();

        let summary = dst.import_json(&export).unwrap();
        assert_eq!(summary.focus_added, 2);

        let focus_texts: Vec<String> = dst.focus_view().into_iter().map(|i| i.text).collect();
        assert_eq!(focus_texts, vec!["local", "beta", "alpha"]);
        // Refs were remapped — none of the source ids appear verbatim.
        for item in dst.focus_view() {
            assert_ne!(item.id, a);
            assert_ne!(item.id, b);
        }
    }

    #[test]
    fn import_json_drops_unresolvable_focus_refs() {
        let src = Doc::new().unwrap();
        let a = src.add_item(LIST_INBOX, "kept").unwrap();
        let mut export = src.export_json();
        // Hand-edited garbage: unknown id, foreign-doc ref, a ref to a
        // skipped (empty-text) item, and a duplicate of a valid ref.
        export.items.push(ExportItem {
            id: "skipped".to_string(),
            text: "   ".to_string(),
            notes: String::new(),
            list_id: LIST_INBOX.to_string(),
            lifecycle: ExportLifecycle {
                state: "backlog".to_string(),
                at: 1,
            },
            deadline: None,
            when: None,
            duration: None,
            created_at: 1,
            started_at: None,
            done_at: None,
            binned_at: None,
        });
        export.focus = vec![
            a.clone(),
            "missing".to_string(),
            format!("deadbeef:{a}"),
            "skipped".to_string(),
            a.clone(),
        ];

        let dst = Doc::new().unwrap();
        let summary = dst.import_json(&export).unwrap();
        assert_eq!(summary.focus_added, 1);
        let focus_texts: Vec<String> = dst.focus_view().into_iter().map(|i| i.text).collect();
        assert_eq!(focus_texts, vec!["kept"]);
    }

    #[test]
    fn import_json_drops_focus_refs_to_non_open_items() {
        let src = Doc::new().unwrap();
        let open = src.add_item(LIST_INBOX, "open").unwrap();
        let done = src.add_item(LIST_INBOX, "done-later").unwrap();
        src.add_to_focus(&done, usize::MAX).unwrap();
        src.add_to_focus(&open, usize::MAX).unwrap();
        let mut export = src.export_json();
        // Simulate a hand-edited dump where a focus ref points at a Done
        // item (a live doc self-compacts these, so force it in the JSON).
        for item in &mut export.items {
            if item.text == "done-later" {
                item.lifecycle = ExportLifecycle {
                    state: "done".to_string(),
                    at: 42,
                };
            }
        }

        let dst = Doc::new().unwrap();
        let summary = dst.import_json(&export).unwrap();
        assert_eq!(summary.focus_added, 1);
        let focus_texts: Vec<String> = dst.focus_view().into_iter().map(|i| i.text).collect();
        assert_eq!(focus_texts, vec!["open"]);
    }

    #[test]
    fn import_json_preserves_timestamps_done_binned_and_notes() {
        let src = Doc::new().unwrap();
        let a = src.add_item(LIST_INBOX, "alpha").unwrap();
        let b = src.add_item(LIST_INBOX, "beta").unwrap();
        let c = src.add_item(LIST_INBOX, "gamma").unwrap();
        src.edit_item_notes(&a, "alpha notes").unwrap();
        src.set_item_done(&b, true).unwrap();
        src.set_item_binned(&c, true).unwrap();

        let src_view_a = src.get_item(&a).unwrap();
        let src_view_b = src.get_item(&b).unwrap();
        let src_view_c = src.get_item(&c).unwrap();
        let export = src.export_json();

        let dst = Doc::new().unwrap();
        dst.import_json(&export).unwrap();

        let imported: Vec<ItemView> = dst.iter_items().collect();
        let by_text = |t: &str| imported.iter().find(|i| i.text == t).unwrap();
        let ia = by_text("alpha");
        let ib = by_text("beta");
        let ic = by_text("gamma");

        assert_eq!(ia.notes, "alpha notes");
        assert_eq!(ia.created_at, src_view_a.created_at);

        assert_eq!(ib.done_at, src_view_b.done_at);
        assert!(ib.done_at.is_some());

        assert_eq!(ic.binned_at, src_view_c.binned_at);
        assert!(ic.binned_at.is_some());
    }

    #[test]
    fn set_and_clear_item_deadline() {
        let doc = Doc::new().unwrap();
        let id = doc.add_item(LIST_INBOX, "pay rent").unwrap();
        let _ = doc.drain_events();

        doc.set_item_deadline(&id, Some("2026-07-15")).unwrap();
        assert_eq!(
            doc.get_item(&id).unwrap().deadline.as_deref(),
            Some("2026-07-15")
        );
        assert_eq!(
            doc.drain_events(),
            vec![AppEvent::ItemDeadlineChanged {
                id: id.clone(),
                deadline: Some("2026-07-15".into()),
            }]
        );

        // Whitespace is trimmed before storage.
        doc.set_item_deadline(&id, Some("  2026-08-01  ")).unwrap();
        assert_eq!(
            doc.get_item(&id).unwrap().deadline.as_deref(),
            Some("2026-08-01")
        );
        let _ = doc.drain_events();

        // Clearing deletes the key.
        doc.set_item_deadline(&id, None).unwrap();
        assert_eq!(doc.get_item(&id).unwrap().deadline, None);
        assert_eq!(
            doc.drain_events(),
            vec![AppEvent::ItemDeadlineChanged {
                id: id.clone(),
                deadline: None,
            }]
        );
    }

    #[test]
    fn set_item_deadline_rejects_malformed_dates() {
        let doc = Doc::new().unwrap();
        let id = doc.add_item(LIST_INBOX, "task").unwrap();
        let _ = doc.drain_events();

        for bad in [
            "2026-7-15",        // single-digit month
            "2026-13-01",       // month out of range
            "2026-02-30",       // day out of range for February
            "2026-00-10",       // zero month
            "2026-01-00",       // zero day
            "07/15/2026",       // wrong separators
            "2026-07-15T00:00", // has a time component
            "1752566400000",    // unix millis
            "not-a-date",
            "",
        ] {
            let err = doc.set_item_deadline(&id, Some(bad)).unwrap_err();
            assert!(
                matches!(err, DocError::Invalid(_)),
                "expected Invalid for {bad:?}, got {err:?}"
            );
        }
        // Nothing was written and no event fired.
        assert_eq!(doc.get_item(&id).unwrap().deadline, None);
        assert!(doc.drain_events().is_empty());

        // A leap day in a leap year is accepted.
        doc.set_item_deadline(&id, Some("2028-02-29")).unwrap();
        assert_eq!(
            doc.get_item(&id).unwrap().deadline.as_deref(),
            Some("2028-02-29")
        );
        // The same day in a non-leap year is rejected.
        assert!(doc.set_item_deadline(&id, Some("2027-02-29")).is_err());
    }

    #[test]
    fn export_import_preserves_deadline() {
        let src = Doc::new().unwrap();
        let a = src.add_item(LIST_INBOX, "with deadline").unwrap();
        let _b = src.add_item(LIST_INBOX, "no deadline").unwrap();
        src.set_item_deadline(&a, Some("2026-09-30")).unwrap();

        let export = src.export_json();
        // The raw dump carries the string.
        assert!(
            export
                .items
                .iter()
                .any(|i| i.deadline.as_deref() == Some("2026-09-30"))
        );

        let dst = Doc::new().unwrap();
        dst.import_json(&export).unwrap();
        let imported: Vec<ItemView> = dst.iter_items().collect();
        let with_deadline = imported.iter().find(|i| i.text == "with deadline").unwrap();
        let no_deadline = imported.iter().find(|i| i.text == "no deadline").unwrap();
        assert_eq!(with_deadline.deadline.as_deref(), Some("2026-09-30"));
        assert_eq!(no_deadline.deadline, None);
    }

    #[test]
    fn deadline_converges_between_peers() {
        let dek = Dek::generate();
        let mut a = Doc::new().unwrap();
        let id = a.add_item(LIST_INBOX, "sync me").unwrap();
        let seed = a.pending_export(&dek).unwrap().unwrap();
        a.mark_persisted();
        let mut b = Doc::empty();
        b.apply_remote(&dek, &seed).unwrap();
        let _ = a.drain_events();
        let _ = b.drain_events();

        // Set a deadline on a, ship the frame to b.
        a.set_item_deadline(&id, Some("2026-11-05")).unwrap();
        let frame = a.pending_export(&dek).unwrap().unwrap();
        b.apply_remote(&dek, &frame).unwrap();

        assert_eq!(
            b.get_item(&id).unwrap().deadline.as_deref(),
            Some("2026-11-05")
        );
        assert_eq!(a.fingerprint(), b.fingerprint());
        // b's consumer learns via a surgical ItemDeadlineChanged event.
        assert!(b.drain_events().iter().any(|e| matches!(
            e,
            AppEvent::ItemDeadlineChanged { id: eid, deadline: Some(d) } if eid == &id && d == "2026-11-05"
        )));

        // Clearing also converges.
        a.set_item_deadline(&id, None).unwrap();
        let frame = a.pending_export(&dek).unwrap().unwrap();
        b.apply_remote(&dek, &frame).unwrap();
        assert_eq!(b.get_item(&id).unwrap().deadline, None);
        assert_eq!(a.fingerprint(), b.fingerprint());
    }

    #[test]
    fn set_and_clear_item_when() {
        let doc = Doc::new().unwrap();
        let id = doc.add_item(LIST_INBOX, "plan me").unwrap();
        assert_eq!(doc.get_item(&id).unwrap().when, None);
        let _ = doc.drain_events();

        // All-day.
        doc.set_item_when(&id, Some("2026-07-13")).unwrap();
        assert_eq!(
            doc.get_item(&id).unwrap().when.as_deref(),
            Some("2026-07-13")
        );
        assert_eq!(
            doc.drain_events(),
            vec![AppEvent::ItemWhenChanged {
                id: id.clone(),
                when: Some("2026-07-13".into()),
            }]
        );

        // Timed, whitespace trimmed, minutes and hour bounds inclusive.
        doc.set_item_when(&id, Some("  2026-07-13T23:59 ")).unwrap();
        assert_eq!(
            doc.get_item(&id).unwrap().when.as_deref(),
            Some("2026-07-13T23:59")
        );
        doc.set_item_when(&id, Some("2026-07-13T00:00")).unwrap();
        assert_eq!(
            doc.get_item(&id).unwrap().when.as_deref(),
            Some("2026-07-13T00:00")
        );
        let _ = doc.drain_events();

        // Clearing deletes the key; deadline is independent and untouched.
        doc.set_item_deadline(&id, Some("2026-10-31")).unwrap();
        let _ = doc.drain_events();
        doc.set_item_when(&id, None).unwrap();
        let view = doc.get_item(&id).unwrap();
        assert_eq!(view.when, None);
        assert_eq!(view.deadline.as_deref(), Some("2026-10-31"));
        assert_eq!(
            doc.drain_events(),
            vec![AppEvent::ItemWhenChanged {
                id: id.clone(),
                when: None,
            }]
        );
    }

    #[test]
    fn set_and_clear_item_duration() {
        let doc = Doc::new().unwrap();
        let id = doc.add_item(LIST_INBOX, "meeting").unwrap();
        assert_eq!(doc.get_item(&id).unwrap().duration, None);
        let _ = doc.drain_events();

        doc.set_item_when(&id, Some("2026-07-13T14:00")).unwrap();
        let _ = doc.drain_events();
        doc.set_item_duration(&id, Some(90)).unwrap();
        assert_eq!(doc.get_item(&id).unwrap().duration, Some(90));
        assert_eq!(
            doc.drain_events(),
            vec![AppEvent::ItemDurationChanged {
                id: id.clone(),
                duration: Some(90),
            }]
        );

        // Bounds are inclusive; zero and over-a-week are rejected and
        // leave the doc untouched.
        doc.set_item_duration(&id, Some(1)).unwrap();
        doc.set_item_duration(&id, Some(MAX_DURATION_MINUTES))
            .unwrap();
        let _ = doc.drain_events();
        for bad in [0, MAX_DURATION_MINUTES + 1] {
            let err = doc.set_item_duration(&id, Some(bad)).unwrap_err();
            assert!(matches!(err, DocError::Invalid(_)), "{bad}: {err:?}");
        }
        assert_eq!(
            doc.get_item(&id).unwrap().duration,
            Some(MAX_DURATION_MINUTES)
        );
        assert!(doc.drain_events().is_empty());

        // Explicit clear deletes the key; `when` is untouched.
        doc.set_item_duration(&id, None).unwrap();
        let view = doc.get_item(&id).unwrap();
        assert_eq!(view.duration, None);
        assert_eq!(view.when.as_deref(), Some("2026-07-13T14:00"));
        assert_eq!(
            doc.drain_events(),
            vec![AppEvent::ItemDurationChanged {
                id: id.clone(),
                duration: None,
            }]
        );
    }

    #[test]
    fn negative_or_oversized_raw_duration_reads_as_unset() {
        // `set_item_duration` takes a `u32`, so a negative value can only
        // arrive from another writer. The reader treats it as absent.
        let doc = Doc::new().unwrap();
        let id = doc.add_item(LIST_INBOX, "meeting").unwrap();
        let map = doc.find_item(&id).unwrap();
        for raw in [-5i64, 0, i64::from(MAX_DURATION_MINUTES) + 1, i64::MAX] {
            map.insert(KEY_DURATION, raw).unwrap();
            doc.inner.commit();
            assert_eq!(doc.get_item(&id).unwrap().duration, None, "{raw}");
        }
        map.insert(KEY_DURATION, 15i64).unwrap();
        doc.inner.commit();
        assert_eq!(doc.get_item(&id).unwrap().duration, Some(15));
    }

    #[test]
    fn clearing_when_clears_duration_but_all_day_keeps_it() {
        let doc = Doc::new().unwrap();
        let id = doc.add_item(LIST_INBOX, "meeting").unwrap();
        doc.set_item_when(&id, Some("2026-07-13T14:00")).unwrap();
        doc.set_item_duration(&id, Some(30)).unwrap();
        let _ = doc.drain_events();

        // Timed → all-day keeps the length (re-adding a time restores it).
        doc.set_item_when(&id, Some("2026-07-13")).unwrap();
        assert_eq!(doc.get_item(&id).unwrap().duration, Some(30));
        assert_eq!(
            doc.drain_events(),
            vec![AppEvent::ItemWhenChanged {
                id: id.clone(),
                when: Some("2026-07-13".into()),
            }]
        );

        // Clearing `when` drops the duration in the same commit and says so.
        doc.set_item_when(&id, None).unwrap();
        let view = doc.get_item(&id).unwrap();
        assert_eq!(view.when, None);
        assert_eq!(view.duration, None);
        assert_eq!(
            doc.drain_events(),
            vec![
                AppEvent::ItemWhenChanged {
                    id: id.clone(),
                    when: None,
                },
                AppEvent::ItemDurationChanged {
                    id: id.clone(),
                    duration: None,
                },
            ]
        );

        // Clearing an already-clear `when` emits no duration event.
        doc.set_item_when(&id, None).unwrap();
        assert_eq!(
            doc.drain_events(),
            vec![AppEvent::ItemWhenChanged {
                id: id.clone(),
                when: None,
            }]
        );
    }

    #[test]
    fn export_import_preserves_duration_and_drops_out_of_range() {
        let src = Doc::new().unwrap();
        let a = src.add_item(LIST_INBOX, "timed").unwrap();
        let _b = src.add_item(LIST_INBOX, "unset").unwrap();
        src.set_item_when(&a, Some("2026-09-12T14:00")).unwrap();
        src.set_item_duration(&a, Some(45)).unwrap();

        let export = src.export_json();
        let json = serde_json::to_string(&export).unwrap();
        assert_eq!(json.matches("\"duration\"").count(), 1);

        let dst = Doc::new().unwrap();
        dst.import_json(&export).unwrap();
        let imported: Vec<ItemView> = dst.iter_items().collect();
        let find = |t: &str| imported.iter().find(|i| i.text == t).unwrap();
        assert_eq!(find("timed").duration, Some(45));
        assert_eq!(find("unset").duration, None);

        let mut edited = export.clone();
        edited.items[0].duration = Some(0);
        let dst2 = Doc::new().unwrap();
        dst2.import_json(&edited).unwrap();
        assert!(dst2.iter_items().all(|i| i.duration.is_none()));
    }

    #[test]
    fn duration_converges_between_peers() {
        let dek = Dek::generate();
        let mut a = Doc::new().unwrap();
        let id = a.add_item(LIST_INBOX, "sync me").unwrap();
        let seed = a.pending_export(&dek).unwrap().unwrap();
        a.mark_persisted();
        let mut b = Doc::empty();
        b.apply_remote(&dek, &seed).unwrap();
        let _ = a.drain_events();
        let _ = b.drain_events();

        a.set_item_when(&id, Some("2026-11-05T09:30")).unwrap();
        a.set_item_duration(&id, Some(120)).unwrap();
        let frame = a.pending_export(&dek).unwrap().unwrap();
        b.apply_remote(&dek, &frame).unwrap();
        assert_eq!(b.get_item(&id).unwrap().duration, Some(120));
        assert_eq!(a.fingerprint(), b.fingerprint());
        assert!(b.drain_events().iter().any(|e| matches!(
            e,
            AppEvent::ItemDurationChanged { id: eid, duration: Some(120) } if eid == &id
        )));

        // A remote clear of `when` carries the duration clear with it.
        a.set_item_when(&id, None).unwrap();
        let frame = a.pending_export(&dek).unwrap().unwrap();
        b.apply_remote(&dek, &frame).unwrap();
        assert_eq!(b.get_item(&id).unwrap().duration, None);
        assert_eq!(a.fingerprint(), b.fingerprint());
        assert!(b.drain_events().iter().any(|e| matches!(
            e,
            AppEvent::ItemDurationChanged { id: eid, duration: None } if eid == &id
        )));
    }

    #[test]
    fn set_item_when_rejects_malformed_values() {
        let doc = Doc::new().unwrap();
        let id = doc.add_item(LIST_INBOX, "task").unwrap();
        let _ = doc.drain_events();

        for bad in [
            "2026-07-13T14:00:00",                   // seconds
            "2026-07-13T14:00Z",                     // UTC marker
            "2026-07-13T14:00+10:00",                // offset
            "2026-07-13T14:00[Australia/Melbourne]", // reserved zone suffix
            "2026-07-13T24:00",                      // hour out of range
            "2026-07-13T14:60",                      // minute out of range
            "2026-07-13T9:00",                       // single-digit hour
            "2026-07-13t14:00",                      // lowercase separator
            "2026-07-13 14:00",                      // space separator
            "2026-07-13T14.00",                      // wrong time separator
            "2026-07-13T",                           // dangling separator
            "2027-02-29T10:00",                      // invalid date, valid time
            "2026-13-01",                            // invalid all-day
            "1752566400000",                         // unix millis
            "",
        ] {
            let err = doc.set_item_when(&id, Some(bad)).unwrap_err();
            assert!(
                matches!(err, DocError::Invalid(_)),
                "expected Invalid for {bad:?}, got {err:?}"
            );
        }
        assert_eq!(doc.get_item(&id).unwrap().when, None);
        assert!(doc.drain_events().is_empty());

        // Leap day accepted with a time, too.
        doc.set_item_when(&id, Some("2028-02-29T08:30")).unwrap();
        assert_eq!(
            doc.get_item(&id).unwrap().when.as_deref(),
            Some("2028-02-29T08:30")
        );
    }

    #[test]
    fn export_import_preserves_when_and_skips_it_when_unset() {
        let src = Doc::new().unwrap();
        let a = src.add_item(LIST_INBOX, "timed").unwrap();
        let b = src.add_item(LIST_INBOX, "all day").unwrap();
        let _c = src.add_item(LIST_INBOX, "unset").unwrap();
        src.set_item_when(&a, Some("2026-09-12T14:00")).unwrap();
        src.set_item_when(&b, Some("2026-09-12")).unwrap();

        let export = src.export_json();
        let json = serde_json::to_string(&export).unwrap();
        // Exactly the two set values appear; the unset item carries no key.
        assert_eq!(json.matches("\"when\"").count(), 2);

        let dst = Doc::new().unwrap();
        dst.import_json(&export).unwrap();
        let imported: Vec<ItemView> = dst.iter_items().collect();
        let find = |t: &str| imported.iter().find(|i| i.text == t).unwrap();
        assert_eq!(find("timed").when.as_deref(), Some("2026-09-12T14:00"));
        assert_eq!(find("all day").when.as_deref(), Some("2026-09-12"));
        assert_eq!(find("unset").when, None);

        // A malformed value in a hand-edited dump is dropped, not fatal.
        let mut edited = export.clone();
        edited.items[0].when = Some("2026-09-12T14:00:00".into());
        let dst2 = Doc::new().unwrap();
        dst2.import_json(&edited).unwrap();
        assert!(
            dst2.iter_items()
                .all(|i| i.when.is_none() || i.text == "all day")
        );
    }

    #[test]
    fn when_converges_between_peers() {
        let dek = Dek::generate();
        let mut a = Doc::new().unwrap();
        let id = a.add_item(LIST_INBOX, "sync me").unwrap();
        let seed = a.pending_export(&dek).unwrap().unwrap();
        a.mark_persisted();
        let mut b = Doc::empty();
        b.apply_remote(&dek, &seed).unwrap();
        let _ = a.drain_events();
        let _ = b.drain_events();

        a.set_item_when(&id, Some("2026-11-05T09:30")).unwrap();
        let frame = a.pending_export(&dek).unwrap().unwrap();
        b.apply_remote(&dek, &frame).unwrap();

        assert_eq!(
            b.get_item(&id).unwrap().when.as_deref(),
            Some("2026-11-05T09:30")
        );
        assert_eq!(a.fingerprint(), b.fingerprint());
        assert!(b.drain_events().iter().any(|e| matches!(
            e,
            AppEvent::ItemWhenChanged { id: eid, when: Some(w) } if eid == &id && w == "2026-11-05T09:30"
        )));

        a.set_item_when(&id, None).unwrap();
        let frame = a.pending_export(&dek).unwrap().unwrap();
        b.apply_remote(&dek, &frame).unwrap();
        assert_eq!(b.get_item(&id).unwrap().when, None);
        assert_eq!(a.fingerprint(), b.fingerprint());
        assert!(b.drain_events().iter().any(|e| matches!(
            e,
            AppEvent::ItemWhenChanged { id: eid, when: None } if eid == &id
        )));
    }

    #[test]
    fn import_json_preserves_per_list_order() {
        let src = Doc::new().unwrap();
        let l = src.add_list("Ordered").unwrap();
        let x = src.add_item(&l, "x").unwrap();
        let _y = src.add_item(&l, "y").unwrap();
        let _z = src.add_item(&l, "z").unwrap();
        // Reorder so array order != creation order.
        src.move_item(&x, &l, 2).unwrap();
        let src_texts: Vec<String> = src
            .items_in_list(&l, true)
            .into_iter()
            .map(|i| i.text)
            .collect();
        assert_eq!(src_texts, vec!["y", "z", "x"]);

        let dst = Doc::new().unwrap();
        dst.import_json(&src.export_json()).unwrap();
        let dst_list = dst.all_lists()[0].id.clone();
        let dst_texts: Vec<String> = dst
            .items_in_list(&dst_list, true)
            .into_iter()
            .map(|i| i.text)
            .collect();
        assert_eq!(dst_texts, src_texts, "array order carries the ordering");
    }

    #[test]
    fn import_json_is_additive_existing_content_untouched() {
        let dst = Doc::new().unwrap();
        let local_list = dst.add_list("LocalKeep").unwrap();
        let local_item = dst.add_item(LIST_INBOX, "local-main").unwrap();
        let _ = dst.add_item(&local_list, "local-other").unwrap();

        let src = Doc::new().unwrap();
        let _ = src.add_list("Imported").unwrap();
        let _ = src.add_item(LIST_INBOX, "src-main").unwrap();
        let export = src.export_json();

        dst.import_json(&export).unwrap();

        // Pre-existing list and item still present, untouched.
        assert!(dst.get_item(&local_item).is_some());
        let names: Vec<String> = dst.all_lists().into_iter().map(|l| l.name).collect();
        assert!(names.contains(&"LocalKeep".to_string()));
        assert!(names.contains(&"Imported".to_string()));
        assert_eq!(names.len(), 2);

        // Both `src-main` and `local-main` open in main.
        let main_texts: Vec<String> = dst
            .iter_items()
            .filter(|i| i.list_id == LIST_INBOX)
            .map(|i| i.text)
            .collect();
        assert!(main_texts.contains(&"local-main".to_string()));
        assert!(main_texts.contains(&"src-main".to_string()));
    }

    #[test]
    fn import_json_orphan_items_fall_back_to_main() {
        // Hand-crafted export with an item pointing at a list_id that
        // isn't in `lists` — same orphan handling as a deleted source
        // list. Should land in main, not silently dropped.
        let export = JsonExport {
            version: 1,
            settings: ExportSettings {
                show_list_counts: false,
                inbox_view: None,
            },
            lists: vec![ExportList {
                id: LIST_INBOX.to_string(),
                name: INBOX_NAME.to_string(),
                icon: None,
                view: None,
                archived_at: None,
                created_at: None,
                builtin: true,
            }],
            items: vec![ExportItem {
                id: "orphan-id".to_string(),
                text: "stranded".to_string(),
                notes: String::new(),
                list_id: "no-such-list".to_string(),
                lifecycle: ExportLifecycle {
                    state: "backlog".to_string(),
                    at: 1_700_000_000_000,
                },
                deadline: None,
                when: None,
                duration: None,
                created_at: 1_700_000_000_000,
                started_at: None,
                done_at: None,
                binned_at: None,
            }],
            focus: vec![],
        };

        let dst = Doc::new().unwrap();
        let summary = dst.import_json(&export).unwrap();
        assert_eq!(summary.items_added, 1);
        assert_eq!(summary.lists_added, 0);

        let items: Vec<ItemView> = dst.iter_items().collect();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].text, "stranded");
        assert_eq!(items[0].list_id, LIST_INBOX);
    }

    #[test]
    fn import_json_str_round_trips_through_serde() {
        let src = Doc::new().unwrap();
        let l = src.add_list("Roundtrip").unwrap();
        let _ = src.add_item(&l, "via-string").unwrap();
        let json = src.export_json_string();

        let dst = Doc::new().unwrap();
        let summary = dst.import_json_str(&json).unwrap();
        assert_eq!(summary.lists_added, 1);
        assert_eq!(summary.items_added, 1);
    }

    #[test]
    fn import_json_rejects_unknown_version() {
        let export = JsonExport {
            version: 99,
            settings: ExportSettings {
                show_list_counts: false,
                inbox_view: None,
            },
            lists: vec![],
            items: vec![],
            focus: vec![],
        };
        let dst = Doc::new().unwrap();
        let err = dst.import_json(&export).unwrap_err();
        assert!(matches!(err, DocError::Invalid(_)));
    }

    #[test]
    fn import_json_emits_item_added_events() {
        let src = Doc::new().unwrap();
        let _ = src.add_item(LIST_INBOX, "e1").unwrap();
        let _ = src.add_item(LIST_INBOX, "e2").unwrap();
        let export = src.export_json();

        let dst = Doc::new().unwrap();
        let _ = dst.drain_events();
        dst.import_json(&export).unwrap();

        let evs = dst.drain_events();
        let added: Vec<&str> = evs
            .iter()
            .filter_map(|e| match e {
                AppEvent::ItemAdded { text, .. } => Some(text.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(added.len(), 2);
        assert!(added.contains(&"e1"));
        assert!(added.contains(&"e2"));
    }

    #[test]
    fn apply_remote_emits_settings_changed_for_peer_toggle() {
        let dek = Dek::generate();
        let a = Doc::new().unwrap();
        let mut b = Doc::new().unwrap();

        // Counts default on, so the meaningful peer toggle is opting out.
        a.set_show_list_counts(false).unwrap();
        let blob = a.pending_export(&dek).unwrap().unwrap();

        b.apply_remote(&dek, &blob).unwrap();

        assert!(!b.get_settings().show_list_counts);
        assert!(matches!(
            b.drain_events().as_slice(),
            [AppEvent::SettingsChanged {
                show_list_counts: false,
                ..
            }]
        ));
    }

    #[test]
    fn export_snapshot_bytes_roundtrips_through_loro_import() {
        // Backup story: bytes from `export_snapshot_bytes` reconstruct
        // the same logical state when imported into a fresh Loro doc.
        let doc = Doc::new().unwrap();
        let other = doc.add_list("Other").unwrap();
        let a = doc.add_item(LIST_INBOX, "a").unwrap();
        let _ = doc.add_item(&other, "b").unwrap();
        doc.set_item_done(&a, true).unwrap();

        let bytes = doc.export_snapshot_bytes().unwrap();
        assert!(!bytes.is_empty());

        let restored_inner = LoroDoc::new();
        restored_inner.import(&bytes).unwrap();
        let undo = Mutex::new(make_undo_manager(&restored_inner));
        let diff_capture = Arc::new(Mutex::new(DiffCapture::default()));
        let _diff_sub = restored_inner.subscribe_root(make_diff_subscriber(diff_capture.clone()));
        let restored = Doc {
            inner: restored_inner,
            last_persisted_vv: VersionVector::default(),
            item_index: Mutex::new(ProjectionIndex::default()),
            events: Mutex::new(VecDeque::new()),
            undo,
            diff_capture,
            notes_shadows: Mutex::new(HashMap::new()),
            _diff_sub,
        };
        restored.rebuild_index();

        // Fingerprint is the canonical "logical-equality" hash used
        // throughout the test suite to assert convergence — same hash
        // ⇒ same doc.
        assert_eq!(doc.fingerprint(), restored.fingerprint());
    }

    #[test]
    fn oplog_replay_rebuilds_item_index() {
        let source = Doc::new().unwrap();
        let id = source.add_item(LIST_INBOX, "replayed").unwrap();
        let updates = source.export_updates_after_bytes(&[]).unwrap();

        let mut restored = Doc::empty();
        restored.import_oplog_updates(&updates).unwrap();
        restored.move_item(&id, LIST_INBOX, 0).unwrap();

        assert_eq!(restored.get_item(&id).unwrap().text, "replayed");
    }

    #[test]
    fn deferred_oplog_replay_rebuilds_once_and_stays_silent() {
        let source = Doc::new().unwrap();
        let first = source.add_item(LIST_INBOX, "first").unwrap();
        let first_updates = source.export_updates_after_bytes(&[]).unwrap();
        let after_first = source.oplog_vv_bytes();
        let second = source.add_item(LIST_INBOX, "second").unwrap();
        let second_updates = source.export_updates_after_bytes(&after_first).unwrap();

        let mut restored = Doc::empty();
        restored.replay_oplog_update(&first_updates).unwrap();
        restored.replay_oplog_update(&second_updates).unwrap();
        // Disposable lookups intentionally remain stale until the one
        // explicit completion point.
        assert!(restored.get_item(&first).is_none());
        restored.finish_oplog_replay();

        assert_eq!(restored.get_item(&first).unwrap().text, "first");
        assert_eq!(restored.get_item(&second).unwrap().text, "second");
        assert!(restored.drain_events().is_empty());
        assert!(!restored.can_undo());
    }

    #[test]
    fn fresh_doc_cannot_undo_seed() {
        // Seed runs before the UndoManager is created, so it isn't on
        // the stack — the doc opens with nothing to undo.
        let doc = Doc::new().unwrap();
        assert!(!doc.can_undo());
        assert!(!doc.can_redo());
        assert!(!doc.undo().unwrap());
    }

    #[test]
    fn undo_after_same_peer_oplog_replay_spares_replayed_history() {
        // Stable leased peer ids mean boot replay carries the *same*
        // peer the live UndoManager is bound to. Loro only advances the
        // manager's internal counter on Local events, so without the
        // finish_oplog_replay re-arm the first local commit after boot
        // records one span stretching back to counter 0 — a single undo
        // would revert the entire replayed history (e.g. a whole JSON
        // import from the previous session).
        const PEER: u64 = 7;
        let source = Doc::new_with_peer(PEER).unwrap();
        let kept_a = source.add_item(LIST_INBOX, "imported a").unwrap();
        let kept_b = source.add_item(LIST_INBOX, "imported b").unwrap();
        let updates = source.export_updates_after_bytes(&[]).unwrap();

        let mut restored = Doc::empty_with_peer(PEER).unwrap();
        restored.replay_oplog_update(&updates).unwrap();
        restored.finish_oplog_replay();
        assert!(!restored.can_undo());

        let fresh = restored.add_item(LIST_INBOX, "post-boot").unwrap();
        assert!(restored.undo().unwrap());

        assert!(restored.get_item(&fresh).is_none());
        assert_eq!(restored.get_item(&kept_a).unwrap().text, "imported a");
        assert_eq!(restored.get_item(&kept_b).unwrap().text, "imported b");
        assert!(!restored.can_undo());
    }

    #[test]
    fn undo_after_same_peer_remote_blob_spares_imported_history() {
        // Same hazard through the encrypted-blob path (native boot_doc
        // replay, or a mid-session snapshot minted under a reused peer
        // slot): an import that advances the local peer's counter must
        // re-arm the UndoManager so the next local commit's undo span
        // starts after the imported ops.
        const PEER: u64 = 11;
        let dek = Dek::generate();
        let mut source = Doc::new_with_peer(PEER).unwrap();
        let kept = source.add_item(LIST_INBOX, "imported").unwrap();
        let blob = source.pending_export(&dek).unwrap().unwrap();

        let mut restored = Doc::empty_with_peer(PEER).unwrap();
        restored.apply_remote(&dek, &blob).unwrap();
        restored.drain_events();

        let fresh = restored.add_item(LIST_INBOX, "post-import").unwrap();
        assert!(restored.undo().unwrap());

        assert!(restored.get_item(&fresh).is_none());
        assert_eq!(restored.get_item(&kept).unwrap().text, "imported");
        assert!(!restored.can_undo());
    }

    #[test]
    fn undo_reverses_local_add_and_redo_replays_it() {
        let doc = Doc::new().unwrap();
        let id = doc.add_item(LIST_INBOX, "buy milk").unwrap();
        assert!(doc.can_undo());

        assert!(doc.undo().unwrap());
        assert!(doc.get_item(&id).is_none());
        assert!(doc.can_redo());

        assert!(doc.redo().unwrap());
        let view = doc.get_item(&id).unwrap();
        assert_eq!(view.text, "buy milk");
    }

    #[test]
    fn undo_emits_app_events() {
        let doc = Doc::new().unwrap();
        let id = doc.add_item(LIST_INBOX, "thing").unwrap();
        let _ = doc.drain_events();

        assert!(doc.undo().unwrap());
        let evs = doc.drain_events();
        assert_eq!(evs, vec![AppEvent::ItemRemoved { id: id.clone() }]);

        assert!(doc.redo().unwrap());
        let evs = doc.drain_events();
        assert_eq!(evs.len(), 1);
        assert!(matches!(
            &evs[0],
            AppEvent::ItemAdded { id: event_id, .. } if event_id == &id
        ));
    }

    #[test]
    fn undo_move_emits_only_the_moved_item() {
        let doc = Doc::new().unwrap();
        let texts: Vec<String> = (0..200).map(|i| format!("item {i}")).collect();
        let refs: Vec<&str> = texts.iter().map(String::as_str).collect();
        let ids = doc.add_items_at(LIST_INBOX, &refs, 0).unwrap();
        let _ = doc.drain_events();

        let moved = ids[199].clone();
        doc.move_item(&moved, LIST_INBOX, 0).unwrap();
        let _ = doc.drain_events();
        assert!(doc.undo().unwrap());

        let evs = doc.drain_events();
        assert_eq!(evs.len(), 1, "undo should be surgical: {evs:?}");
        assert!(matches!(
            &evs[0],
            AppEvent::ItemMoved {
                id,
                open_index: Some(199),
            } if id == &moved
        ));
        assert_eq!(doc.open_item_ids(LIST_INBOX), ids);
    }

    #[test]
    fn undo_redo_round_trips_cross_list_move() {
        let doc = Doc::new().unwrap();
        let other = doc.add_list("Other").unwrap();
        let a = doc.add_item(LIST_INBOX, "a").unwrap();
        let b = doc.add_item(LIST_INBOX, "b").unwrap();
        let c = doc.add_item(LIST_INBOX, "c").unwrap();
        let o = doc.add_item(&other, "o").unwrap();
        let _ = doc.drain_events();

        // Cross-list move is one commit — one undo step.
        doc.move_item(&b, &other, 1).unwrap();
        assert_eq!(doc.open_item_ids(LIST_INBOX), vec![a.clone(), c.clone()]);
        assert_eq!(doc.open_item_ids(&other), vec![o.clone(), b.clone()]);

        assert!(doc.undo().unwrap());
        assert_eq!(
            doc.open_item_ids(LIST_INBOX),
            vec![a.clone(), b.clone(), c.clone()],
            "undo restores the item to its former position in the source list"
        );
        assert_eq!(doc.open_item_ids(&other), vec![o.clone()]);
        assert_eq!(doc.get_item(&b).unwrap().list_id, LIST_INBOX);
        assert_open_projection_matches_doc(&doc);

        assert!(doc.redo().unwrap());
        assert_eq!(doc.open_item_ids(LIST_INBOX), vec![a, c]);
        assert_eq!(doc.open_item_ids(&other), vec![o, b.clone()]);
        assert_eq!(doc.get_item(&b).unwrap().list_id, other);
        assert_open_projection_matches_doc(&doc);
    }

    #[test]
    fn plain_stepwise_undo_redo_round_trips_larger_reorder() {
        let doc = Doc::new().unwrap();
        let texts: Vec<String> = (0..20).map(|i| format!("item {i}")).collect();
        let text_refs: Vec<&str> = texts.iter().map(|s| s.as_str()).collect();
        let ids = doc.add_items_at(LIST_INBOX, &text_refs, 0).unwrap();

        let moved_ids = vec![ids[5].clone(), ids[6].clone(), ids[7].clone()];
        let remaining: Vec<String> = ids
            .iter()
            .filter(|id| !moved_ids.contains(id))
            .cloned()
            .collect();
        let mut next_ids = remaining;
        next_ids.splice(10..10, moved_ids.clone());
        let mut current_ids = ids.clone();
        let mut steps = 0usize;

        for (index, id) in next_ids.iter().enumerate() {
            if current_ids[index] != *id {
                let current_index = current_ids
                    .iter()
                    .position(|cur| cur == id)
                    .expect("moved id must still exist");
                doc.move_item(id, LIST_INBOX, index).unwrap();
                current_ids.remove(current_index);
                current_ids.insert(index, id.clone());
                steps += 1;
            }
        }

        let after_move = doc.open_item_ids(LIST_INBOX);
        assert_eq!(after_move, next_ids);

        for _ in 0..steps {
            assert!(doc.undo().unwrap());
        }
        assert_eq!(doc.open_item_ids(LIST_INBOX), ids);

        for _ in 0..steps {
            assert!(doc.redo().unwrap());
        }
        assert_eq!(doc.open_item_ids(LIST_INBOX), after_move);
    }

    #[test]
    fn undo_skips_remote_ops() {
        // Remote ops imported via `apply_remote` carry origin "remote"
        // and must not be undoable from the local UndoManager. Local
        // mutations made on top of remote state remain undoable.
        let dek = Dek::generate();

        let mut a = Doc::new().unwrap();
        let remote_id = a.add_item(LIST_INBOX, "from A").unwrap();
        let remote_blob = a.pending_export(&dek).unwrap().unwrap();
        a.mark_persisted();

        let mut b = Doc::empty();
        b.apply_remote(&dek, &remote_blob).unwrap();
        // One remote import landed but b has made no local commits.
        assert!(!b.can_undo(), "remote-only ops must not be undoable");

        let local_id = b.add_item(LIST_INBOX, "local on top").unwrap();
        assert!(b.can_undo());

        assert!(b.undo().unwrap());
        assert!(
            b.get_item(&local_id).is_none(),
            "local add should be reversed"
        );
        assert!(
            b.get_item(&remote_id).is_some(),
            "remote item must survive the local undo"
        );
        assert!(
            !b.can_undo(),
            "remote ops still must not be undoable after local undo"
        );
    }

    // ---------- lifecycle (spec/data-model.md, spec/board.md) ----------

    #[test]
    fn new_item_defaults_to_backlog_with_created_at_fallback() {
        let doc = Doc::new().unwrap();
        let id = doc.add_item(LIST_INBOX, "x").unwrap();
        let it = doc.get_item(&id).unwrap();
        assert_eq!(it.lifecycle(), ItemLifecycle::Backlog);
        assert_eq!(it.state, WorkflowState::Backlog);
        assert_eq!(
            it.lifecycle_at, it.created_at,
            "absent register reads as [Backlog, created_at]"
        );
        assert!(it.is_open());
        assert!(it.started_at.is_none() && it.done_at.is_none());
        // The register itself stays unwritten for new items.
        let map = doc.find_item(&id).unwrap();
        assert!(map.get(KEY_LIFECYCLE).is_none());
        assert_eq!(doc.open_item_ids(LIST_INBOX), vec![id]);
    }

    #[test]
    fn add_item_in_state_captures_directly_into_open_lanes() {
        let doc = Doc::new().unwrap();
        for state in [
            WorkflowState::Todo,
            WorkflowState::InProgress,
            WorkflowState::Review,
        ] {
            let id = doc
                .add_item_in_state(LIST_INBOX, state.name(), state)
                .unwrap();
            let it = doc.get_item(&id).unwrap();
            assert_eq!(it.state, state);
            assert!(it.is_open());
            // Capturing straight into In Progress stamps started_at.
            assert_eq!(it.started_at.is_some(), state == WorkflowState::InProgress);
        }
        // Backlog capture leaves the register unwritten (same as add_item).
        let plain = doc
            .add_item_in_state(LIST_INBOX, "plain", WorkflowState::Backlog)
            .unwrap();
        let map = doc.find_item(&plain).unwrap();
        assert!(map.get(KEY_LIFECYCLE).is_none());
        // Done is not a capture lane.
        assert!(matches!(
            doc.add_item_in_state(LIST_INBOX, "nope", WorkflowState::Done)
                .unwrap_err(),
            DocError::Invalid(_)
        ));
    }

    #[test]
    fn open_state_flips_preserve_list_order() {
        let doc = Doc::new().unwrap();
        let a = doc.add_item(LIST_INBOX, "a").unwrap();
        let b = doc.add_item(LIST_INBOX, "b").unwrap();
        let c = doc.add_item(LIST_INBOX, "c").unwrap();
        let ordered = vec![a.clone(), b.clone(), c.clone()];
        assert_eq!(doc.open_item_ids(LIST_INBOX), ordered);
        // Walk b around the open ladder: the shared Open order never moves.
        for lc in [
            ItemLifecycle::Todo,
            ItemLifecycle::InProgress,
            ItemLifecycle::Review,
            ItemLifecycle::Backlog,
        ] {
            doc.set_item_lifecycle(&b, lc).unwrap();
            assert_eq!(doc.open_item_ids(LIST_INBOX), ordered);
            assert_eq!(doc.get_item(&b).unwrap().lifecycle(), lc);
        }
        assert_open_projection_matches_doc(&doc);
    }

    #[test]
    fn undone_lands_in_backlog_not_prior_state() {
        // The workflow ladder has no masking: un-done is a plain write to
        // Backlog, whatever state the item was completed from.
        let doc = Doc::new().unwrap();
        let id = doc
            .add_item_in_state(LIST_INBOX, "x", WorkflowState::Review)
            .unwrap();
        doc.set_item_lifecycle(&id, ItemLifecycle::Done).unwrap();
        assert_eq!(doc.get_item(&id).unwrap().lifecycle(), ItemLifecycle::Done);
        doc.set_item_done(&id, false).unwrap();
        assert_eq!(
            doc.get_item(&id).unwrap().lifecycle(),
            ItemLifecycle::Backlog
        );
        // Un-done on a non-Done item is a no-op, not a Backlog write.
        let open = doc
            .add_item_in_state(LIST_INBOX, "y", WorkflowState::InProgress)
            .unwrap();
        let _ = doc.drain_events();
        doc.set_item_done(&open, false).unwrap();
        assert!(doc.drain_events().is_empty());
        assert_eq!(
            doc.get_item(&open).unwrap().lifecycle(),
            ItemLifecycle::InProgress
        );
    }

    #[test]
    fn binning_and_restoring_preserves_workflow_register() {
        let doc = Doc::new().unwrap();
        let backlog = doc.add_item(LIST_INBOX, "backlog").unwrap();
        let progress = doc
            .add_item_in_state(LIST_INBOX, "progress", WorkflowState::InProgress)
            .unwrap();
        let done = doc.add_item(LIST_INBOX, "done").unwrap();
        doc.set_item_lifecycle(&done, ItemLifecycle::Done).unwrap();
        let done_at_before = doc.get_item(&done).unwrap().lifecycle_at;

        for id in [&backlog, &progress, &done] {
            doc.set_item_lifecycle(id, ItemLifecycle::Binned).unwrap();
            assert_eq!(doc.get_item(id).unwrap().lifecycle(), ItemLifecycle::Binned);
        }
        // Restore (clear the mask only) reveals each preserved state —
        // including Done, with its register timestamp untouched.
        doc.set_item_binned(&backlog, false).unwrap();
        doc.set_item_binned(&progress, false).unwrap();
        doc.set_item_binned(&done, false).unwrap();
        assert_eq!(
            doc.get_item(&backlog).unwrap().lifecycle(),
            ItemLifecycle::Backlog
        );
        assert_eq!(
            doc.get_item(&progress).unwrap().lifecycle(),
            ItemLifecycle::InProgress
        );
        let restored = doc.get_item(&done).unwrap();
        assert_eq!(restored.lifecycle(), ItemLifecycle::Done);
        assert_eq!(
            restored.lifecycle_at, done_at_before,
            "restore never touches the register"
        );
    }

    #[test]
    fn reapplying_resolved_state_is_a_noop() {
        let doc = Doc::new().unwrap();
        let id = doc.add_item(LIST_INBOX, "x").unwrap();
        let _ = doc.drain_events();
        // Backlog onto a fresh (register-less) item: no commit, no event.
        let vv = doc.oplog_vv();
        doc.set_item_lifecycle(&id, ItemLifecycle::Backlog).unwrap();
        assert!(doc.drain_events().is_empty());
        assert_eq!(doc.oplog_vv(), vv);
        // Same-state re-apply after a real transition keeps the timestamp.
        doc.set_item_lifecycle(&id, ItemLifecycle::Todo).unwrap();
        let at = doc.get_item(&id).unwrap().lifecycle_at;
        let _ = doc.drain_events();
        doc.set_item_lifecycle(&id, ItemLifecycle::Todo).unwrap();
        assert!(doc.drain_events().is_empty());
        assert_eq!(doc.get_item(&id).unwrap().lifecycle_at, at);
        // Re-binning an already-binned item is a no-op too.
        doc.set_item_lifecycle(&id, ItemLifecycle::Binned).unwrap();
        let binned_at = doc.get_item(&id).unwrap().binned_at;
        let _ = doc.drain_events();
        doc.set_item_lifecycle(&id, ItemLifecycle::Binned).unwrap();
        assert!(doc.drain_events().is_empty());
        assert_eq!(doc.get_item(&id).unwrap().binned_at, binned_at);
    }

    #[test]
    fn reflection_stamps_ride_transitions() {
        let doc = Doc::new().unwrap();
        let id = doc.add_item(LIST_INBOX, "x").unwrap();
        // First entry into In Progress stamps started_at, write-once.
        doc.set_item_lifecycle(&id, ItemLifecycle::InProgress)
            .unwrap();
        let started = doc.get_item(&id).unwrap().started_at.unwrap();
        std::thread::sleep(std::time::Duration::from_millis(2));
        doc.set_item_lifecycle(&id, ItemLifecycle::Review).unwrap();
        doc.set_item_lifecycle(&id, ItemLifecycle::InProgress)
            .unwrap();
        assert_eq!(
            doc.get_item(&id).unwrap().started_at,
            Some(started),
            "started_at is write-once"
        );
        // Each entry into Done stamps done_at; un-done never clears it.
        doc.set_item_lifecycle(&id, ItemLifecycle::Done).unwrap();
        let first_done = doc.get_item(&id).unwrap().done_at.unwrap();
        doc.set_item_done(&id, false).unwrap();
        assert_eq!(
            doc.get_item(&id).unwrap().done_at,
            Some(first_done),
            "done_at survives un-done"
        );
        std::thread::sleep(std::time::Duration::from_millis(2));
        doc.set_item_lifecycle(&id, ItemLifecycle::Done).unwrap();
        let second_done = doc.get_item(&id).unwrap().done_at.unwrap();
        assert!(second_done > first_done, "done_at re-stamps on each entry");
    }

    #[test]
    fn done_view_sorts_by_register_at_desc() {
        let doc = Doc::new().unwrap();
        let a = doc.add_item(LIST_INBOX, "a").unwrap();
        let b = doc.add_item(LIST_INBOX, "b").unwrap();
        doc.set_item_lifecycle(&a, ItemLifecycle::Done).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(2));
        doc.set_item_lifecycle(&b, ItemLifecycle::Done).unwrap();
        assert_eq!(doc.done_item_ids(), vec![b.clone(), a.clone()]);
        // Re-completing a re-stamps the register; it moves to the top.
        doc.set_item_done(&a, false).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(2));
        doc.set_item_lifecycle(&a, ItemLifecycle::Done).unwrap();
        assert_eq!(doc.done_item_ids(), vec![a, b]);
    }

    #[test]
    fn open_projection_contains_all_four_open_states() {
        let doc = Doc::new().unwrap();
        let backlog = doc.add_item(LIST_INBOX, "backlog").unwrap();
        let todo = doc
            .add_item_in_state(LIST_INBOX, "todo", WorkflowState::Todo)
            .unwrap();
        let progress = doc
            .add_item_in_state(LIST_INBOX, "progress", WorkflowState::InProgress)
            .unwrap();
        let review = doc
            .add_item_in_state(LIST_INBOX, "review", WorkflowState::Review)
            .unwrap();
        let done = doc.add_item(LIST_INBOX, "done").unwrap();
        let binned = doc.add_item(LIST_INBOX, "binned").unwrap();
        doc.set_item_lifecycle(&done, ItemLifecycle::Done).unwrap();
        doc.set_item_lifecycle(&binned, ItemLifecycle::Binned)
            .unwrap();
        assert_eq!(
            doc.open_item_ids(LIST_INBOX),
            vec![backlog, todo, progress, review]
        );
        assert_eq!(doc.done_item_ids(), vec![done]);
        assert_eq!(doc.binned_item_ids(), vec![binned]);
    }

    #[test]
    fn unparseable_register_degrades_to_backlog() {
        let doc = Doc::new().unwrap();
        let id = doc.add_item(LIST_INBOX, "x").unwrap();
        let map = doc.find_item(&id).unwrap();
        // A future client's state code, and outright garbage — both read
        // as the [Backlog, created_at] fallback: visible and open.
        for garbage in [
            LoroValue::List(vec![LoroValue::I64(99), LoroValue::I64(1)].into()),
            LoroValue::String("nonsense".to_string().into()),
            LoroValue::List(vec![LoroValue::I64(1)].into()),
        ] {
            map.insert(KEY_LIFECYCLE, garbage).unwrap();
            doc.inner.commit();
            doc.rebuild_index();
            let it = doc.get_item(&id).unwrap();
            assert_eq!(it.state, WorkflowState::Backlog);
            assert_eq!(it.lifecycle_at, it.created_at);
            assert!(it.is_open(), "future state must never hide data");
            assert_eq!(doc.open_item_ids(LIST_INBOX), vec![id.clone()]);
        }
    }

    #[test]
    fn events_carry_state_and_open_index() {
        let doc = Doc::new().unwrap();
        let _ = doc.drain_events();
        let a = doc.add_item(LIST_INBOX, "a").unwrap(); // Backlog
        let b = doc
            .add_item_in_state(LIST_INBOX, "b", WorkflowState::InProgress)
            .unwrap();
        match doc.drain_events().as_slice() {
            [
                AppEvent::ItemAdded {
                    id: ia,
                    state: sa,
                    open_index: oa,
                    ..
                },
                AppEvent::ItemAdded {
                    id: ib,
                    state: sb,
                    started_at: stb,
                    open_index: ob,
                    ..
                },
            ] => {
                assert_eq!((ia, *sa, *oa), (&a, WorkflowState::Backlog, Some(0)));
                assert_eq!((ib, *sb, *ob), (&b, WorkflowState::InProgress, Some(1)));
                assert!(stb.is_some(), "In Progress capture stamps started_at");
            }
            other => panic!("unexpected add events: {other:?}"),
        }
        // Open→open flip keeps the item at its open index; the event
        // carries the new register state.
        doc.set_item_lifecycle(&a, ItemLifecycle::Todo).unwrap();
        let evs = doc.drain_events();
        assert!(
            matches!(
                evs.as_slice(),
                [AppEvent::ItemLifecycleChanged {
                    id,
                    state: WorkflowState::Todo,
                    done_at: None,
                    binned_at: None,
                    open_index: Some(0),
                    ..
                }] if id == &a
            ),
            "got {evs:?}"
        );
    }

    #[test]
    fn export_import_round_trips_v3_lifecycle() {
        let src = Doc::new().unwrap();
        let _backlog = src.add_item(LIST_INBOX, "backlog").unwrap();
        let _review = src
            .add_item_in_state(LIST_INBOX, "review", WorkflowState::Review)
            .unwrap();
        let started = src
            .add_item_in_state(LIST_INBOX, "started", WorkflowState::InProgress)
            .unwrap();
        let done = src.add_item(LIST_INBOX, "done").unwrap();
        src.set_item_lifecycle(&done, ItemLifecycle::Done).unwrap();

        let export = src.export_json();
        let by_text = |t: &str| export.items.iter().find(|i| i.text == t).unwrap();
        // Every v3 item carries the register with a named state.
        assert_eq!(by_text("backlog").lifecycle.state, "backlog");
        assert_eq!(by_text("review").lifecycle.state, "review");
        assert_eq!(by_text("started").lifecycle.state, "in_progress");
        assert_eq!(by_text("done").lifecycle.state, "done");
        assert!(by_text("started").started_at.is_some());
        assert!(by_text("done").done_at.is_some());

        let dst = Doc::new().unwrap();
        dst.import_json(&export).unwrap();
        let view_of = |t: &str| dst.all_items().into_iter().find(|i| i.text == t).unwrap();
        assert_eq!(view_of("backlog").lifecycle(), ItemLifecycle::Backlog);
        assert_eq!(view_of("review").lifecycle(), ItemLifecycle::Review);
        assert_eq!(view_of("started").lifecycle(), ItemLifecycle::InProgress);
        assert_eq!(view_of("done").lifecycle(), ItemLifecycle::Done);
        assert_eq!(
            view_of("started").started_at,
            src.get_item(&started).unwrap().started_at,
            "reflection stamps round-trip"
        );
        assert_eq!(
            view_of("done").lifecycle_at,
            src.get_item(&done).unwrap().lifecycle_at,
            "register timestamps round-trip"
        );
    }

    #[test]
    fn import_degrades_unknown_register_state_to_backlog() {
        let src = Doc::new().unwrap();
        let _ = src.add_item(LIST_INBOX, "future").unwrap();
        let mut export = src.export_json();
        export.items[0].lifecycle = ExportLifecycle {
            state: "hologram".to_string(),
            at: 42,
        };
        let dst = Doc::new().unwrap();
        dst.import_json(&export).unwrap();
        let it = dst.all_items().into_iter().next().unwrap();
        assert_eq!(it.state, WorkflowState::Backlog);
        assert!(it.is_open());
    }

    #[test]
    fn fingerprint_covers_workflow_state() {
        let doc = Doc::new().unwrap();
        let id = doc.add_item(LIST_INBOX, "x").unwrap();
        let before = doc.fingerprint();
        doc.set_item_lifecycle(&id, ItemLifecycle::Todo).unwrap();
        let todo = doc.fingerprint();
        assert_ne!(todo, before, "register state must diverge the hash");
        // Returning to Backlog writes a *new* [Backlog, now] register —
        // the transition time is logical state, so the hash stays new.
        doc.set_item_lifecycle(&id, ItemLifecycle::Backlog).unwrap();
        assert_ne!(doc.fingerprint(), todo);
    }

    #[test]
    fn concurrent_lifecycle_writes_converge() {
        let dek = Dek::generate();
        let mut a = Doc::new().unwrap();
        let item = a.add_item(LIST_INBOX, "shared").unwrap();
        let mut b = Doc::empty();
        let blob = a.pending_export(&dek).unwrap().unwrap();
        a.mark_persisted();
        b.apply_remote(&dek, &blob).unwrap();

        // Concurrent divergent transitions: A marks Done, B marks Review.
        a.set_item_lifecycle(&item, ItemLifecycle::Done).unwrap();
        b.set_item_lifecycle(&item, ItemLifecycle::Review).unwrap();
        let blob_a = a.pending_export(&dek).unwrap().unwrap();
        let blob_b = b.pending_export(&dek).unwrap().unwrap();
        a.mark_persisted();
        b.mark_persisted();
        a.apply_remote(&dek, &blob_b).unwrap();
        b.apply_remote(&dek, &blob_a).unwrap();

        assert_eq!(a.fingerprint(), b.fingerprint(), "replicas must converge");
        // The whole-value register merged by LWW: both replicas resolve
        // the identical state and timestamp.
        let (va, vb) = (a.get_item(&item).unwrap(), b.get_item(&item).unwrap());
        assert_eq!(va.lifecycle(), vb.lifecycle());
        assert_eq!(va.lifecycle_at, vb.lifecycle_at);
        assert_open_projection_matches_doc(&a);
        assert_open_projection_matches_doc(&b);
    }

    #[test]
    fn concurrent_bin_and_transition_converge_to_both() {
        // A bins while B (offline) moves the item to Review: the merge is
        // binned with Review preserved underneath for restore.
        let dek = Dek::generate();
        let mut a = Doc::new().unwrap();
        let item = a.add_item(LIST_INBOX, "shared").unwrap();
        let seed = a.pending_export(&dek).unwrap().unwrap();
        a.mark_persisted();
        let mut b = Doc::empty();
        b.apply_remote(&dek, &seed).unwrap();

        a.set_item_lifecycle(&item, ItemLifecycle::Binned).unwrap();
        b.set_item_lifecycle(&item, ItemLifecycle::Review).unwrap();
        let blob_a = a.pending_export(&dek).unwrap().unwrap();
        let blob_b = b.pending_export(&dek).unwrap().unwrap();
        a.mark_persisted();
        b.mark_persisted();
        a.apply_remote(&dek, &blob_b).unwrap();
        b.apply_remote(&dek, &blob_a).unwrap();

        assert_eq!(a.fingerprint(), b.fingerprint());
        let v = a.get_item(&item).unwrap();
        assert_eq!(v.lifecycle(), ItemLifecycle::Binned);
        assert_eq!(v.state, WorkflowState::Review);
        // Restore reveals the other device's transition.
        a.set_item_binned(&item, false).unwrap();
        assert_eq!(
            a.get_item(&item).unwrap().lifecycle(),
            ItemLifecycle::Review
        );
    }

    // ---------- focus (spec/focus.md) ----------

    #[test]
    fn focus_ref_encode_parse_roundtrips_both_forms() {
        let local = FocusRef::local("abc");
        assert_eq!(local.encode(), "abc");
        assert_eq!(FocusRef::parse("abc"), Some(FocusRef::local("abc")));
        assert!(FocusRef::parse("abc").unwrap().is_local());

        let cross = FocusRef {
            doc_id: Some("doc1".into()),
            item_id: "item2".into(),
        };
        assert_eq!(cross.encode(), "doc1:item2");
        assert_eq!(FocusRef::parse("doc1:item2"), Some(cross));
        assert!(!FocusRef::parse("doc1:item2").unwrap().is_local());

        // Malformed: empty, empty component either side.
        assert_eq!(FocusRef::parse(""), None);
        assert_eq!(FocusRef::parse(":x"), None);
        assert_eq!(FocusRef::parse("x:"), None);
    }

    #[test]
    fn focus_add_reorder_remove() {
        let doc = Doc::new().unwrap();
        let a = doc.add_item(LIST_INBOX, "a").unwrap();
        let b = doc.add_item(LIST_INBOX, "b").unwrap();
        let c = doc.add_item(LIST_INBOX, "c").unwrap();

        doc.add_to_focus(&a, usize::MAX).unwrap();
        doc.add_to_focus(&b, usize::MAX).unwrap();
        doc.add_to_focus(&c, 0).unwrap(); // insert at top
        assert_eq!(doc.focus_refs(), vec![c.clone(), a.clone(), b.clone()]);
        assert_eq!(
            doc.focus_view()
                .iter()
                .map(|v| v.id.clone())
                .collect::<Vec<_>>(),
            vec![c.clone(), a.clone(), b.clone()]
        );

        doc.move_in_focus(&c, 2).unwrap(); // c to the bottom
        assert_eq!(doc.focus_refs(), vec![a.clone(), b.clone(), c.clone()]);

        doc.remove_from_focus(&b).unwrap();
        assert_eq!(doc.focus_refs(), vec![a, c]);
    }

    #[test]
    fn focus_add_already_focused_is_noop() {
        let doc = Doc::new().unwrap();
        let a = doc.add_item(LIST_INBOX, "a").unwrap();
        let b = doc.add_item(LIST_INBOX, "b").unwrap();
        doc.add_to_focus(&a, usize::MAX).unwrap();
        doc.add_to_focus(&b, usize::MAX).unwrap();
        // Re-adding `a` must not move it to top and must not duplicate.
        doc.add_to_focus(&a, 0).unwrap();
        assert_eq!(doc.focus_refs(), vec![a, b]);
        assert_eq!(doc.focus_list().len(), 2);
    }

    #[test]
    fn focus_add_many_prepends_in_order_and_skips_ineligible() {
        let doc = Doc::new().unwrap();
        let a = doc.add_item(LIST_INBOX, "a").unwrap();
        let b = doc.add_item(LIST_INBOX, "b").unwrap();
        let c = doc.add_item(LIST_INBOX, "c").unwrap();
        let d = doc.add_item(LIST_INBOX, "d").unwrap();
        // `a` is already focused; `c` is done (ineligible); `d` repeats.
        doc.add_to_focus(&a, usize::MAX).unwrap();
        doc.set_item_done(&c, true).unwrap();

        doc.add_to_focus_many(&[&a, &b, &c, &d, &d, "deadbeef"])
            .unwrap();
        // b + d prepended to the top in order, landing above the already-
        // present a; c skipped (done), the duplicate d and unknown id
        // skipped.
        assert_eq!(doc.focus_refs(), vec![b, d, a]);
    }

    #[test]
    fn focus_remove_many_removes_all_targets_in_one_commit() {
        let doc = Doc::new().unwrap();
        let a = doc.add_item(LIST_INBOX, "a").unwrap();
        let b = doc.add_item(LIST_INBOX, "b").unwrap();
        let c = doc.add_item(LIST_INBOX, "c").unwrap();
        doc.add_to_focus(&a, usize::MAX).unwrap();
        doc.add_to_focus(&b, usize::MAX).unwrap();
        doc.add_to_focus(&c, usize::MAX).unwrap();

        doc.remove_from_focus_many(&[&a, &c, "deadbeef"]).unwrap();
        assert_eq!(doc.focus_refs(), vec![b]);
    }

    #[test]
    fn focus_add_unknown_item_errors() {
        let doc = Doc::new().unwrap();
        assert!(matches!(
            doc.add_to_focus("deadbeef", 0).unwrap_err(),
            DocError::ItemNotFound(_)
        ));
    }

    #[test]
    fn focus_add_done_item_is_noop() {
        let doc = Doc::new().unwrap();
        let a = doc.add_item(LIST_INBOX, "a").unwrap();
        doc.set_item_done(&a, true).unwrap();
        doc.add_to_focus(&a, usize::MAX).unwrap();
        assert!(doc.focus_refs().is_empty());
        assert_eq!(doc.focus_list().len(), 0);
    }

    #[test]
    fn focus_dedup_first_wins_and_reconcile_prunes() {
        let doc = Doc::new().unwrap();
        let a = doc.add_item(LIST_INBOX, "a").unwrap();
        doc.add_to_focus(&a, usize::MAX).unwrap();
        // Inject a raw duplicate directly (a concurrent double-add shape).
        doc.focus_list()
            .push(FocusRef::local(&a).encode().as_str())
            .unwrap();
        doc.inner.commit();
        assert_eq!(doc.focus_list().len(), 2);
        // Projection dedups (first wins), read never mutates.
        assert_eq!(doc.focus_refs(), vec![a.clone()]);
        assert_eq!(doc.focus_list().len(), 2);
        // Reconcile prunes the duplicate.
        assert_eq!(doc.reconcile().unwrap(), 1);
        assert_eq!(doc.focus_list().len(), 1);
        assert_eq!(doc.focus_refs(), vec![a]);
        assert_eq!(doc.reconcile().unwrap(), 0);
    }

    #[test]
    fn focus_done_auto_removes_ref_and_undone_does_not_reappear() {
        let doc = Doc::new().unwrap();
        let a = doc.add_item(LIST_INBOX, "a").unwrap();
        doc.add_to_focus(&a, usize::MAX).unwrap();
        assert_eq!(doc.focus_list().len(), 1);
        let _ = doc.drain_events();

        // Done evaporates the item from Focus AND removes the underlying ref.
        doc.set_item_done(&a, true).unwrap();
        assert!(doc.focus_refs().is_empty());
        assert_eq!(doc.focus_list().len(), 0, "ref physically removed on Done");
        assert!(
            doc.drain_events().contains(&AppEvent::FocusChanged),
            "Done that clears a focus ref emits FocusChanged"
        );

        // Un-done does NOT bring it back — re-add is deliberate.
        doc.set_item_done(&a, false).unwrap();
        assert!(doc.focus_refs().is_empty());
        assert_eq!(doc.focus_list().len(), 0);
    }

    #[test]
    fn focus_done_removes_ref_via_lifecycle_and_bulk_paths() {
        // The board uses set_item_lifecycle; other callers use set_items_done.
        // Both must self-compact Focus, like set_item_done.
        let doc = Doc::new().unwrap();
        let a = doc.add_item(LIST_INBOX, "a").unwrap();
        let b = doc.add_item(LIST_INBOX, "b").unwrap();
        doc.add_to_focus(&a, usize::MAX).unwrap();
        doc.add_to_focus(&b, usize::MAX).unwrap();

        doc.set_item_lifecycle(&a, ItemLifecycle::Done).unwrap();
        assert_eq!(doc.focus_refs(), vec![b.clone()]);
        assert_eq!(doc.focus_list().len(), 1);

        doc.set_items_done(&[b.as_str()], true).unwrap();
        assert!(doc.focus_refs().is_empty());
        assert_eq!(doc.focus_list().len(), 0);
    }

    #[test]
    fn focus_binned_is_filtered_then_swept_on_next_interaction() {
        let doc = Doc::new().unwrap();
        let a = doc.add_item(LIST_INBOX, "a").unwrap();
        let b = doc.add_item(LIST_INBOX, "b").unwrap();
        doc.add_to_focus(&a, usize::MAX).unwrap();

        // Binning does NOT touch the focus container (unlike Done).
        doc.set_item_binned(&a, true).unwrap();
        assert!(doc.focus_refs().is_empty(), "binned filtered from view");
        assert_eq!(doc.focus_list().len(), 1, "ref left in place, not swept");

        // The next focus interaction folds in the sweep.
        doc.add_to_focus(&b, usize::MAX).unwrap();
        assert_eq!(doc.focus_refs(), vec![b]);
        assert_eq!(doc.focus_list().len(), 1, "dead binned ref swept");
    }

    #[test]
    fn focus_hard_delete_leaves_dead_ref_gcd_by_reconcile() {
        let doc = Doc::new().unwrap();
        let a = doc.add_item(LIST_INBOX, "a").unwrap();
        doc.add_to_focus(&a, usize::MAX).unwrap();
        doc.set_item_binned(&a, true).unwrap();
        doc.delete_binned(&a).unwrap(); // hard delete
        assert!(doc.focus_refs().is_empty(), "missing item filtered out");
        assert_eq!(doc.focus_list().len(), 1, "dead ref survives as garbage");
        assert_eq!(doc.reconcile().unwrap(), 1);
        assert_eq!(doc.focus_list().len(), 0);
    }

    #[test]
    fn focus_local_add_and_remote_apply_emit_focus_changed() {
        let dek = Dek::generate();
        let mut a = Doc::new().unwrap();
        let x = a.add_item(LIST_INBOX, "x").unwrap();
        let mut b = sync_fresh_peer(&mut a, &dek);

        a.add_to_focus(&x, usize::MAX).unwrap();
        assert!(
            a.drain_events().contains(&AppEvent::FocusChanged),
            "local focus add emits FocusChanged"
        );
        let blob = a.pending_export(&dek).unwrap().unwrap();
        b.apply_remote(&dek, &blob).unwrap();
        let evs = b.drain_events();
        assert!(
            evs.contains(&AppEvent::FocusChanged),
            "remote focus op emits FocusChanged, got {evs:?}"
        );
        assert!(
            !evs.contains(&AppEvent::FullResync),
            "focus-only frame should translate surgically, got {evs:?}"
        );
        assert_eq!(b.focus_refs(), vec![x]);
    }

    #[test]
    fn focus_concurrent_add_converges() {
        let dek = Dek::generate();
        let mut a = Doc::new().unwrap();
        let x = a.add_item(LIST_INBOX, "x").unwrap();
        let y = a.add_item(LIST_INBOX, "y").unwrap();
        let mut b = sync_fresh_peer(&mut a, &dek);

        // Concurrent, divergent focus adds.
        a.add_to_focus(&x, usize::MAX).unwrap();
        b.add_to_focus(&y, usize::MAX).unwrap();
        let blob_a = a.pending_export(&dek).unwrap().unwrap();
        let blob_b = b.pending_export(&dek).unwrap().unwrap();
        a.mark_persisted();
        b.mark_persisted();
        a.apply_remote(&dek, &blob_b).unwrap();
        b.apply_remote(&dek, &blob_a).unwrap();

        assert_eq!(a.fingerprint(), b.fingerprint(), "replicas must converge");
        let refs = a.focus_refs();
        assert_eq!(refs.len(), 2);
        assert!(refs.contains(&x) && refs.contains(&y));
    }

    #[test]
    fn fingerprint_includes_focus_membership_and_order() {
        let dek = Dek::generate();
        let mut a = Doc::new().unwrap();
        let x = a.add_item(LIST_INBOX, "x").unwrap();
        let y = a.add_item(LIST_INBOX, "y").unwrap();
        let z = a.add_item(LIST_INBOX, "z").unwrap();
        let b = sync_fresh_peer(&mut a, &dek);
        assert_eq!(a.fingerprint(), b.fingerprint());

        // Adding a focus ref diverges the hash.
        a.add_to_focus(&x, usize::MAX).unwrap();
        assert_ne!(a.fingerprint(), b.fingerprint(), "focus membership hashed");

        // Curated order is hashed too.
        a.add_to_focus(&y, usize::MAX).unwrap();
        a.add_to_focus(&z, usize::MAX).unwrap();
        let before = a.fingerprint();
        a.move_in_focus(&z, 0).unwrap();
        assert_ne!(a.fingerprint(), before, "focus order hashed");
    }

    // ---- notes as mergeable LoroText (spec/notes-plan.md Phase 1) ----

    /// Push every uncaptured op both ways: `a`'s pending blob to `b`,
    /// then `b`'s to `a`. Events on both sides are left in place.
    fn exchange(a: &mut Doc, b: &mut Doc, dek: &Dek) {
        if let Some(blob) = a.pending_export(dek).unwrap() {
            b.apply_remote(dek, &blob).unwrap();
        }
        a.mark_persisted();
        if let Some(blob) = b.pending_export(dek).unwrap() {
            a.apply_remote(dek, &blob).unwrap();
        }
        b.mark_persisted();
    }

    fn notes_container_count(doc: &Doc, id: &str) -> usize {
        let map = doc.find_item(id).unwrap();
        let mut n = 0;
        map.for_each(|k, v| {
            if k == KEY_NOTES && matches!(v, ValueOrContainer::Container(_)) {
                n += 1;
            }
        });
        n
    }

    #[test]
    fn notes_first_writes_on_two_peers_merge_into_one_text() {
        let dek = Dek::generate();
        let mut a = Doc::new().unwrap();
        let id = a.add_item(LIST_INBOX, "item").unwrap();
        let mut b = sync_fresh_peer(&mut a, &dek);

        // Neither peer has a notes container yet; both create one
        // offline. Mergeable ids make it the same container.
        a.edit_item_notes(&id, "from a").unwrap();
        b.edit_item_notes(&id, "from b").unwrap();
        exchange(&mut a, &mut b, &dek);

        let na = a.get_item(&id).unwrap().notes;
        let nb = b.get_item(&id).unwrap().notes;
        assert_eq!(na, nb, "peers converge");
        assert!(na.contains("from a") && na.contains("from b"), "got {na:?}");
        assert_eq!(notes_container_count(&a, &id), 1);
        assert_eq!(notes_container_count(&b, &id), 1);
        assert_eq!(a.fingerprint(), b.fingerprint());
    }

    #[test]
    fn notes_edits_to_different_regions_both_survive() {
        let dek = Dek::generate();
        let mut a = Doc::new().unwrap();
        let id = a.add_item(LIST_INBOX, "item").unwrap();
        a.edit_item_notes(&id, "alpha\nbeta").unwrap();
        let mut b = sync_fresh_peer(&mut a, &dek);

        a.edit_item_notes(&id, "ALPHA\nbeta").unwrap();
        b.edit_item_notes(&id, "alpha\nBETA").unwrap();
        exchange(&mut a, &mut b, &dek);

        assert_eq!(a.get_item(&id).unwrap().notes, "ALPHA\nBETA");
        assert_eq!(b.get_item(&id).unwrap().notes, "ALPHA\nBETA");
    }

    #[test]
    fn notes_same_region_edits_merge_character_wise() {
        let dek = Dek::generate();
        let mut a = Doc::new().unwrap();
        let id = a.add_item(LIST_INBOX, "item").unwrap();
        a.edit_item_notes(&id, "hello").unwrap();
        let mut b = sync_fresh_peer(&mut a, &dek);

        a.edit_item_notes(&id, "hello world").unwrap();
        b.edit_item_notes(&id, "hello there").unwrap();
        exchange(&mut a, &mut b, &dek);

        let na = a.get_item(&id).unwrap().notes;
        assert_eq!(na, b.get_item(&id).unwrap().notes);
        assert!(na.starts_with("hello"), "got {na:?}");
        assert!(
            na.contains(" world") && na.contains(" there"),
            "nothing dropped: {na:?}"
        );
    }

    #[test]
    fn remote_notes_edit_translates_to_one_surgical_event() {
        let dek = Dek::generate();
        let mut a = Doc::new().unwrap();
        let id = a.add_item(LIST_INBOX, "item").unwrap();
        let other = a.add_item(LIST_INBOX, "other").unwrap();
        let mut b = sync_fresh_peer(&mut a, &dek);

        // First write: marker op on the item map plus the text insert.
        a.edit_item_notes(&id, "first").unwrap();
        let blob = a.pending_export(&dek).unwrap().unwrap();
        a.mark_persisted();
        b.apply_remote(&dek, &blob).unwrap();
        let evs = b.drain_events();
        assert_eq!(
            evs,
            vec![AppEvent::ItemNotesChanged {
                id: id.clone(),
                notes: "first".into(),
            }],
            "first write: {evs:?}"
        );

        // Second write: only the text container changes. This is the
        // `LoroDiff::Text` classifier arm; without it the frame is
        // opaque and forces a FullResync.
        a.edit_item_notes(&id, "first, then more").unwrap();
        let blob = a.pending_export(&dek).unwrap().unwrap();
        a.mark_persisted();
        b.apply_remote(&dek, &blob).unwrap();
        let evs = b.drain_events();
        assert_eq!(
            evs,
            vec![AppEvent::ItemNotesChanged {
                id: id.clone(),
                notes: "first, then more".into(),
            }],
            "text-only write: {evs:?}"
        );
        assert_eq!(b.get_item(&other).unwrap().notes, "");
        assert_eq!(a.fingerprint(), b.fingerprint());
    }

    #[test]
    fn notes_clear_keeps_key_and_concurrent_append_survives() {
        let dek = Dek::generate();
        let mut a = Doc::new().unwrap();
        let id = a.add_item(LIST_INBOX, "item").unwrap();
        a.edit_item_notes(&id, "hello").unwrap();
        let mut b = sync_fresh_peer(&mut a, &dek);

        a.edit_item_notes(&id, "").unwrap();
        assert_eq!(
            notes_container_count(&a, &id),
            1,
            "clear deletes content, never the key"
        );
        assert_eq!(a.get_item(&id).unwrap().notes, "");
        b.edit_item_notes(&id, "hello world").unwrap();
        exchange(&mut a, &mut b, &dek);

        assert_eq!(a.get_item(&id).unwrap().notes, " world");
        assert_eq!(b.get_item(&id).unwrap().notes, " world");
    }

    #[test]
    fn notes_clear_then_type_yields_only_the_typed_text() {
        // The resurface bug a key-delete design would have: `hello`
        // cleared, then `h` typed, must read `h` on every peer, not
        // `hhello`.
        let dek = Dek::generate();
        let mut a = Doc::new().unwrap();
        let id = a.add_item(LIST_INBOX, "item").unwrap();
        a.edit_item_notes(&id, "hello").unwrap();
        let mut b = sync_fresh_peer(&mut a, &dek);

        a.edit_item_notes(&id, "").unwrap();
        a.edit_item_notes(&id, "h").unwrap();
        exchange(&mut a, &mut b, &dek);

        assert_eq!(a.get_item(&id).unwrap().notes, "h");
        assert_eq!(b.get_item(&id).unwrap().notes, "h");
        assert_eq!(a.fingerprint(), b.fingerprint());
    }

    #[test]
    fn notes_unchanged_write_is_a_no_op() {
        let doc = Doc::new().unwrap();
        let id = doc.add_item(LIST_INBOX, "item").unwrap();
        doc.edit_item_notes(&id, "same").unwrap();
        let _ = doc.drain_events();
        let vv = doc.inner.oplog_vv();

        doc.edit_item_notes(&id, "same").unwrap();
        assert!(doc.drain_events().is_empty());
        assert_eq!(doc.inner.oplog_vv(), vv, "no ops written");
    }

    #[test]
    fn notes_undo_restores_previous_text() {
        let doc = Doc::new().unwrap();
        let id = doc.add_item(LIST_INBOX, "item").unwrap();
        doc.edit_item_notes(&id, "one").unwrap();
        doc.edit_item_notes(&id, "one two").unwrap();
        let _ = doc.drain_events();

        assert!(doc.undo().unwrap());
        assert_eq!(doc.get_item(&id).unwrap().notes, "one");
        let evs = doc.drain_events();
        assert_eq!(
            evs,
            vec![AppEvent::ItemNotesChanged {
                id: id.clone(),
                notes: "one".into(),
            }]
        );
        assert!(doc.redo().unwrap());
        assert_eq!(doc.get_item(&id).unwrap().notes, "one two");
    }

    #[test]
    fn v3_json_export_imports_into_v4_with_notes() {
        // The JSON shape did not change across v3 -> v4 (notes is a
        // string in both), so a v3 export imports as-is.
        let src = Doc::new().unwrap();
        let a = src.add_item(LIST_INBOX, "alpha").unwrap();
        src.edit_item_notes(&a, "  keep my\nnotes  ").unwrap();
        let json = src.export_json_string();

        let dst = Doc::new().unwrap();
        dst.import_json_str(&json).unwrap();
        let items: Vec<ItemView> = dst.iter_items().collect();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].notes, "keep my\nnotes");
        assert_eq!(notes_container_count(&dst, &items[0].id), 1);
        // Round trip again: hashes of the two imported docs agree on
        // notes content (ids are regenerated, so compare views).
        let again = Doc::new().unwrap();
        again.import_json_str(&dst.export_json_string()).unwrap();
        let v: Vec<ItemView> = again.iter_items().collect();
        assert_eq!(v[0].notes, items[0].notes);
    }

    // ---- notes delta bridge (spec/notes-plan.md Phase 2) ----

    fn d_retain(n: usize) -> NotesDeltaOp {
        NotesDeltaOp::Retain { retain: n }
    }
    fn d_insert(s: &str) -> NotesDeltaOp {
        NotesDeltaOp::Insert {
            insert: s.to_string(),
        }
    }
    fn d_delete(n: usize) -> NotesDeltaOp {
        NotesDeltaOp::Delete { delete: n }
    }

    fn notes_deltas(evs: &[AppEvent]) -> Vec<(String, Vec<NotesDeltaOp>)> {
        evs.iter()
            .filter_map(|e| match e {
                AppEvent::ItemNotesDelta { id, delta } => Some((id.clone(), delta.clone())),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn notes_delta_ops_serialise_in_quill_shape() {
        let ops = vec![d_retain(2), d_insert("hi"), d_delete(1)];
        let json = serde_json::to_string(&ops).unwrap();
        assert_eq!(json, r#"[{"retain":2},{"insert":"hi"},{"delete":1}]"#);
        let back: Vec<NotesDeltaOp> = serde_json::from_str(&json).unwrap();
        assert_eq!(back, ops);
    }

    #[test]
    fn apply_notes_delta_edits_in_utf16_units() {
        let doc = Doc::new().unwrap();
        let id = doc.add_item(LIST_INBOX, "item").unwrap();
        doc.apply_notes_delta(&id, &[d_insert("a😀b")]).unwrap();
        assert_eq!(doc.get_item(&id).unwrap().notes, "a😀b");
        // 😀 is one scalar, two UTF-16 units: an editor offset of 3 is
        // after it.
        doc.apply_notes_delta(&id, &[d_retain(3), d_insert("X")])
            .unwrap();
        assert_eq!(doc.get_item(&id).unwrap().notes, "a😀Xb");
        doc.apply_notes_delta(&id, &[d_retain(1), d_delete(2)])
            .unwrap();
        assert_eq!(doc.get_item(&id).unwrap().notes, "aXb");
        let evs = doc.drain_events();
        assert!(
            evs.iter()
                .all(|e| !matches!(e, AppEvent::ItemNotesDelta { .. })),
            "local applies never echo a delta: {evs:?}"
        );
        assert_eq!(
            evs.iter()
                .filter(|e| matches!(e, AppEvent::ItemNotesChanged { .. }))
                .count(),
            3,
            "{evs:?}"
        );
    }

    #[test]
    fn apply_notes_delta_rejects_bad_deltas_whole() {
        let doc = Doc::new().unwrap();
        let id = doc.add_item(LIST_INBOX, "item").unwrap();
        doc.apply_notes_delta(&id, &[d_insert("a😀b")]).unwrap();
        let vv = doc.inner.oplog_vv();
        // Split the surrogate pair.
        let err = doc
            .apply_notes_delta(&id, &[d_retain(2), d_insert("X")])
            .unwrap_err();
        assert!(matches!(err, DocError::Invalid(_)), "{err:?}");
        // Retain past the end, after a valid first step.
        let err = doc
            .apply_notes_delta(&id, &[d_insert("ok"), d_retain(9), d_insert("X")])
            .unwrap_err();
        assert!(matches!(err, DocError::Invalid(_)), "{err:?}");
        let err = doc
            .apply_notes_delta(&id, &[d_retain(1), d_delete(1)])
            .unwrap_err();
        assert!(matches!(err, DocError::Invalid(_)), "{err:?}");
        assert_eq!(doc.get_item(&id).unwrap().notes, "a😀b");
        assert_eq!(doc.inner.oplog_vv(), vv, "nothing written");
    }

    #[test]
    fn notes_delta_commits_are_excluded_from_workspace_undo() {
        let doc = Doc::new().unwrap();
        let id = doc.add_item(LIST_INBOX, "item").unwrap();
        doc.apply_notes_delta(&id, &[d_insert("typed")]).unwrap();
        let _ = doc.drain_events();
        // The only undoable step is the add; the notes commit is skipped.
        assert!(doc.undo().unwrap());
        assert!(doc.get_item(&id).is_none());
        assert!(!doc.can_undo());
    }

    #[test]
    fn remote_notes_edit_streams_a_utf16_delta_to_subscribers() {
        let dek = Dek::generate();
        let mut a = Doc::new().unwrap();
        let id = a.add_item(LIST_INBOX, "item").unwrap();
        let other = a.add_item(LIST_INBOX, "other").unwrap();
        a.edit_item_notes(&id, "a😀b").unwrap();
        a.edit_item_notes(&other, "x").unwrap();
        let mut b = sync_fresh_peer(&mut a, &dek);

        assert_eq!(b.subscribe_notes(&id).unwrap(), "a😀b");
        a.apply_notes_delta(&id, &[d_retain(4), d_insert("!")])
            .unwrap();
        a.edit_item_notes(&other, "xy").unwrap();
        exchange(&mut a, &mut b, &dek);

        let evs = b.drain_events();
        assert!(!evs.contains(&AppEvent::FullResync), "{evs:?}");
        assert_eq!(
            notes_deltas(&evs),
            vec![(id.clone(), vec![d_retain(4), d_insert("!")])],
            "subscribed item streams, unsubscribed does not: {evs:?}"
        );
        assert!(evs.contains(&AppEvent::ItemNotesChanged {
            id: other.clone(),
            notes: "xy".into()
        }));
        assert_eq!(b.get_item(&id).unwrap().notes, "a😀b!");

        // Unsubscribe: no more deltas, whole-string events continue.
        b.unsubscribe_notes(&id);
        a.apply_notes_delta(&id, &[d_delete(1)]).unwrap();
        exchange(&mut a, &mut b, &dek);
        let evs = b.drain_events();
        assert!(notes_deltas(&evs).is_empty(), "{evs:?}");
        assert!(evs.contains(&AppEvent::ItemNotesChanged {
            id: id.clone(),
            notes: "😀b!".into()
        }));
    }

    #[test]
    fn remote_delta_converts_against_the_locally_advanced_shadow() {
        let dek = Dek::generate();
        let mut a = Doc::new().unwrap();
        let id = a.add_item(LIST_INBOX, "item").unwrap();
        a.edit_item_notes(&id, "hello").unwrap();
        let mut b = sync_fresh_peer(&mut a, &dek);

        // a's editor is open; a types at the end while b prepends
        // offline. The shadow on a must already include a's own edit
        // when b's insert is converted.
        a.subscribe_notes(&id).unwrap();
        a.apply_notes_delta(&id, &[d_retain(5), d_insert(" a")])
            .unwrap();
        b.apply_notes_delta(&id, &[d_insert("😀 ")]).unwrap();
        let _ = a.drain_events();
        exchange(&mut a, &mut b, &dek);

        assert_eq!(a.get_item(&id).unwrap().notes, "😀 hello a");
        let evs = a.drain_events();
        assert_eq!(
            notes_deltas(&evs),
            vec![(id.clone(), vec![d_insert("😀 ")])],
            "{evs:?}"
        );
        // A follow-up remote edit after the emoji still converts in
        // UTF-16 (😀 = 2 units): b appends after "hello".
        b.apply_notes_delta(&id, &[d_retain(8), d_insert("!")])
            .unwrap();
        exchange(&mut a, &mut b, &dek);
        assert_eq!(a.get_item(&id).unwrap().notes, "😀 hello! a");
        let evs = a.drain_events();
        assert_eq!(
            notes_deltas(&evs),
            vec![(id.clone(), vec![d_retain(8), d_insert("!")])],
            "{evs:?}"
        );
    }

    #[test]
    fn undo_of_whole_string_notes_write_streams_a_delta() {
        let doc = Doc::new().unwrap();
        let id = doc.add_item(LIST_INBOX, "item").unwrap();
        doc.edit_item_notes(&id, "one").unwrap();
        doc.edit_item_notes(&id, "one two").unwrap();
        doc.subscribe_notes(&id).unwrap();
        let _ = doc.drain_events();
        assert!(doc.undo().unwrap());
        let evs = doc.drain_events();
        assert_eq!(doc.get_item(&id).unwrap().notes, "one");
        assert_eq!(
            notes_deltas(&evs),
            vec![(id.clone(), vec![d_retain(3), d_delete(4)])],
            "{evs:?}"
        );
    }

    #[test]
    fn utf16_conversion_and_compose_helpers() {
        // Scalar delta over "a😀b": retain 2 scalars (a, 😀) = 3 units.
        let scalar = vec![
            TextDelta::Retain {
                retain: 2,
                attributes: None,
            },
            TextDelta::Insert {
                insert: "X".into(),
                attributes: None,
            },
            TextDelta::Delete { delete: 1 },
        ];
        let (ops, post) = utf16_delta_from_scalar("a😀b", &scalar).unwrap();
        assert_eq!(ops, vec![d_retain(3), d_insert("X"), d_delete(1)]);
        assert_eq!(post, "a😀X");
        // Out of step: retain past the shadow.
        assert!(utf16_delta_from_scalar("ab", &scalar).is_none());

        assert_eq!(replace_delta("a😀", "z"), vec![d_delete(3), d_insert("z")]);
        assert!(replace_delta("same", "same").is_empty());
        assert_eq!(replace_delta("", "new"), vec![d_insert("new")]);

        // compose: insert "ab" at 0, then delete 1 at 0 -> insert "b".
        let c = compose_utf16_deltas(&[d_insert("ab")], &[d_delete(1)]);
        assert_eq!(c, vec![d_insert("b")]);
        // retain 2 + insert "X", then retain 1 + delete 1 (deletes the
        // original 2nd char) -> retain 1, delete 1, insert "X".
        let c = compose_utf16_deltas(&[d_retain(2), d_insert("X")], &[d_retain(1), d_delete(1)]);
        assert_eq!(c, vec![d_retain(1), d_delete(1), d_insert("X")]);
        // Applying compose(a, b) equals applying a then b.
        let apply = |text: &str, ops: &[NotesDeltaOp]| -> String {
            let units: Vec<u16> = text.encode_utf16().collect();
            let mut out: Vec<u16> = Vec::new();
            let mut pos = 0;
            for op in ops {
                match op {
                    NotesDeltaOp::Retain { retain } => {
                        out.extend(&units[pos..pos + retain]);
                        pos += retain;
                    }
                    NotesDeltaOp::Delete { delete } => pos += delete,
                    NotesDeltaOp::Insert { insert } => out.extend(insert.encode_utf16()),
                }
            }
            out.extend(&units[pos..]);
            String::from_utf16(&out).unwrap()
        };
        let a_ops = vec![d_retain(2), d_insert("XY"), d_delete(1)];
        let b_ops = vec![
            d_retain(1),
            d_delete(2),
            d_insert("q"),
            d_retain(1),
            d_insert("!"),
        ];
        let via_two = apply(&apply("hello", &a_ops), &b_ops);
        let via_one = apply("hello", &compose_utf16_deltas(&a_ops, &b_ops));
        assert_eq!(via_one, via_two);
    }
}
