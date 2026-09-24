// Reactive shell around the wasm `SyncEngine`. Doc state lives in a
// SolidJS `createStore` keyed by id; mutations go through the engine,
// which emits domain-level `AppEvent`s that this layer mirrors into the
// store via surgical `setState` calls. The store is the single source
// of truth the UI reads from — `state.listOpen[listId]` for a list
// view's iteration order, `state.itemsById[id]` for content. Solid's
// proxy tracks each property path independently, so a peer editing one
// item doesn't invalidate the iteration and vice versa.
//
// There is deliberately no global item order: each mutation touches
// only the affected list's Open array (splice at the event's
// `openIndex`) plus maintained counters, so dispatch cost scales with
// the touched list, never with total items in the doc. Done/Bin views
// derive lazily from `itemsById` (timestamp sorts), not CRDT order.
// See spec/list-perf-plan.md.

import type { AppEventJs, SyncEngine } from "@monoplan/core/wasm";
import { ItemLifecycle } from "@monoplan/core/wasm";
import { diffToDelta, type NotesDeltaOp } from "../notesDelta.ts";
import { batch, createSignal, type Accessor } from "solid-js";
import { createStore, produce, reconcile } from "solid-js/store";
import { createSearchEngine, type SearchEngine } from "../search.ts";

/** Workflow register state (`spec/data-model.md` "Lifecycle"): the
 *  five-step ladder held by the atomic `[state, at]` register. Bin is
 *  not a state — it is the orthogonal `binnedAt` mask. */
export type WorkflowState =
  | "backlog"
  | "todo"
  | "in_progress"
  | "review"
  | "done";

/** The four open states, in board lane order. */
export const OPEN_STATES: readonly WorkflowState[] = [
  "backlog",
  "todo",
  "in_progress",
  "review",
];

export interface ItemView {
  id: string;
  text: string;
  notes: string;
  listId: string;
  /** Workflow register state, masked by `binnedAt` when that is set.
   *  The board partitions a list's Open items by this into lanes. */
  state: WorkflowState;
  /** The register's transition timestamp (unix millis) — the Done view's
   *  sort key. Equals `createdAt` while the register is unwritten. */
  lifecycleAt: number;
  /** Optional date-only deadline as a raw `YYYY-MM-DD` string (floating
   *  local calendar date — never parse it with `new Date("YYYY-MM-DD")`,
   *  which reads as UTC midnight). Absent means no deadline. */
  deadline?: string;
  /** Optional planned date: raw `YYYY-MM-DD` (all-day) or
   *  `YYYY-MM-DDTHH:MM` (timed), floating (`spec/calendar-plan.md`).
   *  Same never-`new Date()` rule as `deadline`. Absent means unset. */
  when?: string;
  /** Optional duration in whole minutes. Only meaningful beside a timed
   *  `when`; views ignore it otherwise. Absent means unset. */
  duration?: number;
  createdAt: number;
  /** Reflection stamp: first entry into In Progress, if any. */
  startedAt?: number;
  /** Reflection stamp: last entry into Done, if any. View sorts use
   *  `lifecycleAt`, not this. */
  doneAt?: number;
  binnedAt?: number;
}

/** Resolved lifecycle (`spec/data-model.md`): the workflow state, or
 *  `binned` while the mask is present. */
export type Lifecycle = WorkflowState | "binned";

/** Map the JS lifecycle string onto the wasm `ItemLifecycle` enum the
 *  engine's `setItemLifecycle` / `setItemsLifecycle` expect. */
function lifecycleEnum(l: Lifecycle): ItemLifecycle {
  switch (l) {
    case "backlog":
      return ItemLifecycle.Backlog;
    case "todo":
      return ItemLifecycle.Todo;
    case "in_progress":
      return ItemLifecycle.InProgress;
    case "review":
      return ItemLifecycle.Review;
    case "done":
      return ItemLifecycle.Done;
    case "binned":
      return ItemLifecycle.Binned;
  }
}

/** Normalize a wire state string onto the known ladder; anything a newer
 *  client wrote degrades to `backlog` (visible and open), mirroring the
 *  core's unparseable-register fallback. */
export function parseWorkflowState(s: string | undefined): WorkflowState {
  switch (s) {
    case "todo":
    case "in_progress":
    case "review":
    case "done":
      return s;
    default:
      return "backlog";
  }
}

/** Workflow register says Done (regardless of the bin mask). */
export const isDone = (it: ItemView): boolean => it.state === "done";
export const isBinned = (it: ItemView): boolean => it.binnedAt != null;
/** Open (one of the four open workflow states, not binned) — the
 *  per-list view. */
export const isOpen = (it: ItemView): boolean =>
  !isBinned(it) && it.state !== "done";
/** Resolved lifecycle: `binned` while the mask is present, else the
 *  workflow register's state. */
export const lifecycleOf = (it: ItemView): Lifecycle =>
  isBinned(it) ? "binned" : it.state;

export interface ListView {
  id: string;
  name: string;
  /** User-chosen display icon (a literal emoji grapheme), or absent when
   *  unset — consumers render a built-in fallback glyph. */
  icon?: string;
  /** Saved default view in encoded form (`"list"` / `"board"` /
   *  `"board:<lanes>"` — see `view.ts`), or absent when the user
   *  has never saved one. Synced doc state; a client with a local
   *  override of its own ignores it. See `spec/board.md`. */
  defaultView?: string;
  /** Archive timestamp (`spec/data-model.md` "Archived lists"): absent ≡
   *  active, present ≡ archived. Archived lists stay in `listsOrder` /
   *  `listsById` (name resolution, search, archived-list views); the UI
   *  derives its active/archived projections from this field. */
  archivedAt?: number;
  createdAt: number;
}

