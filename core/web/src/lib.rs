//! wasm-bindgen facade over `monoplan-core`.
//!
//! Surfaces enough of `monoplan-core` for a JS host to (a) round-trip a
//! `Doc` through a storage adapter and (b) drive the sans-IO
//! `SyncEngine` from a browser-owned `WebSocket`, (c) password/recovery derivation.

use wasm_bindgen::prelude::*;

use monoplan_core::{
    AEAD_NONCE_LEN, AppEvent as CoreAppEvent, BootState as CoreBootState, Dek as CoreDek,
    Doc as CoreDoc, DocId as CoreDocId, EngineOptions as CoreEngineOptions, Event as CoreEvent,
    ImportSummary as CoreImportSummary, InFlightPush as CoreInFlightPush,
    ItemLifecycle as CoreItemLifecycle, Kek as CoreKek, LocalSeq as CoreLocalSeq,
    LocalStorage as CoreLocalStorage, NotesDeltaOp, PushId as CorePushId,
    RemoteWalRow as CoreRemoteWalRow, ServerSeq as CoreServerSeq, StorageError as CoreStorageError,
    SyncEngine as CoreSyncEngine, WrappedDek as CoreWrappedDek, derive_password_master,
    derive_recovery_master, generate_recovery_code, kek_from_master, parse_recovery_code,
};
use monoplan_protocol::{EncryptedBlob as CoreEncryptedBlob, KdfParams as CoreKdfParams};

/// Install the panic hook so Rust panics surface as readable JS errors
/// in the console rather than `RuntimeError: unreachable`.
#[wasm_bindgen(start)]
pub fn _start() {
    console_error_panic_hook::set_once();
}

fn parse_notes_delta(delta_json: &str) -> Result<Vec<NotesDeltaOp>, JsError> {
    serde_json::from_str::<Vec<NotesDeltaOp>>(delta_json)
        .map_err(|e| JsError::new(&format!("notes delta: {e}")))
}

fn js_err<E: std::fmt::Display>(e: E) -> JsError {
    JsError::new(&e.to_string())
}

// ---------- lifecycle ----------

/// Resolved item lifecycle (`spec/data-model.md` "Lifecycle"), mirrored
/// from `monoplan_core::ItemLifecycle` for the wasm boundary: the six
/// workflow states (four open, plus the terminal Done and Cancelled)
/// and the orthogonal `Binned` mask. Passed to
/// `setItemLifecycle` / `setItemsLifecycle`; the board's lane-drop
/// primitive.
#[wasm_bindgen]
#[derive(Clone, Copy)]
pub enum ItemLifecycle {
    Backlog,
    Todo,
    InProgress,
    Review,
    Done,
    Cancelled,
    Binned,
}

impl From<ItemLifecycle> for CoreItemLifecycle {
    fn from(l: ItemLifecycle) -> Self {
        match l {
            ItemLifecycle::Backlog => CoreItemLifecycle::Backlog,
            ItemLifecycle::Todo => CoreItemLifecycle::Todo,
            ItemLifecycle::InProgress => CoreItemLifecycle::InProgress,
            ItemLifecycle::Review => CoreItemLifecycle::Review,
            ItemLifecycle::Done => CoreItemLifecycle::Done,
            ItemLifecycle::Cancelled => CoreItemLifecycle::Cancelled,
            ItemLifecycle::Binned => CoreItemLifecycle::Binned,
        }
    }
}

/// Resolve a JS-side lifecycle argument to the open workflow state a
/// direct capture requires (`spec/board.md` "Capture"): Done, Cancelled
/// and Binned are not capture lanes.
fn open_state_arg(l: ItemLifecycle) -> Result<monoplan_core::WorkflowState, JsError> {
    CoreItemLifecycle::from(l)
        .workflow_state()
        .filter(|s| s.is_open())
        .ok_or_else(|| JsError::new("can only capture directly into an open workflow state"))
}

// ---------- Doc ----------

#[wasm_bindgen]
pub struct Doc {
    inner: CoreDoc,
}

#[wasm_bindgen]
impl Doc {
    /// Fresh doc with built-in state initialised.
    #[wasm_bindgen(js_name = create)]
    pub fn create() -> Result<Doc, JsError> {
        Ok(Doc {
            inner: CoreDoc::new().map_err(js_err)?,
        })
    }

    /// As [`create`](Doc::create), but minting under an explicit Loro
    /// peer id (the device's slot peer, `spec/peer-id-plan.md`) instead
    /// of a random one — set before the builtin seed commits, so even
    /// the first ops carry it. The caller must hold the slot's Web Lock
    /// for the life of this doc. `u64::MAX` is reserved by Loro.
    #[wasm_bindgen(js_name = createWithPeer)]
    pub fn create_with_peer(peer: u64) -> Result<Doc, JsError> {
        Ok(Doc {
            inner: CoreDoc::new_with_peer(peer).map_err(js_err)?,
        })
    }

    /// Empty doc — used when bootstrapping a second device that will
    /// receive the seed via the op stream.
    #[wasm_bindgen(js_name = empty)]
    pub fn empty() -> Doc {
        Doc {
            inner: CoreDoc::empty(),
        }
    }

    /// As [`empty`](Doc::empty), but with an explicit Loro peer id —
    /// same contract as [`createWithPeer`](Doc::create_with_peer). The
    /// peer is bound before the replay import so Loro resumes its
    /// counter from the imported history.
    #[wasm_bindgen(js_name = emptyWithPeer)]
    pub fn empty_with_peer(peer: u64) -> Result<Doc, JsError> {
        Ok(Doc {
            inner: CoreDoc::empty_with_peer(peer).map_err(js_err)?,
        })
    }

    /// The Loro peer id local commits are minted under (BigInt in JS).
    #[wasm_bindgen(js_name = peerId)]
    pub fn peer_id(&self) -> u64 {
        self.inner.peer_id()
    }

    /// Decode a doc previously written via [`Doc::save`].
    #[wasm_bindgen(js_name = load)]
    pub fn load(bytes: &[u8]) -> Result<Doc, JsError> {
        Ok(Doc {
            inner: CoreDoc::load(bytes).map_err(js_err)?,
        })
    }

    /// Snapshot + last-pushed VV envelope. Persist this verbatim.
    pub fn save(&self) -> Result<Vec<u8>, JsError> {
        self.inner.save().map_err(js_err)
    }

    /// 32-byte logical-state hash. Stable across replicas at logical
    /// equality. Used in tests to assert convergence.
    pub fn fingerprint(&self) -> Vec<u8> {
        self.inner.fingerprint().to_vec()
    }

    // -- mutations: items --

    #[wasm_bindgen(js_name = addItem)]
    pub fn add_item(&self, list_id: &str, text: &str) -> Result<String, JsError> {
        self.inner.add_item(list_id, text).map_err(js_err)
    }

    #[wasm_bindgen(js_name = addItemAt)]
    pub fn add_item_at(
        &self,
        list_id: &str,
        text: &str,
        target_index: usize,
    ) -> Result<String, JsError> {
        self.inner
            .add_item_at(list_id, text, target_index)
            .map_err(js_err)
    }

    #[wasm_bindgen(js_name = addItemsAt)]
    pub fn add_items_at(
        &self,
        list_id: &str,
        texts: Vec<String>,
        target_index: usize,
    ) -> Result<Vec<String>, JsError> {
        let refs: Vec<&str> = texts.iter().map(|s| s.as_str()).collect();
        self.inner
            .add_items_at(list_id, &refs, target_index)
            .map_err(js_err)
    }

    #[wasm_bindgen(js_name = editItemText)]
    pub fn edit_item_text(&self, item_id: &str, text: &str) -> Result<(), JsError> {
        self.inner.edit_item_text(item_id, text).map_err(js_err)
    }

    /// Start streaming an item's notes as `itemNotesDelta` events; returns
    /// the current plain text for the editor to load. See
    /// `Doc::subscribe_notes`.
    #[wasm_bindgen(js_name = subscribeNotes)]
    pub fn subscribe_notes(&self, item_id: &str) -> Result<String, JsError> {
        self.inner.subscribe_notes(item_id).map_err(js_err)
    }

    #[wasm_bindgen(js_name = unsubscribeNotes)]
    pub fn unsubscribe_notes(&self, item_id: &str) {
        self.inner.unsubscribe_notes(item_id);
    }

    /// Apply an editor delta to an item's notes. `delta_json` is a JSON
    /// array of `{retain}` / `{insert}` / `{delete}` steps in UTF-16
    /// units. One commit with origin `notes:<id>` (excluded from
    /// workspace undo); rejected whole if it does not fit the text.
    #[wasm_bindgen(js_name = applyNotesDelta)]
    pub fn apply_notes_delta(&self, item_id: &str, delta_json: &str) -> Result<(), JsError> {
        let delta = parse_notes_delta(delta_json)?;
        self.inner
            .apply_notes_delta(item_id, &delta)
            .map_err(js_err)
    }

    /// Set (`Some`) or clear (`None`) an item's date-only deadline. The
    /// value must be a `YYYY-MM-DD` calendar date or the call rejects.
    #[wasm_bindgen(js_name = setItemDeadline)]
    pub fn set_item_deadline(
        &self,
        item_id: &str,
        deadline: Option<String>,
    ) -> Result<(), JsError> {
        self.inner
            .set_item_deadline(item_id, deadline.as_deref())
            .map_err(js_err)
    }

    /// Set (`Some`) or clear (`None`) an item's planned date. The value
    /// must be `YYYY-MM-DD` (all-day) or `YYYY-MM-DDTHH:MM` (timed) or
    /// the call rejects.
    #[wasm_bindgen(js_name = setItemWhen)]
    pub fn set_item_when(&self, item_id: &str, when: Option<String>) -> Result<(), JsError> {
        self.inner
            .set_item_when(item_id, when.as_deref())
            .map_err(js_err)
    }

    /// Set (`Some`, whole minutes in `1..=10080`) or clear (`None`) an
    /// item's duration. Only meaningful beside a timed `when`.
    #[wasm_bindgen(js_name = setItemDuration)]
    pub fn set_item_duration(&self, item_id: &str, duration: Option<u32>) -> Result<(), JsError> {
        self.inner
            .set_item_duration(item_id, duration)
            .map_err(js_err)
    }

    #[wasm_bindgen(js_name = moveItem)]
    pub fn move_item(
        &self,
        item_id: &str,
        target_list_id: &str,
        target_index: usize,
    ) -> Result<(), JsError> {
        self.inner
            .move_item(item_id, target_list_id, target_index)
            .map_err(js_err)
    }

    #[wasm_bindgen(js_name = setItemDone)]
    pub fn set_item_done(&self, item_id: &str, done: bool) -> Result<(), JsError> {
        self.inner.set_item_done(item_id, done).map_err(js_err)
    }

    #[wasm_bindgen(js_name = setItemsDone)]
    pub fn set_items_done(&self, item_ids: Vec<String>, done: bool) -> Result<(), JsError> {
        let refs: Vec<&str> = item_ids.iter().map(|s| s.as_str()).collect();
        self.inner.set_items_done(&refs, done).map_err(js_err)
    }

    #[wasm_bindgen(js_name = setItemBinned)]
    pub fn set_item_binned(&self, item_id: &str, binned: bool) -> Result<(), JsError> {
        self.inner.set_item_binned(item_id, binned).map_err(js_err)
    }

    #[wasm_bindgen(js_name = setItemsBinned)]
    pub fn set_items_binned(&self, item_ids: Vec<String>, binned: bool) -> Result<(), JsError> {
        let refs: Vec<&str> = item_ids.iter().map(|s| s.as_str()).collect();
        self.inner.set_items_binned(&refs, binned).map_err(js_err)
    }