export interface WorkspaceState {
  itemsById: Record<string, ItemView>;
  /** Per-list Open order (the four open workflow states) — mirrors the
   *  core's `open` projection of the list's `order/<list-id>` container.
   *  Done/binned items never appear here; a list with no open items may
   *  be absent or hold `[]`. The board partitions this array by each
   *  item's register state into the open lanes. */
  listOpen: Record<string, string[]>;
  /** Binned-item count, maintained incrementally so the Bin badge
   *  never needs a global scan. */
  binCount: number;
  /** Visible Focus refs in curated order (`engine.focusRefIds()` — Open,
   *  local, deduped). The Focus lens iterates this; each id resolves to
   *  `itemsById`. Re-derived wholesale per non-empty event drain, since a
   *  focus mutation *or* an item lifecycle change elsewhere can alter
   *  visibility. See spec/focus.md. */
  focusOrder: string[];
  listsOrder: string[];
  listsById: Record<string, ListView>;
  settings: SettingsView;
}

/** Snapshot of where an item sat in its list's live order at the
 *  moment it was marked done. Feeds the list-view "linger" affordance
 *  (`Workspace`), which briefly re-inserts recently-done rows at their
 *  old position — the live projection itself drops them instantly. */
export interface RecentDoneEntry {
  id: string;
  listId: string;
  /** Open index the item occupied just before leaving the projection. */
  index: number;
  /** Visible Focus slot the item occupied, when it was in the Focus lens
   *  (Done auto-removes the ref, so this is the only record). */
  focusIndex?: number;
  /** The Done transition's register timestamp (`lifecycleAt`). */
  doneAt: number;
}

export interface SettingsView {
  /** When true, the nav renders the live-item count beside each
   *  non-Inbox list (subject to the count > 0 gate). Inbox's count is
   *  always shown regardless. Single global flag; default false. Synced
   *  via the doc-level settings map. */
  showListCounts: boolean;
  /** The reserved `inbox` list's saved default view in encoded form, or
   *  `null` when none is saved. Inbox has no ListMeta row, so its
   *  default lives in the doc-level settings map rather than on
   *  `ListView.defaultView`. */
  inboxView: string | null;
}