    #[wasm_bindgen(js_name = deleteBinned)]
    pub fn delete_binned(&self, item_id: &str) -> Result<(), JsError> {
        self.inner.delete_binned(item_id).map_err(js_err)
    }

    #[wasm_bindgen(js_name = deleteBinnedItems)]
    pub fn delete_binned_items(&self, item_ids: Vec<String>) -> Result<(), JsError> {
        let refs: Vec<&str> = item_ids.iter().map(|s| s.as_str()).collect();
        self.inner.delete_binned_items(&refs).map_err(js_err)
    }

    #[wasm_bindgen(js_name = emptyBin)]
    pub fn empty_bin(&self) -> Result<usize, JsError> {
        self.inner.empty_bin().map_err(js_err)
    }

    /// Explicit stale/duplicate/missing order-entry repair. Returns the
    /// number of repairs (0 = doc was clean, nothing committed).
    pub fn reconcile(&self) -> Result<usize, JsError> {
        self.inner.reconcile().map_err(js_err)
    }

    // -- mutations: lists --

    #[wasm_bindgen(js_name = addList)]
    pub fn add_list(&self, name: &str) -> Result<String, JsError> {
        self.inner.add_list(name).map_err(js_err)
    }

    #[wasm_bindgen(js_name = renameList)]
    pub fn rename_list(&self, list_id: &str, name: &str) -> Result<(), JsError> {
        self.inner.rename_list(list_id, name).map_err(js_err)
    }

    /// Set (`icon` = emoji grapheme) or clear (`icon` = "") a list's
    /// display icon. See `monoplan_core::Doc::set_list_icon`.
    #[wasm_bindgen(js_name = setListIcon)]
    pub fn set_list_icon(&self, list_id: &str, icon: &str) -> Result<(), JsError> {
        self.inner.set_list_icon(list_id, icon).map_err(js_err)
    }

    /// Save (`view` = `"list"` / `"board"` / `"board:<lanes>"`) or clear
    /// (`view` = "") a list's default view. Accepts the reserved `inbox`
    /// too. See `monoplan_core::Doc::set_default_view`.
    #[wasm_bindgen(js_name = setDefaultView)]
    pub fn set_default_view(&self, list_id: &str, view: &str) -> Result<(), JsError> {
        self.inner
            .set_default_view(list_id, parse_default_view_arg(view)?)
            .map_err(js_err)
    }

    #[wasm_bindgen(js_name = moveList)]
    pub fn move_list(&self, list_id: &str, target_index: usize) -> Result<(), JsError> {
        self.inner.move_list(list_id, target_index).map_err(js_err)
    }

    /// Archive (`true`) or unarchive (`false`) a user-created list.
    /// Metadata-only; refuses for `inbox`. See
    /// `monoplan_core::Doc::set_list_archived`.
    #[wasm_bindgen(js_name = setListArchived)]
    pub fn set_list_archived(&self, list_id: &str, archived: bool) -> Result<(), JsError> {
        self.inner
            .set_list_archived(list_id, archived)
            .map_err(js_err)
    }

    /// Internal destructive primitive — not wired to any user-facing UI
    /// (archive is the user-facing removal). Kept for tests and future
    /// permanent-deletion work.
    #[wasm_bindgen(js_name = deleteList)]
    pub fn delete_list(&self, list_id: &str) -> Result<(), JsError> {
        self.inner.delete_list(list_id).map_err(js_err)
    }

    #[wasm_bindgen(js_name = setShowListCounts)]
    pub fn set_show_list_counts(&self, show: bool) -> Result<(), JsError> {
        self.inner.set_show_list_counts(show).map_err(js_err)
    }

    // ---------- lifecycle (spec/board.md, spec/data-model.md) ----------

    /// Move one item to `lifecycle` in a single commit (the board's
    /// lane-drop primitive). Writes the workflow register / bin mask
    /// (plus reflection stamps) per the transition table.
    #[wasm_bindgen(js_name = setItemLifecycle)]
    pub fn set_item_lifecycle(
        &self,
        item_id: &str,
        lifecycle: ItemLifecycle,
    ) -> Result<(), JsError> {
        self.inner
            .set_item_lifecycle(item_id, lifecycle.into())
            .map_err(js_err)
    }

    /// Bulk [`Self::set_item_lifecycle`] — move many items to the same
    /// target lifecycle in one commit.
    #[wasm_bindgen(js_name = setItemsLifecycle)]
    pub fn set_items_lifecycle(
        &self,
        item_ids: Vec<String>,
        lifecycle: ItemLifecycle,
    ) -> Result<(), JsError> {
        let refs: Vec<&str> = item_ids.iter().map(|s| s.as_str()).collect();
        self.inner
            .set_items_lifecycle(&refs, lifecycle.into())
            .map_err(js_err)
    }

    /// Append a new item directly in an open workflow state (board
    /// open-lane capture). `state` must be one of the four open states.
    #[wasm_bindgen(js_name = addItemInState)]
    pub fn add_item_in_state(
        &self,
        list_id: &str,
        text: &str,
        state: ItemLifecycle,
    ) -> Result<String, JsError> {
        self.inner
            .add_item_in_state(list_id, text, open_state_arg(state)?)
            .map_err(js_err)
    }

    /// Insert a new item in an open workflow state at `target_index` in
    /// the list's Open projection (same index space as `addItemAt`), one
    /// commit.
    #[wasm_bindgen(js_name = addItemInStateAt)]
    pub fn add_item_in_state_at(
        &self,
        list_id: &str,
        text: &str,
        state: ItemLifecycle,
        target_index: usize,
    ) -> Result<String, JsError> {
        self.inner
            .add_item_in_state_at(list_id, text, open_state_arg(state)?, target_index)
            .map_err(js_err)
    }

    // ---------- focus (spec/focus.md) ----------

    /// Add a FocusRef to `item_id` at visible position `index`
    /// (`usize::MAX` / a large number appends), one commit. No-op if the
    /// item is already focused or is not Open; errors if unknown. Folds a
    /// dead-ref sweep into the same commit.
    #[wasm_bindgen(js_name = addToFocus)]
    pub fn add_to_focus(&self, item_id: &str, index: usize) -> Result<(), JsError> {
        self.inner.add_to_focus(item_id, index).map_err(js_err)
    }

    /// Batch add: append a FocusRef for each of `item_ids` (in order) to
    /// the Focus lens in one commit. Unknown / not-Open / already-focused
    /// ids are skipped.
    #[wasm_bindgen(js_name = addToFocusMany)]
    pub fn add_to_focus_many(&self, item_ids: Vec<String>) -> Result<(), JsError> {
        let refs: Vec<&str> = item_ids.iter().map(|s| s.as_str()).collect();
        self.inner.add_to_focus_many(&refs).map_err(js_err)
    }

    /// Remove `item_id`'s FocusRef(s) from the Focus lens, one commit.
    /// The item itself is untouched.
    #[wasm_bindgen(js_name = removeFromFocus)]
    pub fn remove_from_focus(&self, item_id: &str) -> Result<(), JsError> {
        self.inner.remove_from_focus(item_id).map_err(js_err)
    }

    /// Batch remove: drop each id's FocusRef(s) from the Focus lens in one
    /// commit. The items are untouched.
    #[wasm_bindgen(js_name = removeFromFocusMany)]
    pub fn remove_from_focus_many(&self, item_ids: Vec<String>) -> Result<(), JsError> {
        let refs: Vec<&str> = item_ids.iter().map(|s| s.as_str()).collect();
        self.inner.remove_from_focus_many(&refs).map_err(js_err)
    }

    /// Reorder `item_id`'s FocusRef to visible position `index` within the
    /// Focus lens, one commit.
    #[wasm_bindgen(js_name = moveInFocus)]
    pub fn move_in_focus(&self, item_id: &str, index: usize) -> Result<(), JsError> {
        self.inner.move_in_focus(item_id, index).map_err(js_err)
    }

    /// Visible Focus item ids in curated order (Open, local, deduped).
    #[wasm_bindgen(js_name = focusRefIds)]
    pub fn focus_ref_ids(&self) -> Vec<String> {
        self.inner.focus_refs()
    }

    /// The Focus lens as a JSON array of `ItemView`s in curated order —
    /// same element shape as `itemsInListJson`.
    #[wasm_bindgen(js_name = focusViewJson)]
    pub fn focus_view_json(&self) -> String {
        items_to_json(&self.inner.focus_view())
    }

    // -- reads (return JSON for now; replace with serde-wasm-bindgen
    //    structured returns once a real consumer needs them) --

    #[wasm_bindgen(js_name = itemsInListJson)]
    pub fn items_in_list_json(&self, list_id: &str, include_binned: bool) -> String {
        let items = self.inner.items_in_list(list_id, include_binned);
        items_to_json(&items)
    }

    #[wasm_bindgen(js_name = binnedItemsJson)]
    pub fn binned_items_json(&self) -> String {
        items_to_json(&self.inner.binned_items())
    }

    #[wasm_bindgen(js_name = allListsJson)]
    pub fn all_lists_json(&self) -> String {
        lists_to_json(&self.inner.all_lists())
    }

    #[wasm_bindgen(js_name = getSettingsJson)]
    pub fn get_settings_json(&self) -> String {
        settings_to_json(&self.inner.get_settings())
    }

    // -- view-id helpers: order-stable id arrays that the JS store
    //    turns into per-view DnD sources --

    /// Ids of `Open` items (the four open workflow states) in `list_id`,
    /// in resolved per-list order (`spec/data-model.md` "Resolved
    /// order"). The board partitions this by each item's register state
    /// into the open lanes.
    #[wasm_bindgen(js_name = openItemIds)]
    pub fn open_item_ids(&self, list_id: &str) -> Vec<String> {
        self.inner.open_item_ids(list_id)
    }

    /// Ids of all closed (`Done` or `Cancelled`), not-binned items,
    /// sorted by the register's `at` descending — the Done view.
    #[wasm_bindgen(js_name = doneItemIds)]
    pub fn done_item_ids(&self) -> Vec<String> {
        self.inner.done_item_ids()
    }

    /// Ids of all `Binned` items, sorted by `binned_at` descending.
    #[wasm_bindgen(js_name = binnedItemIds)]
    pub fn binned_item_ids(&self) -> Vec<String> {
        self.inner.binned_item_ids()
    }

    /// Single-item read by id; returns the JSON shape of `itemsInListJson`'s
    /// elements, or `null` if no such item.
    #[wasm_bindgen(js_name = getItemJson)]
    pub fn get_item_json(&self, item_id: &str) -> Option<String> {
        self.inner.get_item(item_id).map(|i| item_to_json(&i))
    }

    /// Single-list-meta read by id; mirrors `getItemJson` for lists.
    #[wasm_bindgen(js_name = getListMetaJson)]
    pub fn get_list_meta_json(&self, list_id: &str) -> Option<String> {
        self.inner.get_list_meta(list_id).map(|l| list_to_json(&l))
    }

    // -- op stream --

    #[wasm_bindgen(js_name = hasUncapturedOps)]
    pub fn has_uncaptured_ops(&self) -> bool {
        self.inner.has_uncaptured_ops()
    }

    /// Encrypted blob containing every commit since `last_persisted_vv`
    /// (the WAL capture cursor). Returns `null` if there's nothing new.
    #[wasm_bindgen(js_name = pendingExport)]
    pub fn pending_export(&self, dek: &Dek) -> Result<Option<EncryptedBlob>, JsError> {
        Ok(self
            .inner
            .pending_export(&dek.inner)
            .map_err(js_err)?
            .map(|b| EncryptedBlob { inner: b }))
    }

    /// Mark every committed op as durably captured. Call after
    /// `pendingExport`'s blob has been persisted.
    #[wasm_bindgen(js_name = markPersisted)]
    pub fn mark_persisted(&mut self) {
        self.inner.mark_persisted();
    }

    /// Decrypt and apply a peer op blob.
    #[wasm_bindgen(js_name = applyRemote)]
    pub fn apply_remote(&mut self, dek: &Dek, blob: &EncryptedBlob) -> Result<(), JsError> {
        self.inner
            .apply_remote(&dek.inner, &blob.inner)
            .map(|_| ())
            .map_err(js_err)
    }

    // -- oplog primitives --
    //
    // These three methods back the browser local-snapshot + oplog
    // adapter (`spec/local-storage.md`). The JS host:
    //   1. captures `oplogVvBytes()` after each commit,
    //   2. asks for `exportUpdatesAfter(prev_vv)` to get the delta,
    //   3. encrypts + appends it to IndexedDB,
    //   4. on boot, walks the oplog and feeds each plaintext blob back
    //      via `importOplogUpdates`.

    /// Encoded current oplog VersionVector.
    #[wasm_bindgen(js_name = oplogVvBytes)]
    pub fn oplog_vv_bytes(&self) -> Vec<u8> {
        self.inner.oplog_vv_bytes()
    }

    /// Plaintext Loro update bytes for everything committed strictly
    /// after `from_vv`. JS encrypts the result before writing it to
    /// IndexedDB.
    #[wasm_bindgen(js_name = exportUpdatesAfter)]
    pub fn export_updates_after(&self, from_vv: &[u8]) -> Result<Vec<u8>, JsError> {
        self.inner
            .export_updates_after_bytes(from_vv)
            .map_err(js_err)
    }

    /// Plaintext full-state Loro snapshot — for user-driven backup
    /// (`monoplan.bin`). Loro's `Doc::load`-equivalent reconstructs
    /// identical state from this blob.
    #[wasm_bindgen(js_name = exportSnapshot)]
    pub fn export_snapshot(&self) -> Result<Vec<u8>, JsError> {
        self.inner.export_snapshot_bytes().map_err(js_err)
    }

    /// Pretty-printed JSON dump of every list + item — semantic
    /// (human-readable) export, paired with the binary `exportSnapshot`
    /// CRDT-state export.
    #[wasm_bindgen(js_name = exportJson)]
    pub fn export_json(&self) -> String {
        self.inner.export_json_string()
    }

    /// Additive JSON import — counterpart to `exportJson`. Source lists
    /// become fresh local lists; `inbox`-bound items land in the local
    /// `inbox`; existing local content is untouched. Returns a JSON
    /// string of `{ listsAdded, itemsAdded, itemsSkipped, focusAdded }`
    /// for UI summary.
    #[wasm_bindgen(js_name = importJson)]
    pub fn import_json(&self, json: &str) -> Result<String, JsError> {
        let summary = self.inner.import_json_str(json).map_err(js_err)?;
        Ok(summary_to_json(&summary))
    }

    /// Replay one oplog row. Caller has already decrypted; we just feed
    /// it back through the Loro doc tagged so the per-session undo
    /// stack stays clean.
    #[wasm_bindgen(js_name = importOplogUpdates)]
    pub fn import_oplog_updates(&mut self, plaintext: &[u8]) -> Result<(), JsError> {
        self.inner.import_oplog_updates(plaintext).map_err(js_err)
    }

    /// Replay one boot blob without rebuilding the item index. Pair every
    /// sequence of calls with exactly one `finishOplogReplay()`.
    #[wasm_bindgen(js_name = replayOplogUpdate)]
    pub fn replay_oplog_update(&mut self, plaintext: &[u8]) -> Result<(), JsError> {
        self.inner.replay_oplog_update(plaintext).map_err(js_err)
    }

    /// Complete silent local hydration and rebuild disposable indexes once.
    #[wasm_bindgen(js_name = finishOplogReplay)]
    pub fn finish_oplog_replay(&self) {
        self.inner.finish_oplog_replay();
    }

    // -- undo / redo --

    pub fn undo(&self) -> Result<bool, JsError> {
        self.inner.undo().map_err(js_err)
    }

    pub fn redo(&self) -> Result<bool, JsError> {
        self.inner.redo().map_err(js_err)
    }

    #[wasm_bindgen(js_name = canUndo)]
    pub fn can_undo(&self) -> bool {
        self.inner.can_undo()
    }

    #[wasm_bindgen(js_name = canRedo)]
    pub fn can_redo(&self) -> bool {
        self.inner.can_redo()
    }
}

// ---------- Dek ----------

#[wasm_bindgen]
pub struct Dek {
    inner: CoreDek,
}

#[wasm_bindgen]
impl Dek {
    /// Fresh random DEK. Called once at signup.
    pub fn generate() -> Dek {
        Dek {
            inner: CoreDek::generate(),
        }
    }

    #[wasm_bindgen(js_name = fromHex)]
    pub fn from_hex(hex: &str) -> Result<Dek, JsError> {
        let bytes = hex::decode(hex).map_err(js_err)?;
        Ok(Dek {
            inner: CoreDek::from_bytes(&bytes).map_err(js_err)?,
        })
    }

    /// Duplicate the DEK handle. Needed because
    /// `new SyncEngine(doc, dek, ...)` *consumes* the JS handle, but
    /// we also want the DEK around for encrypt-at-rest of the local op log.
    /// Named `dup` (not `clone`) to dodge the `Clone` trait clash on
    /// the wasm wrapper while still being clear about the intent.
    #[wasm_bindgen(js_name = clone)]
    pub fn dup(&self) -> Dek {
        Dek {
            inner: self.inner.clone(),
        }
    }

    #[wasm_bindgen(js_name = toHex)]
    pub fn to_hex(&self) -> String {
        hex::encode(self.inner.as_bytes())
    }

    /// Encrypt an arbitrary byte buffer with this DEK. Used to
    /// encrypt-at-rest the local op log + snapshots in IndexedDB
    /// (`IdbStorage`).
    pub fn seal(&self, plaintext: &[u8]) -> Result<EncryptedBlob, JsError> {
        let (ciphertext, nonce) = self.inner.seal(plaintext).map_err(js_err)?;
        Ok(EncryptedBlob {
            inner: CoreEncryptedBlob {
                ciphertext,
                nonce: nonce.to_vec(),
            },
        })
    }

    /// Decrypt a buffer previously produced by `seal`.
    pub fn open(&self, blob: &EncryptedBlob) -> Result<Vec<u8>, JsError> {
        self.inner
            .open(&blob.inner.ciphertext, &blob.inner.nonce)
            .map_err(js_err)
    }
}

// ---------- Auth derivation ----------

/// Result of `deriveLogin`: KEK (in-memory only — pass to `unwrapDek`)
/// and the auth secret to ship to the server. Both are 32-byte arrays.
#[wasm_bindgen]
pub struct DerivedLogin {
    kek: Vec<u8>,
    auth_secret: Vec<u8>,
}

#[wasm_bindgen]
impl DerivedLogin {
    #[wasm_bindgen(getter)]
    pub fn kek(&self) -> Vec<u8> {
        self.kek.clone()
    }

    #[wasm_bindgen(getter, js_name = authSecret)]
    pub fn auth_secret(&self) -> Vec<u8> {
        self.auth_secret.clone()
    }
}

/// Run Argon2id over `password + salt` and split the master into a
/// `kek` (used to unwrap the DEK locally) and `auth_secret` (sent to
/// the server as the login credential). Hundreds of milliseconds —
/// the caller should show a spinner.
///
/// `m_kib`/`t`/`p` are the KdfParams the server returned from
/// `/api/account/prelogin`.
#[wasm_bindgen(js_name = deriveLogin)]
pub fn derive_login(
    password: &str,
    salt: &[u8],
    m_kib: u32,
    t: u32,
    p: u32,
) -> Result<DerivedLogin, JsError> {
    let params = CoreKdfParams { m_kib, t, p };
    let master = derive_password_master(password.as_bytes(), salt, params).map_err(js_err)?;
    let kek = kek_from_master(&master).map_err(js_err)?;
    let auth = master.auth_secret().map_err(js_err)?;
    Ok(DerivedLogin {
        kek: kek.as_bytes().to_vec(),
        auth_secret: auth.as_bytes().to_vec(),
    })
}

/// `wrapDek` output: ciphertext + 24-byte XChaCha20-Poly1305 nonce.
/// Pair with the local `kek` from `deriveLogin` at signup; ship both
/// to `/api/account/signup` as `wrapped_dek` / `wrapped_dek_nonce`.
#[wasm_bindgen]
pub struct WrappedDekJs {
    ciphertext: Vec<u8>,
    nonce: Vec<u8>,
}

#[wasm_bindgen]
impl WrappedDekJs {
    #[wasm_bindgen(getter)]
    pub fn ciphertext(&self) -> Vec<u8> {
        self.ciphertext.clone()
    }

    #[wasm_bindgen(getter)]
    pub fn nonce(&self) -> Vec<u8> {
        self.nonce.clone()
    }
}

/// Encrypt a DEK with a KEK. Used at signup and on password change /
/// reset. Browser WebCrypto can't do XChaCha20-Poly1305 so this lives
/// in wasm alongside the rest of the e2ee primitives.
#[wasm_bindgen(js_name = wrapDek)]
pub fn wrap_dek(kek_bytes: &[u8], dek: &Dek) -> Result<WrappedDekJs, JsError> {
    let kek = CoreKek::from_bytes(kek_bytes).map_err(js_err)?;
    let w = kek.wrap(&dek.inner).map_err(js_err)?;
    Ok(WrappedDekJs {
        ciphertext: w.ciphertext,
        nonce: w.nonce.to_vec(),
    })
}

/// Decrypt the DEK shipped from `/api/account/login` using the local
/// `kek` from `deriveLogin`. Wrong password → throws.
#[wasm_bindgen(js_name = unwrapDek)]
pub fn unwrap_dek(
    kek_bytes: &[u8],
    wrapped_ciphertext: &[u8],
    wrapped_nonce: &[u8],
) -> Result<Dek, JsError> {
    let kek = CoreKek::from_bytes(kek_bytes).map_err(js_err)?;
    if wrapped_nonce.len() != AEAD_NONCE_LEN {
        return Err(JsError::new(&format!(
            "wrapped_dek_nonce: expected {AEAD_NONCE_LEN} bytes, got {}",
            wrapped_nonce.len()
        )));
    }
    let mut nonce = [0u8; AEAD_NONCE_LEN];
    nonce.copy_from_slice(wrapped_nonce);
    let wrapped = CoreWrappedDek {
        ciphertext: wrapped_ciphertext.to_vec(),
        nonce,
    };
    let dek = kek.unwrap(&wrapped).map_err(js_err)?;
    Ok(Dek { inner: dek })
}

// ---------- Recovery code ----------

/// Result of `deriveRecovery`: recovery KEK (in-memory only — pass to
/// `unwrapDek` to open the server-held recovery wrap) and the recovery
/// auth secret to send to `/api/account/recover`. Both are 32-byte
/// arrays, mirroring `DerivedLogin`.
#[wasm_bindgen]
pub struct DerivedRecovery {
    recovery_kek: Vec<u8>,
    recovery_auth_secret: Vec<u8>,
}

#[wasm_bindgen]
impl DerivedRecovery {
    #[wasm_bindgen(getter, js_name = recoveryKek)]
    pub fn recovery_kek(&self) -> Vec<u8> {
        self.recovery_kek.clone()
    }

    #[wasm_bindgen(getter, js_name = recoveryAuthSecret)]
    pub fn recovery_auth_secret(&self) -> Vec<u8> {
        self.recovery_auth_secret.clone()
    }
}

/// Fresh 12-word BIP39 phrase. Show once at signup; never persist on
/// the client (the user's the only one who keeps it).
#[wasm_bindgen(js_name = generateRecoveryCode)]
pub fn generate_recovery_code_js() -> Result<String, JsError> {
    Ok(generate_recovery_code()
        .map_err(js_err)?
        .as_str()
        .to_string())
}