export interface DocApp {
  engine: SyncEngine;
  state: WorkspaceState;
  /** Local plaintext search index over items + lists in the active
   *  account. Built once after initial materialization and maintained
   *  incrementally from the same AppEvent stream that drives the store.
   *  See `spec/search.md`. */
  search: SearchEngine;
  /** Bumps every time at least one event is dispatched (local or
   *  remote). The persistence layer reads this to debounce-save the
   *  doc. The UI doesn't read it — Solid's store gives it granular
   *  reactivity directly. */
  version: Accessor<number>;
  /** Rolling capture of items that just left a live list by being
   *  marked done (local or remote). Entries are dropped when the item
   *  is restored, binned, removed, or moved, and pruned by age; the
   *  linger UI applies its own (shorter) expiry window on top. */
  recentDone: Accessor<readonly RecentDoneEntry[]>;
  /** Pump the engine's AppEvent queue into the store. The WS bridge
   *  calls this after every server frame; mutation methods call it
   *  inline so local writes flow through the same dispatcher path. */
  drainEvents(): void;
  /** Hook the WS bridge installs to push outbox bytes immediately
   *  after a local mutation rather than waiting for a server frame. */
  setOnFlush(cb: () => void): void;
  /** Hook run *before* `engine.flush()` on every local commit. The web
   *  host wires `engine.captureLocalOps()` here so the just-committed
   *  mutation is a durable op-log row before `flush()` triggers the
   *  outbox-driven push (which reads `storage.outbox()`). Without this
   *  the push would see an empty outbox and fall back to the legacy
   *  `pending_export` path. */
  setBeforeFlush(cb: () => void): void;
  // Reads
  getItem(id: string): ItemView | undefined;
  // Mutations
  addItem(listId: string, text: string): string;
  /** Insert a single item at `indexInList` (per the live-item view of
   *  `listId`). Past-end indices append. Single Loro op — no
   *  intermediate "appended at end" state. */
  addItemAt(listId: string, text: string, indexInList: number): string;
  /** Bulk-insert `texts` as a contiguous run starting at
   *  `indexInList`. Single commit, single drain — peers and the local
   *  UI see one update, not N. */
  addItemsAt(listId: string, texts: string[], indexInList: number): string[];
  editItemText(id: string, text: string): void;
  /** Set the free-form notes string: a whole-value convenience over
   *  `applyNotesDelta` (the diff against the current text, one excluded
   *  commit), so notes never enter the core's UndoManager. Empty clears
   *  it; whitespace is preserved verbatim. Undo: an open editing session
   *  for the item absorbs it; inside `withActionBatch` it joins the
   *  batch's step; otherwise it is one workspace step of its own. */
  setItemNotes(id: string, notes: string): void;
  /** Notes delta bridge (spec/notes-plan.md Phase 2). `subscribeNotes`
   *  returns the current text and starts streaming remote / other-tab /
   *  undo changes to `onNotesDelta` listeners as UTF-16 deltas; call it
   *  again after a `null` (resync) notification to reload. Local edits go
   *  through `applyNotesDelta`, which commits immediately (excluded from
   *  workspace undo) but defers the durable capture + push to a short
   *  idle timer so a typing burst is one op blob; `flushNotes` runs that
   *  capture now (blur, close, page hide). */
  subscribeNotes(id: string): string;
  unsubscribeNotes(id: string): void;
  applyNotesDelta(id: string, ops: readonly NotesDeltaOp[]): void;
  flushNotes(): void;
  /** End the current notes editing session (the editor blurred): flush,
   *  then record the net change since `subscribeNotes` / the previous
   *  session end as one workspace undo step. Keystrokes stay out of the
   *  workspace undo while the editor is focused (the browser's own text
   *  undo owns them there); this is what makes the edit reversible once
   *  focus has left. `unsubscribeNotes` ends the session too. */
  endNotesSession(): void;
  /** Listen for inbound notes deltas. `ops` is `null` after a
   *  `fullResync`: the subscriber must reload the text. Returns the
   *  unsubscribe function. */
  onNotesDelta(cb: (id: string, ops: readonly NotesDeltaOp[] | null) => void): () => void;
  /** Set (`YYYY-MM-DD`) or clear (`null`) an item's date-only deadline.
   *  The value is a floating local calendar date; a malformed string is
   *  rejected by the core. */
  setItemDeadline(id: string, deadline: string | null): void;
  /** Set (`YYYY-MM-DD` or `YYYY-MM-DDTHH:MM`) or clear (`null`) an
   *  item's planned date. Floating; malformed values are rejected by the
   *  core. */
  setItemWhen(id: string, when: string | null): void;
  /** Set (whole minutes, 1..=10080) or clear (`null`) an item's duration.
   *  Clearing `when` clears it in the core as well. */
  setItemDuration(id: string, minutes: number | null): void;
  /** Done toggle: `true` is the Done transition; `false` is un-done — a
   *  plain write to Backlog, applied only to currently-Done items. */
  setDone(id: string, done: boolean): void;
  setDoneMany(ids: string[], done: boolean): void;
  /** Un-done from inside the Focus lens: clear each id's done flag and
   *  re-pin it at the Focus slot it vacated (Done auto-removes the ref,
   *  spec/focus.md, so "un-done" in Focus is the deliberate re-add). Ids
   *  without a captured slot land at the top. One undo step. */
  undoneIntoFocus(ids: string[]): void;
  /** Bin toggle: `true` sets the bin mask (workflow state preserved for
   *  restore); `false` restores — clears the mask only, revealing the
   *  preserved state (which may itself be done). */
  setBinned(id: string, binned: boolean): void;
  setBinnedMany(ids: string[], binned: boolean): void;
  /** Move one item to a lifecycle in a single commit — the board's
   *  lane-drop primitive (`spec/board.md`). An open→open flip keeps
   *  the item in its list's Open order; done/binned remove it. */
  setLifecycle(id: string, lifecycle: Lifecycle): void;
  setLifecycleMany(ids: string[], lifecycle: Lifecycle): void;
  /** Board open-lane capture: append a new item directly in an open
   *  workflow state. */
  addItemInState(listId: string, text: string, state: WorkflowState): string;
  /** Board open-lane capture at a position: insert a new item in `state`
   *  at `indexInList` in the list's Open projection (like `addItemAt`). */
  addItemInStateAt(
    listId: string,
    text: string,
    state: WorkflowState,
    indexInList: number,
  ): string;
  moveItem(id: string, listId: string, indexInList: number): void;
  deleteBinned(id: string): void;
  deleteBinnedMany(ids: string[]): void;
  emptyBin(): number;
  addList(name: string): string;
  renameList(id: string, name: string): void;
  /** Set (`icon` = emoji grapheme) or clear (`icon` = "") a list's
   *  display icon. */
  setListIcon(id: string, icon: string): void;
  /** Save (`view` = an encoded `DefaultView`) or clear (`view` = `null`)
   *  a list's default view — the lens clients render it in when they
   *  have no local override. Accepts the reserved `inbox`, whose default
   *  lands in the doc-level settings map. See `spec/board.md`. */
  setDefaultView(id: string, view: string | null): void;
  /** Reorder an active list to `index` in the **active-list** projection
   *  (the index space the nav's draggable section renders); the core
   *  resolves it to the raw CRDT index across any interspersed archived
   *  rows. */
  moveList(id: string, index: number): void;
  /** Archive (`true`) or unarchive (`false`) a user-created list — the
   *  user-facing removal from the active workspace. Metadata-only: items,
   *  ordering, lifecycle, and Focus are untouched, and the list stays in
   *  `listsOrder` / `listsById` (`spec/data-model.md` "Archived lists").
   *  There is deliberately no user-facing list delete. */
  setListArchived(id: string, archived: boolean): void;
  /** Toggle the global "show counts on non-Queue lists" setting.
   *  Queue's own count is always visible (subject to count > 0) and is
   *  not gated by this flag. */
  setShowListCounts(show: boolean): void;
  /** Pin an item into the Focus lens at `index` in the visible curated
   *  order (default: the top — new focus items are "what am I working on
   *  now"). No-op if the item already has a visible ref or isn't Open;
   *  throws if the item is unknown. See spec/focus.md. */
  addToFocus(id: string, index?: number): void;
  /** Batch add: pin each id into the Focus lens (prepended to the top in
   *  order) in a single commit. Unknown / not-Open / already-focused ids
   *  are skipped. Backs multi-select "add to focus". */
  addToFocusMany(ids: string[]): void;
  /** Remove an item's ref(s) from the Focus lens. The item is untouched. */
  removeFromFocus(id: string): void;
  /** Batch remove: drop each id's ref(s) from the Focus lens in one commit. */
  removeFromFocusMany(ids: string[]): void;
  /** Reorder an item within the Focus lens to visible position `index`. */
  moveInFocus(id: string, index: number): void;
  /** Per-session local undo. Returns whether a step was applied so the
   *  caller can decide whether to `preventDefault()` the keybinding.
   *  Remote-applied ops are excluded by origin tag — see
   *  `spec/sync-protocol.md` "Commit origin tagging". */
  undo(): boolean;
  redo(): boolean;
  canUndo(): boolean;
  canRedo(): boolean;
  /** Subscribe to applied undo / redo steps. The outcome names the items
   *  and lists whose events the step produced, in event order, so the
   *  view can land the user on what just changed. An id may no longer
   *  resolve (an undone add removes the item outright). */
  onUndoRedo(cb: (outcome: UndoOutcome) => void): () => void;
  withActionBatch<T>(fn: () => T): T;
  /** Additive JSON import: lists in `json` are created as fresh user
   *  lists, items get fresh IDs and route into them (or local `main`).
   *  Single Loro commit → one undo step. */
  importJson(json: string): ImportSummary;
}

/** What an undo / redo step touched: see `DocApp.onUndoRedo`. */
export interface UndoOutcome {
  itemIds: string[];
  listIds: string[];
}

export interface ImportSummary {
  listsAdded: number;
  itemsAdded: number;
  itemsSkipped: number;
  focusAdded: number;
}

const COARSE_BATCH_THRESHOLD = 64;
const COARSE_EVENT_KINDS = new Set([
  "itemAdded",
  "itemMoved",
  "itemRemoved",
  "itemLifecycleChanged",
  "itemListChanged",
]);