/// Validate + normalize a user-typed recovery phrase. Tolerates extra
/// whitespace and case. Throws on invalid words or bad checksum — the
/// UI should surface that as "recovery code not recognized."
#[wasm_bindgen(js_name = parseRecoveryCode)]
pub fn parse_recovery_code_js(input: &str) -> Result<String, JsError> {
    Ok(parse_recovery_code(input)
        .map_err(js_err)?
        .as_str()
        .to_string())
}

/// Argon2id over the recovery phrase + `recovery_salt`, then HKDF into
/// the recovery KEK and recovery auth secret. Mirrors `deriveLogin`'s
/// shape so the host can shovel both into `wrapDek`/`unwrapDek` and the
/// recovery endpoint without thinking about the split.
#[wasm_bindgen(js_name = deriveRecovery)]
pub fn derive_recovery_js(
    recovery_code: &str,
    salt: &[u8],
    m_kib: u32,
    t: u32,
    p: u32,
) -> Result<DerivedRecovery, JsError> {
    let params = CoreKdfParams { m_kib, t, p };
    let master = derive_recovery_master(recovery_code, salt, params).map_err(js_err)?;
    let kek = master.kek().map_err(js_err)?;
    let auth = master.auth_secret().map_err(js_err)?;
    Ok(DerivedRecovery {
        recovery_kek: kek.as_bytes().to_vec(),
        recovery_auth_secret: auth.as_bytes().to_vec(),
    })
}

// ---------- EncryptedBlob ----------

#[wasm_bindgen]
pub struct EncryptedBlob {
    inner: CoreEncryptedBlob,
}

#[wasm_bindgen]
impl EncryptedBlob {
    #[wasm_bindgen(constructor)]
    pub fn new(nonce: Vec<u8>, ciphertext: Vec<u8>) -> EncryptedBlob {
        EncryptedBlob {
            inner: CoreEncryptedBlob { nonce, ciphertext },
        }
    }

    #[wasm_bindgen(getter)]
    pub fn nonce(&self) -> Vec<u8> {
        self.inner.nonce.clone()
    }

    #[wasm_bindgen(getter)]
    pub fn ciphertext(&self) -> Vec<u8> {
        self.inner.ciphertext.clone()
    }
}

// ---------- EngineStorage (JS-implemented LocalStorage) ----------
//
// The engine's `LocalStorage` trait is synchronous, but IndexedDB is
// async. The JS side bridges the gap with an in-memory mirror that
// answers these methods immediately and flushes IDB in the
// background (see `js/core/src/storage/idb-storage.ts`). These extern
// methods are therefore plain synchronous JS calls.
//
// Marshalling conventions across the boundary:
//   - encrypted payloads cross as `(ciphertext, nonce)` byte pairs;
//   - `localSeq` / `serverSeq` cross as JS `number` (`f64`) — both fit
//     in 2^53 for any realistic WAL, so we skip the `bigint` dance;
//   - `pushId` crosses as the raw 16 UUID bytes;
//   - encoded VersionVectors cross as opaque byte arrays;
//   - `appendRemoteWal` returns `{ localSeq, inserted }`, read back via
//     `js_sys` reflection.
//
// `boot()` is intentionally absent — on web the host reconstructs the
// `Doc` in JS directly from IDB before constructing the engine (and
// seeds the engine's cursors via the `seed*` methods), so the engine
// never calls `storage.boot()` itself (verified in `core/src/sync.rs`).
// `WebStorage::boot` returns an empty `BootState` to satisfy the trait.
#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(typescript_type = "EngineStorage")]
    pub type EngineStorage;

    #[wasm_bindgen(method, catch, js_name = appendLocalWal)]
    fn append_local_wal(
        this: &EngineStorage,
        ciphertext: &[u8],
        nonce: &[u8],
    ) -> Result<f64, JsValue>;

    #[wasm_bindgen(method, catch, js_name = appendRemoteWal)]
    fn append_remote_wal(
        this: &EngineStorage,
        server_seq: f64,
        ciphertext: &[u8],
        nonce: &[u8],
        server_known_vv: &[u8],
    ) -> Result<JsValue, JsValue>;

    #[wasm_bindgen(method, catch, js_name = writeSnapshot)]
    fn write_snapshot(
        this: &EngineStorage,
        cutoff: f64,
        ciphertext: &[u8],
        nonce: &[u8],
    ) -> Result<(), JsValue>;

    #[wasm_bindgen(method, catch, js_name = writeAckedSeq)]
    fn write_acked_seq(this: &EngineStorage, seq: f64) -> Result<(), JsValue>;

    #[wasm_bindgen(method, catch, js_name = writeServerKnownVv)]
    fn write_server_known_vv(this: &EngineStorage, vv: &[u8]) -> Result<(), JsValue>;

    #[wasm_bindgen(method, catch, js_name = putInFlightPush)]
    fn put_in_flight_push(
        this: &EngineStorage,
        push_id: &[u8],
        ciphertext: &[u8],
        nonce: &[u8],
        from_vv: &[u8],
        to_vv: &[u8],
    ) -> Result<(), JsValue>;

    #[wasm_bindgen(method, catch, js_name = completePush)]
    fn complete_push(
        this: &EngineStorage,
        push_id: &[u8],
        server_known_vv: &[u8],
    ) -> Result<(), JsValue>;
}

#[wasm_bindgen(typescript_custom_section)]
const TS_ENGINE_STORAGE: &'static str = r#"
export interface EngineStorage {
  appendLocalWal(ciphertext: Uint8Array, nonce: Uint8Array): number;
  appendRemoteWal(serverSeq: number, ciphertext: Uint8Array, nonce: Uint8Array, serverKnownVv: Uint8Array): { localSeq: number; inserted: boolean };
  writeSnapshot(cutoff: number, ciphertext: Uint8Array, nonce: Uint8Array): void;
  writeAckedSeq(seq: number): void;
  writeServerKnownVv(vv: Uint8Array): void;
  putInFlightPush(pushId: Uint8Array, ciphertext: Uint8Array, nonce: Uint8Array, fromVv: Uint8Array, toVv: Uint8Array): void;
  completePush(pushId: Uint8Array, serverKnownVv: Uint8Array): void;
}
"#;

/// Rust `LocalStorage` over a JS-implemented `EngineStorage`. Every
/// call forwards synchronously to the JS in-memory mirror; durability
/// (the background IDB flush) is the JS side's concern and is surfaced
/// to the engine out-of-band via `notifyOplogDurable`.
struct WebStorage {
    js: EngineStorage,
}

fn jsval_err(e: JsValue) -> CoreStorageError {
    CoreStorageError::Backend(
        e.as_string()
            .or_else(|| js_sys::Error::from(e).message().as_string())
            .unwrap_or_else(|| "JS exception".into()),
    )
}

fn reflect_f64(obj: &JsValue, key: &str) -> Result<f64, CoreStorageError> {
    js_sys::Reflect::get(obj, &JsValue::from_str(key))
        .map_err(jsval_err)?
        .as_f64()
        .ok_or_else(|| CoreStorageError::Backend(format!("appendRemoteWal: {key} not a number")))
}

fn reflect_bool(obj: &JsValue, key: &str) -> Result<bool, CoreStorageError> {
    js_sys::Reflect::get(obj, &JsValue::from_str(key))
        .map_err(jsval_err)?
        .as_bool()
        .ok_or_else(|| CoreStorageError::Backend(format!("appendRemoteWal: {key} not a boolean")))
}

impl CoreLocalStorage for WebStorage {
    fn boot(&self, _doc_id: CoreDocId) -> Result<CoreBootState, CoreStorageError> {
        // Never reached: the web host rebuilds the `Doc` from IDB in JS
        // before constructing the engine, and the engine itself never
        // calls `boot()`.
        Ok(CoreBootState::default())
    }

    fn append_local_wal(
        &self,
        _doc_id: CoreDocId,
        payload: CoreEncryptedBlob,
    ) -> Result<CoreLocalSeq, CoreStorageError> {
        let seq = self
            .js
            .append_local_wal(&payload.ciphertext, &payload.nonce)
            .map_err(jsval_err)?;
        Ok(CoreLocalSeq(seq as u64))
    }

    fn append_remote_wal(
        &self,
        _doc_id: CoreDocId,
        row: CoreRemoteWalRow,
        server_known_vv: &[u8],
    ) -> Result<(CoreLocalSeq, bool), CoreStorageError> {
        let result = self
            .js
            .append_remote_wal(
                row.server_seq.0 as f64,
                &row.payload.ciphertext,
                &row.payload.nonce,
                server_known_vv,
            )
            .map_err(jsval_err)?;
        let local_seq = reflect_f64(&result, "localSeq")? as u64;
        let inserted = reflect_bool(&result, "inserted")?;
        Ok((CoreLocalSeq(local_seq), inserted))
    }

    fn write_snapshot(
        &self,
        _doc_id: CoreDocId,
        cutoff: CoreLocalSeq,
        payload: CoreEncryptedBlob,
    ) -> Result<(), CoreStorageError> {
        self.js
            .write_snapshot(cutoff.0 as f64, &payload.ciphertext, &payload.nonce)
            .map_err(jsval_err)
    }

    fn write_acked_seq(
        &self,
        _doc_id: CoreDocId,
        seq: CoreServerSeq,
    ) -> Result<(), CoreStorageError> {
        self.js.write_acked_seq(seq.0 as f64).map_err(jsval_err)
    }

    fn write_server_known_vv(&self, _doc_id: CoreDocId, vv: &[u8]) -> Result<(), CoreStorageError> {
        self.js.write_server_known_vv(vv).map_err(jsval_err)
    }

    fn put_in_flight_push(
        &self,
        _doc_id: CoreDocId,
        push: CoreInFlightPush,
    ) -> Result<(), CoreStorageError> {
        self.js
            .put_in_flight_push(
                push.push_id.0.as_bytes(),
                &push.payload.ciphertext,
                &push.payload.nonce,
                &push.from_vv,
                &push.to_vv,
            )
            .map_err(jsval_err)
    }

    fn complete_push(
        &self,
        _doc_id: CoreDocId,
        push_id: CorePushId,
        server_known_vv: &[u8],
    ) -> Result<(), CoreStorageError> {
        self.js
            .complete_push(push_id.0.as_bytes(), server_known_vv)
            .map_err(jsval_err)
    }
}

// ---------- SyncEngine ----------

#[wasm_bindgen]
pub struct SyncEngine {
    inner: CoreSyncEngine,
}

#[wasm_bindgen]
impl SyncEngine {
    /// Build a new engine. Consumes `doc` and `dek` — the JS handles
    /// must not be reused after this call. `doc_id` is the server-
    /// assigned UUID for this doc (the value JS already has in
    /// `session.primaryDocId`). `last_acked_seq` is the contiguous-
    /// prefix seq persisted from the previous session (or 0 for a
    /// fresh device).
    #[wasm_bindgen(constructor)]
    pub fn new(
        doc: Doc,
        doc_id: String,
        dek: Dek,
        last_acked_seq: u64,
        client_name: String,
        client_version: String,
        storage: EngineStorage,
    ) -> Result<SyncEngine, JsError> {
        let parsed = uuid::Uuid::parse_str(&doc_id)
            .map_err(|e| JsError::new(&format!("invalid doc_id: {e}")))?;
        // The host always passes a JS `EngineStorage` (the `IdbStorage`
        // mirror): capture, ack, and remote-apply all flow through it.
        let dyn_storage: Box<dyn CoreLocalStorage> = Box::new(WebStorage { js: storage });
        Ok(SyncEngine {
            inner: CoreSyncEngine::new(
                doc.inner,
                CoreDocId(parsed),
                dek.inner,
                last_acked_seq,
                CoreEngineOptions {
                    client_name,
                    client_version,
                },
                dyn_storage,
            ),
        })
    }

    /// Persist any locally-committed mutations as a durable WAL row
    /// (and advance the capture cursor). Returns the assigned `localSeq`
    /// or `undefined` when nothing was pending. Call **before**
    /// `flush()` so pushed bytes are always disk-bound first.
    #[wasm_bindgen(js_name = captureLocalOps)]
    pub fn capture_local_ops(&mut self) -> Result<Option<f64>, JsError> {
        Ok(self
            .inner
            .capture_local_ops()
            .map_err(js_err)?
            .map(|s| s.0 as f64))
    }

    /// Fold the WAL into a fresh full-history snapshot when it has at
    /// least `max_rows` rows or more than `max_bytes` payload bytes.
    /// Runs regardless of sync state — unsent local ops survive
    /// folding (the push derives them from `server_known_vv`).
    /// Snapshot export is O(doc) — pass a real row threshold on hot
    /// pulses (per-ack) and `1` from idle hooks. Returns whether a
    /// snapshot was written.
    #[wasm_bindgen(js_name = snapshotIfWalExceeds)]
    pub fn snapshot_if_wal_exceeds(
        &mut self,
        max_rows: u32,
        max_bytes: f64,
    ) -> Result<bool, JsError> {
        self.inner
            .snapshot_if_wal_exceeds(u64::from(max_rows), max_bytes as u64)
            .map_err(js_err)
    }

    /// Unconditionally fold any non-empty WAL into a snapshot — the
    /// exit/hide hook. `false` when the WAL was already empty.
    #[wasm_bindgen(js_name = forceSnapshot)]
    pub fn force_snapshot(&mut self) -> Result<bool, JsError> {
        self.inner.force_snapshot().map_err(js_err)
    }

    /// Seed the highest `localSeq` the storage has assigned for this
    /// doc (from the JS boot). Call once right after construction.
    #[wasm_bindgen(js_name = setLastLocalSeq)]
    pub fn set_last_local_seq(&mut self, seq: f64) {
        self.inner.set_last_local_seq(CoreLocalSeq(seq as u64));
    }

    /// Seed the WAL statistics (surviving rows / payload bytes past
    /// the last snapshot) from the JS boot.
    #[wasm_bindgen(js_name = seedWalStats)]
    pub fn seed_wal_stats(&mut self, rows: f64, bytes: f64) {
        self.inner.seed_wal_stats(rows as u64, bytes as u64);
    }

    /// Seed `server_known_vv` from its persisted encoding (empty array
    /// = never synced). Call once right after construction.
    #[wasm_bindgen(js_name = seedServerKnownVv)]
    pub fn seed_server_known_vv(&mut self, encoded: &[u8]) {
        self.inner.seed_server_known_vv(encoded);
    }

    /// Seed the durable in-flight push from a previous session; the
    /// reconnect flow re-sends it (same `pushId`) after the initial
    /// pull and the server deduplicates.
    #[wasm_bindgen(js_name = seedInFlightPush)]
    pub fn seed_in_flight_push(
        &mut self,
        push_id: &[u8],
        ciphertext: &[u8],
        nonce: &[u8],
        from_vv: &[u8],
        to_vv: &[u8],
    ) -> Result<(), JsError> {
        let push_id = uuid::Uuid::from_slice(push_id).map_err(js_err)?;
        self.inner.seed_in_flight_push(CoreInFlightPush {
            push_id: CorePushId(push_id),
            payload: CoreEncryptedBlob {
                nonce: nonce.to_vec(),
                ciphertext: ciphertext.to_vec(),
            },
            from_vv: from_vv.to_vec(),
            to_vv: to_vv.to_vec(),
        });
        Ok(())
    }

    // -- transport callbacks --

    /// Caller's WebSocket has opened. Engine queues the `Hello` frame.
    #[wasm_bindgen(js_name = handleConnected)]
    pub fn handle_connected(&mut self) {
        self.inner.handle_connected();
    }

    /// Caller's WebSocket dropped. Engine returns to `Disconnected` and
    /// clears the outbox; reconnect re-derives any pending frames.
    #[wasm_bindgen(js_name = handleDisconnected)]
    pub fn handle_disconnected(&mut self) {
        self.inner.handle_disconnected();
    }

    /// Caller-driven tick: escalates the `Hello` handshake watchdog
    /// (no-op outside Hello). Hosts call this periodically (e.g.,
    /// every ~1s via `setInterval`).
    #[wasm_bindgen(js_name = handleTimeout)]
    pub fn handle_timeout(&mut self) {
        self.inner.handle_timeout();
    }

    /// Hand one binary WebSocket frame to the engine.
    #[wasm_bindgen(js_name = handleServerBytes)]
    pub fn handle_server_bytes(&mut self, bytes: &[u8]) {
        self.inner.handle_server_bytes(bytes);
    }

    // -- caller drives --

    /// Signal that the user just committed local mutations. Triggers a
    /// push when idle; queues the intent otherwise.
    pub fn flush(&mut self) {
        self.inner.flush();
    }

    /// Drain the next frame to ship to the server. Returns `undefined`
    /// when the outbox is empty.
    #[wasm_bindgen(js_name = popOutbox)]
    pub fn pop_outbox(&mut self) -> Option<Vec<u8>> {
        self.inner.pop_outbox()
    }

    /// Drain the next engine event. Returns `undefined` when the queue
    /// is empty.
    #[wasm_bindgen(js_name = popEvent)]
    pub fn pop_event(&mut self) -> Option<EngineEvent> {
        self.inner.pop_event().map(EngineEvent::from)
    }

    /// Drain the next domain-level change event. Pair with `popEvent`
    /// — that one carries protocol/connection events; this one carries
    /// item / list lifecycle deltas the UI store mirrors.
    #[wasm_bindgen(js_name = popAppEvent)]
    pub fn pop_app_event(&self) -> Option<AppEventJs> {
        self.inner.pop_app_event().map(AppEventJs::from)
    }

    /// Synthetic event burst describing current doc state — `ListAdded`
    /// for every list, then `ItemAdded` for every item. Consumers feed
    /// this through the same dispatcher used for live deltas, so a
    /// fresh attach is "current state, then live changes" without a
    /// separate "load initial" path.
    #[wasm_bindgen(js_name = snapshotEvents)]
    pub fn snapshot_events(&self) -> Vec<AppEventJs> {
        self.inner
            .doc()
            .snapshot_events()
            .into_iter()
            .map(AppEventJs::from)
            .collect()
    }

    /// Compact one-shot workspace materialization for initial attach and
    /// `fullResync`. One JSON string crosses the wasm boundary instead of
    /// thousands of heap-allocated `AppEventJs` wrappers and cloned getters.
    #[wasm_bindgen(js_name = workspaceSnapshotJson)]
    pub fn workspace_snapshot_json(&self) -> String {
        workspace_snapshot_json(self.inner.doc())
    }

    // -- introspection --

    #[wasm_bindgen(js_name = isOnline)]
    pub fn is_online(&self) -> bool {
        self.inner.is_online()
    }

    #[wasm_bindgen(js_name = isIdle)]
    pub fn is_idle(&self) -> bool {
        self.inner.is_idle()
    }

    /// Contiguous-prefix seq the engine has applied **in memory**.
    /// Use for transport-layer decisions; persist `lastDurableSeq()`
    /// as the resume cursor instead so a crash never resumes from a
    /// seq the local oplog doesn't cover.
    #[wasm_bindgen(js_name = lastContiguousSeq)]
    pub fn last_contiguous_seq(&self) -> u64 {
        self.inner.last_contiguous_seq()
    }

    /// Contiguous-prefix seq the host has confirmed locally durable.
    /// Equals the highest `last_acked_seq` the engine has shipped (or
    /// will ship) in `Ack` frames. Persist this between sessions.
    #[wasm_bindgen(js_name = lastDurableSeq)]
    pub fn last_durable_seq(&self) -> u64 {
        self.inner.last_durable_seq()
    }

    /// Host signal: bytes covering up to `seq` are now durable in
    /// local storage (encrypted oplog row committed). Advances the
    /// durable frontier and queues an `Ack` frame if it overtakes the
    /// last one shipped. Caller must `popOutbox()` afterwards to
    /// drain the queued frame onto the wire.
    #[wasm_bindgen(js_name = notifyOplogDurable)]
    pub fn notify_oplog_durable(&mut self, seq: u64) {
        self.inner.notify_oplog_durable(seq);
    }

    // -- doc passthrough --
    //
    // Engine owns the `Doc`, but JS still needs to read it (render UI)
    // and mutate it (user actions). Loro mutations take `&self` thanks
    // to interior mutability, so the wrapper can share the engine's
    // borrow without `&mut self` ceremony.

    /// Snapshot envelope to persist (loro snapshot + last-pushed VV).
    pub fn save(&self) -> Result<Vec<u8>, JsError> {
        self.inner.doc().save().map_err(js_err)
    }

    /// 32-byte logical-state hash. Stable across replicas.
    pub fn fingerprint(&self) -> Vec<u8> {
        self.inner.doc().fingerprint().to_vec()
    }

    #[wasm_bindgen(js_name = hasUncapturedOps)]
    pub fn has_uncaptured_ops(&self) -> bool {
        self.inner.doc().has_uncaptured_ops()
    }

    /// True when local operations lack server proof (commits beyond
    /// `server_known_vv`, or an outstanding in-flight push) — the UI's
    /// "pending changes" indicator.
    #[wasm_bindgen(js_name = hasUnsyncedOps)]
    pub fn has_unsynced_ops(&self) -> bool {
        self.inner.has_unsynced_ops()
    }

    // -- oplog passthrough --
    //
    // After each local mutation the browser host captures the oplog
    // VV, asks for the delta since the previous capture, and pushes
    // that into the IndexedDB oplog. Replay (`importOplogUpdates`) runs
    // on the bare `Doc` before the engine is constructed, so it lives
    // on `Doc` only.

    #[wasm_bindgen(js_name = oplogVvBytes)]
    pub fn oplog_vv_bytes(&self) -> Vec<u8> {
        self.inner.doc().oplog_vv_bytes()
    }

    #[wasm_bindgen(js_name = exportUpdatesAfter)]
    pub fn export_updates_after(&self, from_vv: &[u8]) -> Result<Vec<u8>, JsError> {
        self.inner
            .doc()
            .export_updates_after_bytes(from_vv)
            .map_err(js_err)
    }

    /// Plaintext full-state Loro snapshot — a lossless, round-trippable
    /// backup. Currently unexposed in the UI (there's no matching import
    /// path yet); kept for the eventual lossless restore. Side-effect-free;
    /// doesn't touch `last_pushed_vv` or the oplog frontier.
    #[wasm_bindgen(js_name = exportSnapshot)]
    pub fn export_snapshot(&self) -> Result<Vec<u8>, JsError> {
        self.inner.doc().export_snapshot_bytes().map_err(js_err)
    }

    /// Pretty-printed JSON dump — powers the "Export JSON" menu item.
    #[wasm_bindgen(js_name = exportJson)]
    pub fn export_json(&self) -> String {
        self.inner.doc().export_json_string()
    }

    /// Additive JSON import — counterpart of `exportJson` on the engine
    /// surface. Returns a JSON string of `{ listsAdded, itemsAdded,
    /// itemsSkipped, focusAdded }`. Doesn't push or flush — caller's normal
    /// post-mutation flow (oplog append, sync push) handles the new ops
    /// like any other local commit.
    #[wasm_bindgen(js_name = importJson)]
    pub fn import_json(&self, json: &str) -> Result<String, JsError> {
        let summary = self.inner.doc().import_json_str(json).map_err(js_err)?;
        Ok(summary_to_json(&summary))
    }

    // -- mutations: items --