interface WorkspaceSnapshotPayload {
  settings: SettingsView;
  lists: ListView[];
  /** `state` crosses the boundary as a wire string; normalize on read so
   *  a newer client's state degrades to `backlog` instead of breaking
   *  the lane partition. */
  items: (Omit<ItemView, "state"> & { state?: string })[];
}

function materializeEngineSnapshot(engine: SyncEngine): WorkspaceState {
  const payload = JSON.parse(engine.workspaceSnapshotJson()) as WorkspaceSnapshotPayload;
  const itemsById: Record<string, ItemView> = {};
  const listOpen: Record<string, string[]> = {};
  let binCount = 0;
  for (const raw of payload.items) {
    const item: ItemView = { ...raw, state: parseWorkflowState(raw.state) };
    itemsById[item.id] = item;
    if (isOpen(item)) (listOpen[item.listId] ??= []).push(item.id);
    if (isBinned(item)) binCount++;
  }
  const listsById: Record<string, ListView> = {};
  for (const list of payload.lists) {
    listsById[list.id] = {
      id: list.id,
      name: list.name,
      icon: list.icon,
      // Absent in the JSON when the list has no saved default; carry it
      // through or a boot / coarse resync silently drops the saved view
      // and the client falls back to the built-in list lens.
      defaultView: list.defaultView,
      // Same carry-through for archive state: dropping it would resurrect
      // archived lists into the active nav on every boot / resync.
      archivedAt: list.archivedAt,
      createdAt: list.createdAt,
    };
  }
  return {
    itemsById,
    listOpen,
    binCount,
    // Not part of the workspace snapshot JSON (which is items + lists +
    // settings only) — read straight from the engine's focus projection.
    // Covers the coarse / fullResync reconcile path too, since both
    // rematerialize through here.
    focusOrder: engine.focusRefIds(),
    listsOrder: payload.lists.map((list) => list.id),
    listsById,
    settings: {
      showListCounts: payload.settings.showListCounts ?? true,
      inboxView: payload.settings.inboxView ?? null,
    },
  };
}

function shouldUseCoarseProjection(events: readonly AppEventJs[]): boolean {
  if (events.length < COARSE_BATCH_THRESHOLD) return false;
  let coarseCandidates = 0;
  for (const ev of events) {
    if (COARSE_EVENT_KINDS.has(ev.kind)) coarseCandidates++;
  }
  return coarseCandidates >= events.length / 2;
}