    #[wasm_bindgen(js_name = addItem)]
    pub fn add_item(&self, list_id: &str, text: &str) -> Result<String, JsError> {
        self.inner.doc().add_item(list_id, text).map_err(js_err)
    }

    #[wasm_bindgen(js_name = addItemAt)]
    pub fn add_item_at(
        &self,
        list_id: &str,
        text: &str,
        target_index: usize,
    ) -> Result<String, JsError> {
        self.inner
            .doc()
            .add_item_at(list_id, text, target_index)
            .map_err(js_err)
    }

    #[wasm_bindgen(js_name = addItemsAt)]
    pub fn add_items_at(
        &self,
        list_id: &str,
        texts: Vec<String>,
        target_index: usize,
    ) -> Result<Vec<String>, JsError> {
        let refs: Vec<&str> = texts.iter().map(|s| s.as_str()).collect();
        self.inner
            .doc()
            .add_items_at(list_id, &refs, target_index)
            .map_err(js_err)
    }

    #[wasm_bindgen(js_name = editItemText)]
    pub fn edit_item_text(&self, item_id: &str, text: &str) -> Result<(), JsError> {
        self.inner
            .doc()
            .edit_item_text(item_id, text)
            .map_err(js_err)
    }

    /// Start streaming an item's notes as `itemNotesDelta` events; returns
    /// the current plain text for the editor to load. See
    /// `Doc::subscribe_notes`.
    #[wasm_bindgen(js_name = subscribeNotes)]
    pub fn subscribe_notes(&self, item_id: &str) -> Result<String, JsError> {
        self.inner.doc().subscribe_notes(item_id).map_err(js_err)
    }

    #[wasm_bindgen(js_name = unsubscribeNotes)]
    pub fn unsubscribe_notes(&self, item_id: &str) {
        self.inner.doc().unsubscribe_notes(item_id);
    }

    /// Apply an editor delta to an item's notes. `delta_json` is a JSON
    /// array of `{retain}` / `{insert}` / `{delete}` steps in UTF-16
    /// units. One commit with origin `notes:<id>` (excluded from
    /// workspace undo); rejected whole if it does not fit the text.
    /// Does not capture or push: the host coalesces a typing burst into
    /// one flush (`spec/notes-plan.md` "Commit policy").
    #[wasm_bindgen(js_name = applyNotesDelta)]
    pub fn apply_notes_delta(&self, item_id: &str, delta_json: &str) -> Result<(), JsError> {
        let delta = parse_notes_delta(delta_json)?;
        self.inner
            .doc()
            .apply_notes_delta(item_id, &delta)
            .map_err(js_err)
    }

    /// Set (`Some`) or clear (`None`) an item's date-only deadline. The
    /// value must be a `YYYY-MM-DD` calendar date or the call rejects.
    #[wasm_bindgen(js_name = setItemDeadline)]
    pub fn set_item_deadline(
        &self,
        item_id: &str,
        deadline: Option<String>,
    ) -> Result<(), JsError> {
        self.inner
            .doc()
            .set_item_deadline(item_id, deadline.as_deref())
            .map_err(js_err)
    }

    /// Set (`Some`) or clear (`None`) an item's planned date. The value
    /// must be `YYYY-MM-DD` (all-day) or `YYYY-MM-DDTHH:MM` (timed) or
    /// the call rejects.
    #[wasm_bindgen(js_name = setItemWhen)]
    pub fn set_item_when(&self, item_id: &str, when: Option<String>) -> Result<(), JsError> {
        self.inner
            .doc()
            .set_item_when(item_id, when.as_deref())
            .map_err(js_err)
    }

    /// Set (`Some`, whole minutes in `1..=10080`) or clear (`None`) an
    /// item's duration. Only meaningful beside a timed `when`.
    #[wasm_bindgen(js_name = setItemDuration)]
    pub fn set_item_duration(&self, item_id: &str, duration: Option<u32>) -> Result<(), JsError> {
        self.inner
            .doc()
            .set_item_duration(item_id, duration)
            .map_err(js_err)
    }

    #[wasm_bindgen(js_name = moveItem)]
    pub fn move_item(
        &self,
        item_id: &str,
        target_list_id: &str,
        target_index: usize,
    ) -> Result<(), JsError> {
        self.inner
            .doc()
            .move_item(item_id, target_list_id, target_index)
            .map_err(js_err)
    }

    #[wasm_bindgen(js_name = setItemDone)]
    pub fn set_item_done(&self, item_id: &str, done: bool) -> Result<(), JsError> {
        self.inner
            .doc()
            .set_item_done(item_id, done)
            .map_err(js_err)
    }

    #[wasm_bindgen(js_name = setItemsDone)]
    pub fn set_items_done(&self, item_ids: Vec<String>, done: bool) -> Result<(), JsError> {
        let refs: Vec<&str> = item_ids.iter().map(|s| s.as_str()).collect();
        self.inner.doc().set_items_done(&refs, done).map_err(js_err)
    }

    #[wasm_bindgen(js_name = setItemBinned)]
    pub fn set_item_binned(&self, item_id: &str, binned: bool) -> Result<(), JsError> {
        self.inner
            .doc()
            .set_item_binned(item_id, binned)
            .map_err(js_err)
    }

    #[wasm_bindgen(js_name = setItemsBinned)]
    pub fn set_items_binned(&self, item_ids: Vec<String>, binned: bool) -> Result<(), JsError> {
        let refs: Vec<&str> = item_ids.iter().map(|s| s.as_str()).collect();
        self.inner
            .doc()
            .set_items_binned(&refs, binned)
            .map_err(js_err)
    }

    #[wasm_bindgen(js_name = deleteBinned)]
    pub fn delete_binned(&self, item_id: &str) -> Result<(), JsError> {
        self.inner.doc().delete_binned(item_id).map_err(js_err)
    }

    #[wasm_bindgen(js_name = deleteBinnedItems)]
    pub fn delete_binned_items(&self, item_ids: Vec<String>) -> Result<(), JsError> {
        let refs: Vec<&str> = item_ids.iter().map(|s| s.as_str()).collect();
        self.inner.doc().delete_binned_items(&refs).map_err(js_err)
    }

    #[wasm_bindgen(js_name = emptyBin)]
    pub fn empty_bin(&self) -> Result<usize, JsError> {
        self.inner.doc().empty_bin().map_err(js_err)
    }

    /// Explicit stale/duplicate/missing order-entry repair. Returns the
    /// number of repairs (0 = doc was clean, nothing committed).
    pub fn reconcile(&self) -> Result<usize, JsError> {
        self.inner.doc().reconcile().map_err(js_err)
    }

    // -- mutations: lists --

    #[wasm_bindgen(js_name = addList)]
    pub fn add_list(&self, name: &str) -> Result<String, JsError> {
        self.inner.doc().add_list(name).map_err(js_err)
    }

    #[wasm_bindgen(js_name = renameList)]
    pub fn rename_list(&self, list_id: &str, name: &str) -> Result<(), JsError> {
        self.inner.doc().rename_list(list_id, name).map_err(js_err)
    }

    /// Set (`icon` = emoji grapheme) or clear (`icon` = "") a list's
    /// display icon. See `monoplan_core::Doc::set_list_icon`.
    #[wasm_bindgen(js_name = setListIcon)]
    pub fn set_list_icon(&self, list_id: &str, icon: &str) -> Result<(), JsError> {
        self.inner
            .doc()
            .set_list_icon(list_id, icon)
            .map_err(js_err)
    }

    /// Save (`view` = `"list"` / `"board"` / `"board:<lanes>"`) or clear
    /// (`view` = "") a list's default view. Accepts the reserved `inbox`
    /// too. See `monoplan_core::Doc::set_default_view`.
    #[wasm_bindgen(js_name = setDefaultView)]
    pub fn set_default_view(&self, list_id: &str, view: &str) -> Result<(), JsError> {
        self.inner
            .doc()
            .set_default_view(list_id, parse_default_view_arg(view)?)
            .map_err(js_err)
    }

    #[wasm_bindgen(js_name = moveList)]
    pub fn move_list(&self, list_id: &str, target_index: usize) -> Result<(), JsError> {
        self.inner
            .doc()
            .move_list(list_id, target_index)
            .map_err(js_err)
    }

    /// Archive (`true`) or unarchive (`false`) a user-created list.
    /// Metadata-only; refuses for `inbox`. See
    /// `monoplan_core::Doc::set_list_archived`.
    #[wasm_bindgen(js_name = setListArchived)]
    pub fn set_list_archived(&self, list_id: &str, archived: bool) -> Result<(), JsError> {
        self.inner
            .doc()
            .set_list_archived(list_id, archived)
            .map_err(js_err)
    }

    /// Internal destructive primitive — not wired to any user-facing UI
    /// (archive is the user-facing removal). Kept for tests and future
    /// permanent-deletion work.
    #[wasm_bindgen(js_name = deleteList)]
    pub fn delete_list(&self, list_id: &str) -> Result<(), JsError> {
        self.inner.doc().delete_list(list_id).map_err(js_err)
    }

    #[wasm_bindgen(js_name = setShowListCounts)]
    pub fn set_show_list_counts(&self, show: bool) -> Result<(), JsError> {
        self.inner.doc().set_show_list_counts(show).map_err(js_err)
    }

    // ---------- lifecycle (spec/board.md, spec/data-model.md) ----------

    /// Move one item to `lifecycle` in a single commit (the board's
    /// lane-drop primitive).
    #[wasm_bindgen(js_name = setItemLifecycle)]
    pub fn set_item_lifecycle(
        &self,
        item_id: &str,
        lifecycle: ItemLifecycle,
    ) -> Result<(), JsError> {
        self.inner
            .doc()
            .set_item_lifecycle(item_id, lifecycle.into())
            .map_err(js_err)
    }

    /// Bulk [`Self::set_item_lifecycle`] — move many items to the same
    /// target lifecycle in one commit.
    #[wasm_bindgen(js_name = setItemsLifecycle)]
    pub fn set_items_lifecycle(
        &self,
        item_ids: Vec<String>,
        lifecycle: ItemLifecycle,
    ) -> Result<(), JsError> {
        let refs: Vec<&str> = item_ids.iter().map(|s| s.as_str()).collect();
        self.inner
            .doc()
            .set_items_lifecycle(&refs, lifecycle.into())
            .map_err(js_err)
    }

    /// Append a new item directly in an open workflow state (board
    /// open-lane capture). `state` must be one of the four open states.
    #[wasm_bindgen(js_name = addItemInState)]
    pub fn add_item_in_state(
        &self,
        list_id: &str,
        text: &str,
        state: ItemLifecycle,
    ) -> Result<String, JsError> {
        self.inner
            .doc()
            .add_item_in_state(list_id, text, open_state_arg(state)?)
            .map_err(js_err)
    }

    /// Insert a new item in an open workflow state at `target_index` in
    /// the list's Open projection (same index space as `addItemAt`), one
    /// commit.
    #[wasm_bindgen(js_name = addItemInStateAt)]
    pub fn add_item_in_state_at(
        &self,
        list_id: &str,
        text: &str,
        state: ItemLifecycle,
        target_index: usize,
    ) -> Result<String, JsError> {
        self.inner
            .doc()
            .add_item_in_state_at(list_id, text, open_state_arg(state)?, target_index)
            .map_err(js_err)
    }

    // ---------- focus (spec/focus.md) ----------

    /// Add a FocusRef to `item_id` at visible position `index`
    /// (`usize::MAX` / a large number appends), one commit. No-op if the
    /// item is already focused or is not Open; errors if unknown. Folds a
    /// dead-ref sweep into the same commit.
    #[wasm_bindgen(js_name = addToFocus)]
    pub fn add_to_focus(&self, item_id: &str, index: usize) -> Result<(), JsError> {
        self.inner
            .doc()
            .add_to_focus(item_id, index)
            .map_err(js_err)
    }

    /// Batch add: append a FocusRef for each of `item_ids` (in order) to
    /// the Focus lens in one commit. Unknown / not-Open / already-focused
    /// ids are skipped. Backs multi-select "add to focus".
    #[wasm_bindgen(js_name = addToFocusMany)]
    pub fn add_to_focus_many(&self, item_ids: Vec<String>) -> Result<(), JsError> {
        let refs: Vec<&str> = item_ids.iter().map(|s| s.as_str()).collect();
        self.inner.doc().add_to_focus_many(&refs).map_err(js_err)
    }

    /// Remove `item_id`'s FocusRef(s) from the Focus lens, one commit.
    /// The item itself is untouched.
    #[wasm_bindgen(js_name = removeFromFocus)]
    pub fn remove_from_focus(&self, item_id: &str) -> Result<(), JsError> {
        self.inner.doc().remove_from_focus(item_id).map_err(js_err)
    }

    /// Batch remove: drop each id's FocusRef(s) from the Focus lens in one
    /// commit. The items are untouched.
    #[wasm_bindgen(js_name = removeFromFocusMany)]
    pub fn remove_from_focus_many(&self, item_ids: Vec<String>) -> Result<(), JsError> {
        let refs: Vec<&str> = item_ids.iter().map(|s| s.as_str()).collect();
        self.inner
            .doc()
            .remove_from_focus_many(&refs)
            .map_err(js_err)
    }

    /// Reorder `item_id`'s FocusRef to visible position `index` within the
    /// Focus lens, one commit.
    #[wasm_bindgen(js_name = moveInFocus)]
    pub fn move_in_focus(&self, item_id: &str, index: usize) -> Result<(), JsError> {
        self.inner
            .doc()
            .move_in_focus(item_id, index)
            .map_err(js_err)
    }

    /// Visible Focus item ids in curated order (Open, local, deduped).
    #[wasm_bindgen(js_name = focusRefIds)]
    pub fn focus_ref_ids(&self) -> Vec<String> {
        self.inner.doc().focus_refs()
    }

    /// The Focus lens as a JSON array of `ItemView`s in curated order —
    /// same element shape as `itemsInListJson`.
    #[wasm_bindgen(js_name = focusViewJson)]
    pub fn focus_view_json(&self) -> String {
        items_to_json(&self.inner.doc().focus_view())
    }

    // -- undo / redo --

    pub fn undo(&self) -> Result<bool, JsError> {
        self.inner.doc().undo().map_err(js_err)
    }

    pub fn redo(&self) -> Result<bool, JsError> {
        self.inner.doc().redo().map_err(js_err)
    }

    #[wasm_bindgen(js_name = canUndo)]
    pub fn can_undo(&self) -> bool {
        self.inner.doc().can_undo()
    }

    #[wasm_bindgen(js_name = canRedo)]
    pub fn can_redo(&self) -> bool {
        self.inner.doc().can_redo()
    }

    // -- reads --

    #[wasm_bindgen(js_name = itemsInListJson)]
    pub fn items_in_list_json(&self, list_id: &str, include_binned: bool) -> String {
        items_to_json(&self.inner.doc().items_in_list(list_id, include_binned))
    }

    #[wasm_bindgen(js_name = binnedItemsJson)]
    pub fn binned_items_json(&self) -> String {
        items_to_json(&self.inner.doc().binned_items())
    }

    #[wasm_bindgen(js_name = allListsJson)]
    pub fn all_lists_json(&self) -> String {
        lists_to_json(&self.inner.doc().all_lists())
    }

    #[wasm_bindgen(js_name = getSettingsJson)]
    pub fn get_settings_json(&self) -> String {
        settings_to_json(&self.inner.doc().get_settings())
    }

    #[wasm_bindgen(js_name = openItemIds)]
    pub fn open_item_ids(&self, list_id: &str) -> Vec<String> {
        self.inner.doc().open_item_ids(list_id)
    }

    #[wasm_bindgen(js_name = doneItemIds)]
    pub fn done_item_ids(&self) -> Vec<String> {
        self.inner.doc().done_item_ids()
    }

    #[wasm_bindgen(js_name = binnedItemIds)]
    pub fn binned_item_ids(&self) -> Vec<String> {
        self.inner.doc().binned_item_ids()
    }

    #[wasm_bindgen(js_name = getItemJson)]
    pub fn get_item_json(&self, item_id: &str) -> Option<String> {
        self.inner.doc().get_item(item_id).map(|i| item_to_json(&i))
    }

    #[wasm_bindgen(js_name = getListMetaJson)]
    pub fn get_list_meta_json(&self, list_id: &str) -> Option<String> {
        self.inner
            .doc()
            .get_list_meta(list_id)
            .map(|l| list_to_json(&l))
    }
}

// ---------- EngineEvent ----------

/// Flat JS-friendly view of `monoplan_core::Event`. The host switches on
/// `kind` and reads the payload-specific getter — exactly one of
/// `online`, `seq`, `message` will be set per event (or none, for
/// payload-less variants).
#[wasm_bindgen]
pub struct EngineEvent {
    kind: &'static str,
    online: Option<bool>,
    seq: Option<u64>,
    message: Option<String>,
}

#[wasm_bindgen]
impl EngineEvent {
    #[wasm_bindgen(getter)]
    pub fn kind(&self) -> String {
        self.kind.to_string()
    }

    #[wasm_bindgen(getter)]
    pub fn online(&self) -> Option<bool> {
        self.online
    }

    #[wasm_bindgen(getter)]
    pub fn seq(&self) -> Option<u64> {
        self.seq
    }

    #[wasm_bindgen(getter)]
    pub fn message(&self) -> Option<String> {
        self.message.clone()
    }
}

impl From<CoreEvent> for EngineEvent {
    fn from(e: CoreEvent) -> Self {
        match e {
            CoreEvent::ConnStateChanged { online } => EngineEvent {
                kind: "connStateChanged",
                online: Some(online),
                seq: None,
                message: None,
            },
            CoreEvent::PulledInitial => EngineEvent {
                kind: "pulledInitial",
                online: None,
                seq: None,
                message: None,
            },
            CoreEvent::Pushed => EngineEvent {
                kind: "pushed",
                online: None,
                seq: None,
                message: None,
            },
            CoreEvent::FrontierAdvanced { seq } => EngineEvent {
                kind: "frontierAdvanced",
                online: None,
                seq: Some(seq),
                message: None,
            },
            CoreEvent::Error(message) => EngineEvent {
                kind: "error",
                online: None,
                seq: None,
                message: Some(message),
            },
        }
    }
}

// ---------- AppEventJs ----------

/// Flat JS-friendly view of `monoplan_core::AppEvent`. The host switches
/// on `kind` and reads only the fields documented for that variant —
/// every other getter returns `undefined`.
///
/// Variant → fields:
/// - `fullResync` — no fields; rematerialize current state once
/// - `itemAdded` — id, listId, text, notes, createdAt, state, lifecycleAt, startedAt?, doneAt?, binnedAt?, deadline?, when?, duration?, openIndex?
/// - `itemRemoved` — id
/// - `itemMoved` — id, openIndex?
/// - `itemTextChanged` — id, text
/// - `itemNotesChanged` — id, notes
/// - `itemNotesDelta` — id, delta (JSON array of `{retain}` / `{insert}` / `{delete}`, UTF-16 units; only for items with a `subscribeNotes` subscription)
/// - `itemDeadlineChanged` — id, deadline? (undefined = no deadline)
/// - `itemWhenChanged` — id, when? (undefined = unset; `YYYY-MM-DD` or `YYYY-MM-DDTHH:MM`)
/// - `itemDurationChanged` — id, duration? (undefined = unset; whole minutes)
/// - `itemLifecycleChanged` — id, state, lifecycleAt, startedAt?, doneAt?, binnedAt?, openIndex?
/// - `itemListChanged` — id, listId, openIndex?
/// - `listAdded` — id, name, createdAt, archivedAt?, index
/// - `listRemoved` — id
/// - `listMoved` — id, index
/// - `listRenamed` — id, name
/// - `listIconChanged` — id, icon? (undefined = icon removed)
/// - `listDefaultViewChanged` — id, defaultView? (undefined = cleared)
/// - `listArchivedChanged` — id, archivedAt? (undefined = unarchived)
/// - `settingsChanged` — showListCounts, inboxView?
/// - `focusChanged` — no fields; re-derive the Focus projection
#[wasm_bindgen]
pub struct AppEventJs {
    kind: &'static str,
    id: String,
    list_id: Option<String>,
    text: Option<String>,
    notes: Option<String>,
    /// `itemNotesDelta` only: the delta as JSON text.
    delta: Option<String>,
    name: Option<String>,
    /// List display icon (`listIconChanged`): `Some(grapheme)` when set,
    /// `None` when the icon was removed or on events that don't carry one.
    icon: Option<String>,
    /// Saved default view in encoded form (`listDefaultViewChanged`):
    /// `"list"` / `"board"` / `"board:<lanes>"`, or `None` when the
    /// default was cleared or the event doesn't carry one.
    default_view: Option<String>,
    /// Workflow register state name (`itemAdded` / `itemLifecycleChanged`):
    /// `"backlog" | "todo" | "in_progress" | "review" | "done" |
    /// "cancelled"`, masked by `binned_at` when that is set. `None` on
    /// events that don't carry lifecycle.
    state: Option<&'static str>,
    /// The workflow register's transition timestamp (`itemAdded` /
    /// `itemLifecycleChanged`).
    lifecycle_at: Option<i64>,
    /// Reflection stamp: first entry into In Progress, if any.
    started_at: Option<i64>,
    /// Date-only deadline `YYYY-MM-DD` (`itemAdded` / `itemDeadlineChanged`);
    /// `None` means no deadline.
    deadline: Option<String>,
    /// Planned date (`itemAdded` / `itemWhenChanged`); `None` means unset.
    when: Option<String>,
    /// Duration in whole minutes (`itemAdded` / `itemDurationChanged`);
    /// `None` means unset.
    duration: Option<u32>,
    created_at: Option<i64>,
    /// Reflection stamp: last entry into Done, if any.
    done_at: Option<i64>,
    binned_at: Option<i64>,
    /// List archive timestamp (`listAdded` / `listArchivedChanged`):
    /// `Some(ts)` when the list is archived, `None` when active or on
    /// events that don't carry archive state.
    archived_at: Option<i64>,
    show_list_counts: Option<bool>,
    /// Settings (`settingsChanged`): Inbox's saved default view in
    /// encoded form, or `None` when none is saved.
    inbox_view: Option<String>,
    /// List-event ordering position (`listAdded` / `listMoved`). Item
    /// events no longer carry a doc-wide index in the v2 schema — use
    /// `open_index`.
    index: Option<usize>,
    /// Position within the owning list's *Open* projection (the four
    /// open workflow states; Done/binned excluded). Present on item
    /// events whenever the item is open after the change; `undefined`
    /// otherwise. See `monoplan_core::AppEvent` for per-variant semantics.
    open_index: Option<usize>,
}