export function createSyncedApp(engine: SyncEngine): DocApp {
  const [state, setState] = createStore<WorkspaceState>({
    itemsById: {},
    listOpen: {},
    binCount: 0,
    focusOrder: [],
    listsOrder: [],
    listsById: {},
    settings: {
      showListCounts: true,
      inboxView: null,
    },
  });
  const [version, setVersion] = createSignal(0);
  const [recentDone, setRecentDone] = createSignal<readonly RecentDoneEntry[]>(
    [],
  );
  const search = createSearchEngine();
  let actionBatchDepth = 0;
  let flushDeferred = false;
  let actionBatchStartVersion = 0;
  let pendingActionSteps = 0;
  // A workspace undo entry pairs a count of core `UndoManager` steps
  // with the notes writes made alongside them. Notes commits carry the
  // `notes:` origin the core excludes, so the store reverts them itself
  // by re-applying the inverse delta through that same excluded path.
  // Undo reverts the notes first, then the core steps (an item's notes
  // go before the item does); redo mirrors that.
  type NotesStep = { id: string; before: string; after: string };
  type UndoEntry = { steps: number; notes: NotesStep[] };
  const undoStack: UndoEntry[] = [];
  const redoStack: UndoEntry[] = [];
  // Notes writes made inside the open action batch, folded into its entry.
  let pendingBatchNotes: NotesStep[] = [];
  // While an undo / redo step is being applied, every drained event's
  // subject id is collected here (items and lists separately, deduped,
  // event order) and handed to `onUndoRedo` listeners once it lands.
  let undoTouched: UndoOutcome | null = null;
  const undoListeners = new Set<(outcome: UndoOutcome) => void>();
  const collectTouched = (events: readonly AppEventJs[]): void => {
    const touched = undoTouched;
    if (!touched) return;
    for (const ev of events) {
      if (!ev.id) continue;
      const bucket = ev.kind.startsWith("item")
        ? touched.itemIds
        : ev.kind.startsWith("list")
          ? touched.listIds
          : null;
      if (bucket && !bucket.includes(ev.id)) bucket.push(ev.id);
    }
  };
  // Run `step` with touched-id collection on; notify listeners when it
  // reports a step was applied.
  const withUndoOutcome = (step: () => boolean): boolean => {
    undoTouched = { itemIds: [], listIds: [] };
    let did = false;
    try {
      did = step();
    } finally {
      const outcome = undoTouched;
      undoTouched = null;
      if (did && outcome) for (const cb of undoListeners) cb(outcome);
    }
    return did;
  };

  // ---- listOpen helpers: every write is list-local. `insertOpen`
  // removes any existing occurrence first so re-dispatch of an id
  // (e.g. an add event for an item we already track) stays idempotent.
  const insertOpen = (
    listId: string,
    id: string,
    index: number | undefined,
  ): void => {
    const cur = state.listOpen[listId];
    const next = cur ? cur.filter((x) => x !== id) : [];
    next.splice(Math.min(index ?? next.length, next.length), 0, id);
    setState("listOpen", listId, next);
  };
  const removeOpen = (listId: string, id: string): void => {
    const cur = state.listOpen[listId];
    if (!cur || !cur.includes(id)) return;
    setState(
      "listOpen",
      listId,
      cur.filter((x) => x !== id),
    );
  };
  const adjustBinCount = (delta: number): void => {
    if (delta !== 0) setState("binCount", (n) => n + delta);
  };

  // ---- recentDone (linger capture). Entries older than this are
  // unreachable by any linger chain (Workspace's window is shorter);
  // pruning on write keeps the array a handful of entries.
  const RECENT_DONE_TTL_MS = 15_000;
  const captureRecentDone = (entry: RecentDoneEntry): void => {
    setRecentDone((prev) => [
      ...prev.filter(
        (e) => e.id !== entry.id && entry.doneAt - e.doneAt < RECENT_DONE_TTL_MS,
      ),
      entry,
    ]);
  };
  const dropRecentDone = (id: string): void => {
    setRecentDone((prev) =>
      prev.some((e) => e.id === id) ? prev.filter((e) => e.id !== id) : prev,
    );
  };

  // Notes delta bridge state: editor listeners, and the idle timer that
  // coalesces a typing burst into one capture + push.
  const notesListeners = new Set<
    (id: string, ops: readonly NotesDeltaOp[] | null) => void
  >();
  const NOTES_FLUSH_IDLE_MS = 300;
  let notesFlushTimer: ReturnType<typeof setTimeout> | undefined;
  let notesFlushPending = false;
  // The open notes editing session: which item, and its text when the
  // session began. Its net change becomes one undo step at session end.
  let notesSession: { id: string; base: string } | null = null;

  const dispatch = (ev: AppEventJs): void => {
    switch (ev.kind) {
      case "fullResync":
        // `drainEvents` handles this control event before dispatch.
        break;
      case "itemAdded": {
        const prev = state.itemsById[ev.id];
        if (prev) {
          if (isOpen(prev)) removeOpen(prev.listId, ev.id);
          if (isBinned(prev)) adjustBinCount(-1);
        }
        const item: ItemView = {
          id: ev.id,
          listId: ev.listId ?? "",
          text: ev.text ?? "",
          notes: ev.notes ?? "",
          state: parseWorkflowState(ev.state),
          lifecycleAt: Number(ev.lifecycleAt ?? ev.createdAt ?? 0),
          deadline: ev.deadline ?? undefined,
          when: ev.when ?? undefined,
          duration: ev.duration != null ? Number(ev.duration) : undefined,
          createdAt: Number(ev.createdAt ?? 0),
          startedAt: ev.startedAt != null ? Number(ev.startedAt) : undefined,
          doneAt: ev.doneAt != null ? Number(ev.doneAt) : undefined,
          binnedAt: ev.binnedAt != null ? Number(ev.binnedAt) : undefined,
        };
        setState("itemsById", ev.id, item);
        if (isOpen(item)) insertOpen(item.listId, ev.id, ev.openIndex);
        if (isBinned(item)) adjustBinCount(1);
        break;
      }
      case "itemRemoved": {
        const prev = state.itemsById[ev.id];
        if (!prev) break;
        if (isOpen(prev)) removeOpen(prev.listId, ev.id);
        if (isBinned(prev)) adjustBinCount(-1);
        dropRecentDone(ev.id);
        setState(
          "itemsById",
          produce((by) => {
            delete by[ev.id];
          }),
        );
        break;
      }
      case "itemMoved": {
        // Pure reordering. Any list change arrived as the preceding
        // `itemListChanged`, so `prev.listId` is already the
        // destination; done/binned items carry no openIndex and their
        // view order is timestamp-derived, so there is nothing to do.
        const prev = state.itemsById[ev.id];
        if (!prev) break;
        if (isOpen(prev) && ev.openIndex != null) {
          insertOpen(prev.listId, ev.id, ev.openIndex);
        }
        break;
      }
      case "itemTextChanged": {
        if (state.itemsById[ev.id]) {
          setState("itemsById", ev.id, "text", ev.text ?? "");
        }
        break;
      }
      case "itemNotesChanged": {
        if (state.itemsById[ev.id]) {
          setState("itemsById", ev.id, "notes", ev.notes ?? "");
        }
        break;
      }
      case "itemNotesDelta": {
        let ops: NotesDeltaOp[] = [];
        try {
          ops = JSON.parse(ev.delta ?? "[]") as NotesDeltaOp[];
        } catch {
          break;
        }
        for (const cb of notesListeners) cb(ev.id, ops);
        break;
      }
      case "itemLifecycleChanged": {
        const prev = state.itemsById[ev.id];
        if (!prev) break;
        const wasOpen = isOpen(prev);
        const wasBinned = isBinned(prev);
        const nextState = parseWorkflowState(ev.state);
        const lifecycleAt = Number(ev.lifecycleAt ?? prev.lifecycleAt);
        const startedAt = ev.startedAt != null ? Number(ev.startedAt) : undefined;
        const doneAt = ev.doneAt != null ? Number(ev.doneAt) : undefined;
        const binnedAt = ev.binnedAt != null ? Number(ev.binnedAt) : undefined;
        const nowOpen = binnedAt == null && nextState !== "done";
        if (wasOpen && nextState === "done" && binnedAt == null) {
          // Leaving the Open projection by being marked done: snapshot
          // the vacated position (before the removal below) for the
          // linger re-insert.
          const idx = state.listOpen[prev.listId]?.indexOf(ev.id) ?? -1;
          // Done also auto-removes the item's Focus ref (spec/focus.md), so
          // snapshot its Focus slot too. `focusOrder` is re-derived
          // wholesale after the drain; drop the id here as well so later
          // captures in the same drain see sequential indices, matching
          // what `restoreCapturedPositions` replays.
          const focusIdx = state.focusOrder.indexOf(ev.id);
          if (focusIdx >= 0) {
            setState("focusOrder", (order) => order.filter((x) => x !== ev.id));
          }
          captureRecentDone({
            id: ev.id,
            listId: prev.listId,
            index: idx >= 0 ? idx : 0,
            focusIndex: focusIdx >= 0 ? focusIdx : undefined,
            doneAt: lifecycleAt,
          });
        } else if (nowOpen || binnedAt != null) {
          dropRecentDone(ev.id);
        }
        // Only an open↔closed transition touches `listOpen`. An open→open
        // workflow flip (wasOpen && nowOpen) leaves the item in place —
        // its lane is recomputed from the updated register state below.
        if (wasOpen && !nowOpen) removeOpen(prev.listId, ev.id);
        if (!wasOpen && nowOpen) insertOpen(prev.listId, ev.id, ev.openIndex);
        setState("itemsById", ev.id, {
          state: nextState,
          lifecycleAt,
          startedAt,
          doneAt,
          binnedAt,
        });
        adjustBinCount((binnedAt != null ? 1 : 0) - (wasBinned ? 1 : 0));
        break;
      }
      case "itemDeadlineChanged": {
        if (state.itemsById[ev.id]) {
          setState("itemsById", ev.id, "deadline", ev.deadline ?? undefined);
        }
        break;
      }
      case "itemWhenChanged": {
        if (state.itemsById[ev.id]) {
          setState("itemsById", ev.id, "when", ev.when ?? undefined);
        }
        break;
      }
      case "itemDurationChanged": {
        if (state.itemsById[ev.id]) {
          setState(
            "itemsById",
            ev.id,
            "duration",
            ev.duration != null ? Number(ev.duration) : undefined,
          );
        }
        break;
      }
      case "itemListChanged": {
        const prev = state.itemsById[ev.id];
        if (!prev) break;
        // Lifecycle is untouched by this event — membership in the Open
        // projection carries over, only the owning list changes.
        const open = isOpen(prev);
        if (open) removeOpen(prev.listId, ev.id);
        setState("itemsById", ev.id, "listId", ev.listId ?? "");
        if (open) insertOpen(ev.listId ?? "", ev.id, ev.openIndex);
        dropRecentDone(ev.id);
        break;
      }
      case "listAdded": {
        setState("listsById", ev.id, {
          id: ev.id,
          name: ev.name ?? "",
          archivedAt: ev.archivedAt != null ? Number(ev.archivedAt) : undefined,
          createdAt: Number(ev.createdAt ?? 0),
        });
        const targetIndex = ev.index ?? state.listsOrder.length;
        setState(
          "listsOrder",
          produce((order) => {
            const cur = order.indexOf(ev.id);
            if (cur >= 0) order.splice(cur, 1);
            const insertAt = Math.min(targetIndex, order.length);
            order.splice(insertAt, 0, ev.id);
          }),
        );
        break;
      }
      case "listRemoved": {
        setState("listsOrder", (o) => o.filter((id) => id !== ev.id));
        setState(
          "listsById",
          produce((by) => {
            delete by[ev.id];
          }),
        );
        // Core reassigns the list's items to `main` (as preceding
        // `itemListChanged` events), so the Open array is empty by now
        // — drop the key so `listOpen` doesn't accumulate dead lists.
        setState(
          "listOpen",
          produce((by) => {
            delete by[ev.id];
          }),
        );
        break;
      }
      case "listMoved": {
        const target = ev.index ?? 0;
        setState(
          "listsOrder",
          produce((order) => {
            const cur = order.indexOf(ev.id);
            if (cur < 0) return;
            order.splice(cur, 1);
            order.splice(Math.min(target, order.length), 0, ev.id);
          }),
        );
        break;
      }
      case "listRenamed": {
        if (state.listsById[ev.id]) {
          setState("listsById", ev.id, "name", ev.name ?? "");
        }
        break;
      }
      case "listIconChanged": {
        if (state.listsById[ev.id]) {
          // `ev.icon` is undefined when the icon was removed — mirror
          // that so the nav falls back to the built-in glyph.
          setState("listsById", ev.id, "icon", ev.icon ?? undefined);
        }
        break;
      }
      case "listDefaultViewChanged": {
        if (state.listsById[ev.id]) {
          // `ev.defaultView` is undefined when the default was cleared —
          // mirror that so this client falls back to its own default.
          setState("listsById", ev.id, "defaultView", ev.defaultView ?? undefined);
        }
        break;
      }
      case "listArchivedChanged": {
        if (state.listsById[ev.id]) {
          // `ev.archivedAt` is undefined when the list was unarchived —
          // mirror that so the list re-enters the active projection.
          setState(
            "listsById",
            ev.id,
            "archivedAt",
            ev.archivedAt != null ? Number(ev.archivedAt) : undefined,
          );
        }
        break;
      }
      case "settingsChanged": {
        // Mirror the whole event payload — settings are tiny and the
        // wire format always sends the full known shape, so a single
        // setState keeps the store in lockstep with the doc.
        setState("settings", {
          showListCounts: ev.showListCounts ?? true,
          inboxView: ev.inboxView ?? null,
        });
        break;
      }
    }
  };

  const drainEvents = (): void => {
    const events: AppEventJs[] = [];
    while (true) {
      const ev = engine.popAppEvent();
      if (!ev) break;
      events.push(ev);
    }
    collectTouched(events);
    const coarse = shouldUseCoarseProjection(events);
    const fullResync = events.some((ev) => ev.kind === "fullResync");
    // Batch so a multi-event drain (e.g. addItemsAt for a multi-line
    // paste, or a server frame applying many remote ops) shows up as
    // one reactive update — otherwise consumers like the dnd briefly
    // see the intermediate order and animate through it.
    batch(() => {
      if (fullResync || coarse) {
        const next = materializeEngineSnapshot(engine);
        setState(reconcile(next));
        // The bulk path skips per-event store dispatch, so let the
        // search engine do a wholesale rebuild from the fresh state
        // rather than try to track which events fell into the bucket.
        search.rebuild(next);
        // Any per-item delta in the bucket is lost with it; subscribed
        // editors reload from the fresh state.
        for (const cb of notesListeners) cb("", null);
      } else {
        for (const ev of events) {
          dispatch(ev);
          search.apply(ev);
        }
        // Focus visibility depends on both focus-container mutations
        // (`focusChanged`) and item add/remove/lifecycle events elsewhere
        // (a Done/Bin drops a focused item from the view, and Done also
        // auto-removes its ref). One wholesale re-derive per non-empty
        // drain is the cheapest correct approach — `reconcile` no-ops when
        // the order is unchanged. See spec/focus.md B.8.
        if (events.length > 0) {
          setState("focusOrder", reconcile(engine.focusRefIds()));
        }
      }
      if (events.length > 0) setVersion((v) => v + 1);
    });
  };

  // Initial attach is explicit materialization, not a live event replay.
  // A single compact JSON snapshot crosses the wasm boundary and the
  // historical event queue remains empty.
  const initialState = materializeEngineSnapshot(engine);
  setState(reconcile(initialState));
  search.rebuild(initialState);

  let onFlush: () => void = () => {};
  let beforeFlush: () => void = () => {};
  const flush = (): void => {
    if (actionBatchDepth > 0) {
      flushDeferred = true;
      return;
    }
    beforeFlush();
    engine.flush();
    onFlush();
    // Local mutations enqueue AppEvents synchronously; pull them so
    // the next Solid tick sees the store update.
    drainEvents();
  };

  const flushNotesNow = (): void => {
    clearTimeout(notesFlushTimer);
    notesFlushTimer = undefined;
    if (!notesFlushPending) return;
    notesFlushPending = false;
    flush();
  };

  const recordAction = (steps: number, notes: NotesStep[] = []): void => {
    if (steps <= 0 && notes.length === 0) return;
    undoStack.push({ steps, notes });
    redoStack.length = 0;
  };

  // Close the open notes session as one undo step (nothing if the text
  // is back where it started, e.g. fully undone natively while focused)
  // and start the next one from the current text.
  const endNotesSession = (): void => {
    flushNotesNow();
    const session = notesSession;
    if (!session) return;
    const item = state.itemsById[session.id];
    if (!item) {
      notesSession = null;
      return;
    }
    const after = item.notes ?? "";
    if (after !== session.base) {
      recordAction(0, [{ id: session.id, before: session.base, after }]);
    }
    notesSession = { id: session.id, base: after };
  };

  // Write `to` over `from` on a notes step's item as a positional delta
  // via the undo-excluded notes path, so reverting never registers a
  // core step of its own. Peer edits since the step landed shift or
  // survive around the region; a delta the core rejects (the item is
  // gone, or the text no longer fits) skips the step. Subscribed editors
  // learn of the change like any inbound delta.
  const applyNotesStep = (id: string, from: string, to: string): boolean => {
    const ops = diffToDelta(from, to);
    if (!ops) return true;
    try {
      engine.applyNotesDelta(id, JSON.stringify(ops));
    } catch (e) {
      console.warn("notes undo step skipped:", e);
      return false;
    }
    drainEvents();
    if (notesSession?.id === id) {
      notesSession = { id, base: state.itemsById[id]?.notes ?? "" };
    }
    for (const cb of notesListeners) cb(id, ops);
    return true;
  };

  const mutate = <T>(fn: () => T, assumedSteps = 1): T => {
    if (actionBatchDepth > 0) {
      pendingActionSteps += assumedSteps;
      const result = fn();
      flush();
      return result;
    }
    const before = version();
    const result = fn();
    flush();
    if (version() !== before) recordAction(assumedSteps);
    return result;
  };

  const withActionBatch = <T,>(fn: () => T): T => {
    const outermost = actionBatchDepth === 0;
    actionBatchDepth++;
    if (outermost) {
      actionBatchStartVersion = version();
      pendingActionSteps = 0;
      pendingBatchNotes = [];
    }
    try {
      return fn();
    } finally {
      actionBatchDepth--;
      if (outermost) {
        if (flushDeferred) {
          flushDeferred = false;
          flush();
        }
        if (version() !== actionBatchStartVersion) {
          recordAction(pendingActionSteps, pendingBatchNotes);
        }
        pendingActionSteps = 0;
        pendingBatchNotes = [];
      }
    }
  };

  // One undo / redo step off its stack. Both run under
  // `withUndoOutcome`, which reports the touched ids on success.
  const undoStep = (): boolean => {
    for (;;) {
      const entry = undoStack.pop();
      if (entry == null) return false;
      // Notes first (newest first), then the core steps. A notes step
      // the core rejects is dropped from the entry.
      const notes = entry.notes.filter((n) => applyNotesStep(n.id, n.after, n.before));
      let applied = 0;
      for (let i = 0; i < entry.steps; i++) {
        if (!engine.undo()) break;
        applied++;
      }
      if (applied === 0 && notes.length === 0) {
        if (entry.steps > 0) {
          undoStack.push(entry);
          return false;
        }
        continue;
      }
      flush();
      redoStack.push({ steps: applied, notes });
      return true;
    }
  };

  const redoStep = (): boolean => {
    for (;;) {
      const entry = redoStack.pop();
      if (entry == null) return false;
      let applied = 0;
      for (let i = 0; i < entry.steps; i++) {
        if (!engine.redo()) break;
        applied++;
      }
      const notes = entry.notes.filter((n) => applyNotesStep(n.id, n.before, n.after));
      if (applied === 0 && notes.length === 0) {
        if (entry.steps > 0) {
          redoStack.push(entry);
          return false;
        }
        continue;
      }
      flush();
      undoStack.push({ steps: applied, notes });
      return true;
    }
  };

  return {
    engine,
    state,
    version,
    recentDone,
    search,
    drainEvents,
    setOnFlush(cb) {
      onFlush = cb;
    },
    setBeforeFlush(cb) {
      beforeFlush = cb;
    },
    getItem(id) {
      return state.itemsById[id];
    },
    addItem(listId, text) {
      return mutate(() => engine.addItem(listId, text));
    },
    addItemAt(listId, text, indexInList) {
      return mutate(() => engine.addItemAt(listId, text, indexInList));
    },
    addItemsAt(listId, texts, indexInList) {
      return mutate(() => engine.addItemsAt(listId, texts, indexInList));
    },
    editItemText(id, text) {
      mutate(() => engine.editItemText(id, text));
    },
    setItemNotes(id, notes) {
      const before = state.itemsById[id]?.notes ?? "";
      const ops = diffToDelta(before, notes);
      if (!ops) return;
      engine.applyNotesDelta(id, JSON.stringify(ops));
      // A caller-driven write: no delta notification (the caller owns
      // any editor showing this item), durable now rather than on the
      // typing idle timer.
      flush();
      const step = { id, before, after: notes };
      if (notesSession?.id === id) {
        // The session records its net change at blur / switch.
      } else if (actionBatchDepth > 0) {
        pendingBatchNotes.push(step);
      } else {
        recordAction(0, [step]);
      }
    },
    subscribeNotes(id) {
      endNotesSession();
      const text = engine.subscribeNotes(id);
      notesSession = { id, base: text };
      return text;
    },
    unsubscribeNotes(id) {
      endNotesSession();
      if (notesSession?.id === id) notesSession = null;
      engine.unsubscribeNotes(id);
    },
    applyNotesDelta(id, ops) {
      if (ops.length === 0) return;
      // Commits into the doc now (the store and search index see the
      // new string on this drain) without recording a workspace undo
      // step; the durable capture + push waits for the idle timer.
      engine.applyNotesDelta(id, JSON.stringify(ops));
      drainEvents();
      notesFlushPending = true;
      clearTimeout(notesFlushTimer);
      notesFlushTimer = setTimeout(flushNotesNow, NOTES_FLUSH_IDLE_MS);
    },
    flushNotes() {
      flushNotesNow();
    },
    endNotesSession() {
      endNotesSession();
    },
    onNotesDelta(cb) {
      notesListeners.add(cb);
      return () => {
        notesListeners.delete(cb);
      };
    },
    setItemDeadline(id, deadline) {
      mutate(() => engine.setItemDeadline(id, deadline ?? undefined));
    },
    setItemWhen(id, when) {
      mutate(() => engine.setItemWhen(id, when ?? undefined));
    },
    setItemDuration(id, minutes) {
      // The core rejects zero, negative, and over-a-week values; refuse
      // anything that is not a positive whole number here so a bad value
      // never reaches the engine (where it would throw).
      if (minutes != null && (!Number.isInteger(minutes) || minutes <= 0)) return;
      mutate(() => engine.setItemDuration(id, minutes ?? undefined));
    },
    setDone(id, done) {
      mutate(() => engine.setItemDone(id, done));
    },
    setDoneMany(ids, done) {
      mutate(() => engine.setItemsDone(ids, done));
    },
    undoneIntoFocus(ids) {
      if (ids.length === 0) return;
      const wanted = new Set(ids);
      const captured = recentDone().filter((r) => wanted.has(r.id));
      const capturedIds = new Set(captured.map((r) => r.id));
      withActionBatch(() => {
        mutate(() => engine.setItemsDone(ids, false));
        // Slots were captured sequentially (each after earlier removals),
        // so re-insert newest-first to rebuild the original layout, as
        // `restoreCapturedPositions` does for the render overlay.
        for (let i = captured.length - 1; i >= 0; i--) {
          const r = captured[i];
          mutate(() => engine.addToFocus(r.id, r.focusIndex ?? 0));
        }
        for (const id of ids) {
          if (!capturedIds.has(id)) mutate(() => engine.addToFocus(id, 0));
        }
      });
    },
    setBinned(id, binned) {
      mutate(() => engine.setItemBinned(id, binned));
    },
    setBinnedMany(ids, binned) {
      mutate(() => engine.setItemsBinned(ids, binned));
    },
    setLifecycle(id, lifecycle) {
      mutate(() => engine.setItemLifecycle(id, lifecycleEnum(lifecycle)));
    },
    setLifecycleMany(ids, lifecycle) {
      mutate(() => engine.setItemsLifecycle(ids, lifecycleEnum(lifecycle)));
    },
    addItemInState(listId, text, state) {
      return mutate(() => engine.addItemInState(listId, text, lifecycleEnum(state)));
    },
    addItemInStateAt(listId, text, state, indexInList) {
      return mutate(() =>
        engine.addItemInStateAt(listId, text, lifecycleEnum(state), indexInList),
      );
    },
    moveItem(id, listId, indexInList) {
      mutate(() => engine.moveItem(id, listId, indexInList));
    },
    deleteBinned(id) {
      mutate(() => engine.deleteBinned(id));
    },
    deleteBinnedMany(ids) {
      mutate(() => engine.deleteBinnedItems(ids));
    },
    emptyBin() {
      const before = version();
      const removed = engine.emptyBin();
      if (removed > 0) {
        flush();
        if (version() !== before) recordAction(1);
      }
      return removed;
    },
    addList(name) {
      return mutate(() => engine.addList(name));
    },
    renameList(id, name) {
      mutate(() => engine.renameList(id, name));
    },
    setListIcon(id, icon) {
      mutate(() => engine.setListIcon(id, icon));
    },
    setDefaultView(id, view) {
      mutate(() => engine.setDefaultView(id, view ?? ""));
    },
    importJson(json) {
      return mutate(() => {
        const summaryJson = engine.importJson(json);
        return JSON.parse(summaryJson) as ImportSummary;
      });
    },
    moveList(id, index) {
      mutate(() => engine.moveList(id, index));
    },
    setListArchived(id, archived) {
      mutate(() => engine.setListArchived(id, archived));
    },
    setShowListCounts(show) {
      mutate(() => engine.setShowListCounts(show));
    },
    addToFocus(id, index) {
      // Default to the top: newly-focused items are "what am I working on
      // now" and belong at the head of the lens. An explicit index (e.g.
      // drag-to-reorder) still wins.
      mutate(() => engine.addToFocus(id, index ?? 0));
    },
    addToFocusMany(ids) {
      mutate(() => engine.addToFocusMany(ids));
    },
    removeFromFocus(id) {
      mutate(() => engine.removeFromFocus(id));
    },
    removeFromFocusMany(ids) {
      mutate(() => engine.removeFromFocusMany(ids));
    },
    moveInFocus(id, index) {
      mutate(() => engine.moveInFocus(id, index));
    },
    undo() {
      // An open session is settled first so Cmd+Z right after typing
      // (blur pending) reverts the typing, not an older step.
      endNotesSession();
      return withUndoOutcome(undoStep);
    },
    redo() {
      return withUndoOutcome(redoStep);
    },
    canUndo() {
      return undoStack.length > 0;
    },
    canRedo() {
      return redoStack.length > 0;
    },
    onUndoRedo(cb) {
      undoListeners.add(cb);
      return () => {
        undoListeners.delete(cb);
      };
    },
    withActionBatch,
  };
}