#[wasm_bindgen]
impl AppEventJs {
    #[wasm_bindgen(getter)]
    pub fn kind(&self) -> String {
        self.kind.to_string()
    }
    #[wasm_bindgen(getter)]
    pub fn id(&self) -> String {
        self.id.clone()
    }
    #[wasm_bindgen(getter, js_name = listId)]
    pub fn list_id(&self) -> Option<String> {
        self.list_id.clone()
    }
    #[wasm_bindgen(getter)]
    pub fn text(&self) -> Option<String> {
        self.text.clone()
    }
    #[wasm_bindgen(getter)]
    pub fn notes(&self) -> Option<String> {
        self.notes.clone()
    }
    #[wasm_bindgen(getter)]
    pub fn delta(&self) -> Option<String> {
        self.delta.clone()
    }
    #[wasm_bindgen(getter)]
    pub fn name(&self) -> Option<String> {
        self.name.clone()
    }
    #[wasm_bindgen(getter)]
    pub fn icon(&self) -> Option<String> {
        self.icon.clone()
    }
    #[wasm_bindgen(getter, js_name = createdAt)]
    pub fn created_at(&self) -> Option<i64> {
        self.created_at
    }
    #[wasm_bindgen(getter, js_name = doneAt)]
    pub fn done_at(&self) -> Option<i64> {
        self.done_at
    }
    #[wasm_bindgen(getter, js_name = binnedAt)]
    pub fn binned_at(&self) -> Option<i64> {
        self.binned_at
    }
    #[wasm_bindgen(getter, js_name = archivedAt)]
    pub fn archived_at(&self) -> Option<i64> {
        self.archived_at
    }
    #[wasm_bindgen(getter)]
    pub fn index(&self) -> Option<usize> {
        self.index
    }
    #[wasm_bindgen(getter, js_name = openIndex)]
    pub fn open_index(&self) -> Option<usize> {
        self.open_index
    }
    #[wasm_bindgen(getter, js_name = showListCounts)]
    pub fn show_list_counts(&self) -> Option<bool> {
        self.show_list_counts
    }
    #[wasm_bindgen(getter, js_name = defaultView)]
    pub fn default_view(&self) -> Option<String> {
        self.default_view.clone()
    }
    #[wasm_bindgen(getter, js_name = inboxView)]
    pub fn inbox_view(&self) -> Option<String> {
        self.inbox_view.clone()
    }
    #[wasm_bindgen(getter)]
    pub fn state(&self) -> Option<String> {
        self.state.map(str::to_string)
    }
    #[wasm_bindgen(getter, js_name = lifecycleAt)]
    pub fn lifecycle_at(&self) -> Option<i64> {
        self.lifecycle_at
    }
    #[wasm_bindgen(getter, js_name = startedAt)]
    pub fn started_at(&self) -> Option<i64> {
        self.started_at
    }
    #[wasm_bindgen(getter, js_name = deadline)]
    pub fn deadline(&self) -> Option<String> {
        self.deadline.clone()
    }
    #[wasm_bindgen(getter, js_name = when)]
    pub fn when(&self) -> Option<String> {
        self.when.clone()
    }
    #[wasm_bindgen(getter, js_name = duration)]
    pub fn duration(&self) -> Option<u32> {
        self.duration
    }
}

impl From<CoreAppEvent> for AppEventJs {
    fn from(e: CoreAppEvent) -> Self {
        let blank = AppEventJs {
            kind: "",
            id: String::new(),
            list_id: None,
            text: None,
            notes: None,
            delta: None,
            name: None,
            icon: None,
            default_view: None,
            state: None,
            lifecycle_at: None,
            started_at: None,
            deadline: None,
            when: None,
            duration: None,
            created_at: None,
            done_at: None,
            binned_at: None,
            archived_at: None,
            show_list_counts: None,
            inbox_view: None,
            index: None,
            open_index: None,
        };
        match e {
            CoreAppEvent::FullResync => AppEventJs {
                kind: "fullResync",
                ..blank
            },
            CoreAppEvent::FocusChanged => AppEventJs {
                kind: "focusChanged",
                ..blank
            },
            CoreAppEvent::ItemAdded {
                id,
                list_id,
                text,
                notes,
                created_at,
                state,
                lifecycle_at,
                started_at,
                done_at,
                binned_at,
                deadline,
                when,
                duration,
                open_index,
            } => AppEventJs {
                kind: "itemAdded",
                id,
                list_id: Some(list_id),
                text: Some(text),
                notes: Some(notes),
                created_at: Some(created_at),
                state: Some(state.name()),
                lifecycle_at: Some(lifecycle_at),
                started_at,
                done_at,
                binned_at,
                deadline,
                when,
                duration,
                open_index,
                ..blank
            },
            CoreAppEvent::ItemRemoved { id } => AppEventJs {
                kind: "itemRemoved",
                id,
                ..blank
            },
            CoreAppEvent::ItemMoved { id, open_index } => AppEventJs {
                kind: "itemMoved",
                id,
                open_index,
                ..blank
            },
            CoreAppEvent::ItemTextChanged { id, text } => AppEventJs {
                kind: "itemTextChanged",
                id,
                text: Some(text),
                ..blank
            },
            CoreAppEvent::ItemNotesChanged { id, notes } => AppEventJs {
                kind: "itemNotesChanged",
                id,
                notes: Some(notes),
                ..blank
            },
            CoreAppEvent::ItemNotesDelta { id, delta } => AppEventJs {
                kind: "itemNotesDelta",
                id,
                delta: Some(serde_json::to_string(&delta).unwrap_or_else(|_| "[]".into())),
                ..blank
            },
            CoreAppEvent::ItemDeadlineChanged { id, deadline } => AppEventJs {
                kind: "itemDeadlineChanged",
                id,
                deadline,
                ..blank
            },
            CoreAppEvent::ItemWhenChanged { id, when } => AppEventJs {
                kind: "itemWhenChanged",
                id,
                when,
                ..blank
            },
            CoreAppEvent::ItemDurationChanged { id, duration } => AppEventJs {
                kind: "itemDurationChanged",
                id,
                duration,
                ..blank
            },
            CoreAppEvent::ItemLifecycleChanged {
                id,
                state,
                lifecycle_at,
                started_at,
                done_at,
                binned_at,
                open_index,
            } => AppEventJs {
                kind: "itemLifecycleChanged",
                id,
                state: Some(state.name()),
                lifecycle_at: Some(lifecycle_at),
                started_at,
                done_at,
                binned_at,
                open_index,
                ..blank
            },
            CoreAppEvent::ItemListChanged {
                id,
                list_id,
                open_index,
            } => AppEventJs {
                kind: "itemListChanged",
                id,
                list_id: Some(list_id),
                open_index,
                ..blank
            },
            CoreAppEvent::ListAdded {
                id,
                name,
                created_at,
                archived_at,
                index,
            } => AppEventJs {
                kind: "listAdded",
                id,
                name: Some(name),
                created_at: Some(created_at),
                archived_at,
                index: Some(index),
                ..blank
            },
            CoreAppEvent::ListRemoved { id } => AppEventJs {
                kind: "listRemoved",
                id,
                ..blank
            },
            CoreAppEvent::ListMoved { id, index } => AppEventJs {
                kind: "listMoved",
                id,
                index: Some(index),
                ..blank
            },
            CoreAppEvent::ListRenamed { id, name } => AppEventJs {
                kind: "listRenamed",
                id,
                name: Some(name),
                ..blank
            },
            CoreAppEvent::ListIconChanged { id, icon } => AppEventJs {
                kind: "listIconChanged",
                id,
                icon,
                ..blank
            },
            CoreAppEvent::ListDefaultViewChanged { id, view } => AppEventJs {
                kind: "listDefaultViewChanged",
                id,
                default_view: view.map(|v| v.encode()),
                ..blank
            },
            CoreAppEvent::ListArchivedChanged { id, archived_at } => AppEventJs {
                kind: "listArchivedChanged",
                id,
                archived_at,
                ..blank
            },
            CoreAppEvent::SettingsChanged {
                show_list_counts,
                inbox_view,
            } => AppEventJs {
                kind: "settingsChanged",
                show_list_counts: Some(show_list_counts),
                inbox_view: inbox_view.map(|v| v.encode()),
                ..blank
            },
        }
    }
}

// ---------- private helpers ----------

/// Decode the JS-side `view` argument of `setDefaultView`: `""` clears
/// the saved default, anything else must be a known encoded
/// [`DefaultView`]. An unknown string is an error rather than a silent
/// clear, so a caller typo surfaces instead of wiping the default.
fn parse_default_view_arg(view: &str) -> Result<Option<monoplan_core::DefaultView>, JsError> {
    if view.is_empty() {
        return Ok(None);
    }
    monoplan_core::DefaultView::parse(view)
        .map(Some)
        .ok_or_else(|| JsError::new(&format!("unknown default view: {view}")))
}

fn list_to_json(l: &monoplan_core::ListView) -> String {
    let mut s = format!(
        "{{\"id\":{},\"name\":{}",
        json_string(&l.id),
        json_string(&l.name),
    );
    if let Some(icon) = &l.icon {
        s.push_str(",\"icon\":");
        s.push_str(&json_string(icon));
    }
    if let Some(v) = &l.default_view {
        s.push_str(",\"defaultView\":");
        s.push_str(&json_string(&v.encode()));
    }
    if let Some(t) = l.archived_at {
        s.push_str(&format!(",\"archivedAt\":{t}"));
    }
    s.push_str(&format!(",\"createdAt\":{}}}", l.created_at));
    s
}

fn settings_to_json(s: &monoplan_core::SettingsView) -> String {
    let mut out = format!("{{\"showListCounts\":{}", s.show_list_counts);
    if let Some(v) = &s.inbox_view {
        out.push_str(",\"inboxView\":");
        out.push_str(&json_string(&v.encode()));
    }
    out.push('}');
    out
}

fn lists_to_json(lists: &[monoplan_core::ListView]) -> String {
    let mut s = String::from("[");
    for (i, l) in lists.iter().enumerate() {
        if i > 0 {
            s.push(',');
        }
        s.push_str(&list_to_json(l));
    }
    s.push(']');
    s
}

fn item_to_json(it: &monoplan_core::ItemView) -> String {
    let mut s = format!(
        "{{\"id\":{},\"text\":{},\"notes\":{},\"listId\":{},\"createdAt\":{},\"state\":{},\"lifecycleAt\":{}",
        json_string(&it.id),
        json_string(&it.text),
        json_string(&it.notes),
        json_string(&it.list_id),
        it.created_at,
        json_string(it.state.name()),
        it.lifecycle_at,
    );
    if let Some(t) = it.started_at {
        s.push_str(&format!(",\"startedAt\":{t}"));
    }
    if let Some(t) = it.done_at {
        s.push_str(&format!(",\"doneAt\":{t}"));
    }
    if let Some(t) = it.binned_at {
        s.push_str(&format!(",\"binnedAt\":{t}"));
    }
    if let Some(d) = &it.deadline {
        s.push_str(",\"deadline\":");
        s.push_str(&json_string(d));
    }
    if let Some(w) = &it.when {
        s.push_str(",\"when\":");
        s.push_str(&json_string(w));
    }
    if let Some(n) = it.duration {
        s.push_str(&format!(",\"duration\":{n}"));
    }
    s.push('}');
    s
}

fn items_to_json(items: &[monoplan_core::ItemView]) -> String {
    let mut s = String::from("[");
    for (i, it) in items.iter().enumerate() {
        if i > 0 {
            s.push(',');
        }
        s.push_str(&item_to_json(it));
    }
    s.push(']');
    s
}

fn workspace_snapshot_json(doc: &CoreDoc) -> String {
    let settings = doc.get_settings();
    let lists = doc.all_lists();
    let items = doc.all_items();
    let mut out = String::from("{\"settings\":");
    out.push_str(&settings_to_json(&settings));
    out.push_str(",\"lists\":[");
    for (i, list) in lists.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        out.push_str(&list_to_json(list));
    }
    out.push_str("],\"items\":[");
    for (i, item) in items.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        out.push_str(&item_to_json(item));
    }
    out.push_str("]}");
    out
}

fn summary_to_json(s: &CoreImportSummary) -> String {
    format!(
        "{{\"listsAdded\":{},\"itemsAdded\":{},\"itemsSkipped\":{},\"focusAdded\":{}}}",
        s.lists_added, s.items_added, s.items_skipped, s.focus_added,
    )
}

fn json_string(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => {
                out.push_str(&format!("\\u{:04x}", c as u32));
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}
