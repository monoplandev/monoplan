// Reactive shell around the wasm `Replica`. Doc state lives in a
// SolidJS `createStore` keyed by id; mutations go through the replica,
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

import type { AppEventJs, Replica } from "@monoplan/core/wasm";
import { ItemLifecycle } from "@monoplan/core/wasm";
import { diffToDelta, type NotesDeltaOp } from "../notesDelta.ts";
import { batch, createSignal, type Accessor } from "solid-js";
import { createStore, produce, reconcile } from "solid-js/store";
import { createSearchEngine, type SearchEngine } from "../search.ts";

/** Workflow register state (`spec/data-model.md` "Lifecycle"): the
 *  four-step open ladder plus the two terminal (closed) states, Done and
 *  Cancelled, held by the atomic `[state, at]` register. Bin is not a
 *  state — it is the orthogonal `binnedAt` mask. */
export type WorkflowState =
  | "backlog"
  | "todo"
  | "in_progress"
  | "review"
  | "done"
  | "cancelled";

/** The two closed states. Both leave the list's Open projection and
 *  render in the Done view / lane; only `done` is a completion. */
export const CLOSED_STATES: readonly WorkflowState[] = ["done", "cancelled"];

/** The four open states, in board lane order. */
export const OPEN_STATES: readonly WorkflowState[] = [
  "backlog",
  "todo",
  "in_progress",
  "review",
];

/** All six workflow states in ladder order: what a status picker offers
 *  (the bin is not a state, so it is never one of the choices). */
export const WORKFLOW_STATES: readonly WorkflowState[] = [
  ...OPEN_STATES,
  ...CLOSED_STATES,
];

/** A named place, optionally pinned to coordinates (`spec/place-plan.md`).
 *  `label` is the user-facing name and the only required part; `lat` /
 *  `lon` are WGS84 decimal degrees and come as a pair or not at all;
 *  `address` is a display-only formatted address from whatever geocoder
 *  found it; `ref` is an opaque provider hint (`osm:node/123`). */
export interface Place {
  label: string;
  lat?: number;
  lon?: number;
  address?: string;
  ref?: string;
}

/** Parse the JSON text the wasm boundary carries a place as; `undefined`
 *  for unset or anything that is not a place. */
export function parsePlaceJson(json: string | undefined | null): Place | undefined {
  if (!json) return undefined;
  try {
    const v = JSON.parse(json) as unknown;
    if (typeof v !== "object" || v === null) return undefined;
    const p = v as Record<string, unknown>;
    if (typeof p.label !== "string" || !p.label) return undefined;
    const out: Place = { label: p.label };
    if (typeof p.lat === "number" && typeof p.lon === "number") {
      out.lat = p.lat;
      out.lon = p.lon;
    }
    if (typeof p.address === "string" && p.address) out.address = p.address;
    if (typeof p.ref === "string" && p.ref) out.ref = p.ref;
    return out;
  } catch {
    return undefined;
  }
}

export interface ItemView {
  id: string;
  /** The doc this item lives in — stamped by the owning `DocApp` at
   *  projection, never carried on the wire. Every cross-doc lens
   *  (calendar, Focus, Done) merges `ItemView`s from several apps and
   *  routes a mutation back to the owner by this field. */
  docId: string;
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
  /** A date read out of the title as it was typed, waiting for a yes or
   *  no (`spec/calendar-plan.md`, "Typed date phrases"): `when`-shaped.
   *  Shown only while `when` is unset and the item is open. */
  whenSuggested?: string;
  /** Optional duration in whole minutes. Only meaningful beside a timed
   *  `when`; views ignore it otherwise. Absent means unset. */
  duration?: number;
  /** Optional place (`spec/place-plan.md`). Absent means unset. */
  place?: Place;
  createdAt: number;
  /** Account id of the item's creator, when the creating client stamped
   *  one (`Doc.setAuthor`). Absent on items from anonymous sessions and
   *  on everything from before the register existed. Rendered as an
   *  avatar on shared docs only (`spec/sharing-plan.md` §Attribution). */
  createdBy?: string;
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
 *  replica's `setItemLifecycle` / `setItemsLifecycle` expect. */
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
    case "cancelled":
      return ItemLifecycle.Cancelled;
    case "binned":
      return ItemLifecycle.Binned;
  }
}

/** Normalize a wire state string onto the known ladder; anything a newer
 *  client wrote degrades to `backlog` (visible and open), mirroring the
 *  core's unparseable-register fallback. */
function parseWorkflowState(s: string | undefined): WorkflowState {
  switch (s) {
    case "todo":
    case "in_progress":
    case "review":
    case "done":
    case "cancelled":
      return s;
    default:
      return "backlog";
  }
}

/** Workflow register says Done (regardless of the bin mask). */
export const isDone = (it: ItemView): boolean => it.state === "done";
/** Workflow register says Cancelled (regardless of the bin mask). */
export const isCancelled = (it: ItemView): boolean => it.state === "cancelled";
/** Closed state (`done` or `cancelled`) on the register, regardless of
 *  the bin mask. The Done view shows closed, not-binned items. */
export const isClosedState = (s: WorkflowState): boolean =>
  s === "done" || s === "cancelled";
export const isClosed = (it: ItemView): boolean => isClosedState(it.state);
export const isBinned = (it: ItemView): boolean => it.binnedAt != null;
/** Open (one of the four open workflow states, not binned) — the
 *  per-list view. */
export const isOpen = (it: ItemView): boolean =>
  !isBinned(it) && !isClosedState(it.state);
/** The reserved list whose items are events (`spec/events-plan.md`).
 *  Like `inbox` it has no `ListView` row; unlike `inbox` it has no list
 *  or board view: its items surface only on the calendar. */
export const LIST_EVENTS = "events";
/** Located in the reserved `events` list. That is the whole definition
 *  of an event: there is no item kind. */
export const isEvent = (it: ItemView): boolean => it.listId === LIST_EVENTS;
/** What a status picker offers on an event: its open states collapse
 *  into one choice (labelled "Event", standing in as `backlog`, which is
 *  what reopening writes), beside the two closed states. */
export const EVENT_STATES: readonly WorkflowState[] = [
  "backlog",
  "done",
  "cancelled",
];
/** The value a status picker shows for `it`: an Open event reads as the
 *  single open choice whatever open state it holds underneath. */
export const statusValue = (it: ItemView): WorkflowState =>
  isEvent(it) && !isClosedState(it.state) ? "backlog" : it.state;
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
  listsOrder: string[];
  listsById: Record<string, ListView>;
  settings: SettingsView;
}

/** The Home doc's state: a doc's, plus Focus. Focus is personal and
 *  lives only in the account's Home doc (`spec/focus.md`); a shared
 *  doc has no Focus of its own, so its `DocApp` carries none of this. */
export interface HomeState extends WorkspaceState {
  /** Every live Focus entry in curated order, foreign refs included
   *  (`replica.focusEntriesJson()`; `spec/focus.md` §FocusRef grammar).
   *  The personal layer (`docs.ts`) resolves each entry against the doc
   *  that holds its item and prunes the dead foreign ones. This is the
   *  index space `addToFocus` / `moveInFocus` address. Re-derived
   *  wholesale per non-empty event drain, since a focus mutation *or*
   *  an item lifecycle change elsewhere can alter it. */
  focusEntries: FocusEntry[];
}

/** Snapshot of where an item sat in its list's live order at the
 *  moment it was closed (done or cancelled). Feeds the list-view "linger" affordance
 *  (`Workspace`), which briefly re-inserts recently-done rows at their
 *  old position — the live projection itself drops them instantly.
 *  The Focus lens has its own capture in the personal layer
 *  (`docs.ts` `focusRecentDone`): a doc does not know Focus. */
export interface RecentDoneEntry {
  id: string;
  listId: string;
  /** Open index the item occupied just before leaving the projection. */
  index: number;
  /** The closing transition's register timestamp (`lifecycleAt`). */
  doneAt: number;
}

/** One Focus entry as the core keeps it: `docId` null for this doc's
 *  own items, a doc id for a foreign ref (`spec/focus.md`). */
export interface FocusEntry {
  docId: string | null;
  itemId: string;
}

export interface SettingsView {
  /** The reserved `inbox` list's saved default view in encoded form, or
   *  `null` when none is saved. Inbox has no ListMeta row, so its
   *  default lives in the doc-level settings map rather than on
   *  `ListView.defaultView`. */
  inboxView: string | null;
  /** The doc's name, or `null` when unnamed. Doc content, so every
   *  member sees it (`spec/sharing-plan.md`). The Home doc is labelled
   *  "Personal" by clients regardless. */
  docName: string | null;
  /** Doc ids hidden from the merged calendar lens. A personal cross-doc
   *  preference: only the Home doc's value is ever read. */
  calendarHiddenDocs: string[];
}

/** The item-keyed half of a `DocApp`: the per-doc item mutations.
 *  Every member takes item ids, so the personal layer can implement it
 *  too, routing each call to the doc that owns the item by
 *  `ItemView.docId` (`spec/sharing-plan.md` "Decisions", 2026-10-20).
 *  Batches are per doc: a routed multi-id call that spans docs commits
 *  (and undoes) once per doc. Nothing here knows Focus: see
 *  `ItemActions`. */
export interface DocItemActions {
  getItem(id: string): ItemView | undefined;
  editItemText(id: string, text: string): void;
  setItemNotes(id: string, notes: string): void;
  setItemDeadline(id: string, deadline: string | null): void;
  setItemWhen(id: string, when: string | null): void;
  setItemWhenSuggested(id: string, when: string | null): void;
  acceptItemWhenSuggestion(id: string): void;
  setItemDuration(id: string, minutes: number | null): void;
  setItemPlace(id: string, place: Place | null): void;
  setDone(id: string, done: boolean): void;
  setDoneMany(ids: string[], done: boolean): void;
  setBinned(id: string, binned: boolean): void;
  setBinnedMany(ids: string[], binned: boolean): void;
  setLifecycle(id: string, lifecycle: Lifecycle): void;
  setLifecycleMany(ids: string[], lifecycle: Lifecycle): void;
  /** Move within the item's own doc; the routed layer ignores a target
   *  list the item's doc does not hold. A move to another doc is the
   *  personal layer's `moveToDoc` (`spec/sharing-plan.md` "Cross-doc
   *  move"). */
  moveItem(id: string, listId: string, indexInList: number): void;
  deleteBinned(id: string): void;
  deleteBinnedMany(ids: string[]): void;
  withActionBatch<T>(fn: () => T): T;
}

/** What a row, a context menu, the agenda or the task surface needs to
 *  act on the items it shows: the per-doc mutations plus Focus. Only
 *  the personal layer's `routed` implements it. Focus is the account's,
 *  held in the Home doc, and takes items of any open doc (a shared
 *  doc's item as a foreign ref), so no single `DocApp` can answer for
 *  it: a shared doc has no Focus, and Home alone cannot see a foreign
 *  item (`spec/focus.md`). */
export interface ItemActions extends DocItemActions {
  /** Whether the item has a live Focus entry, local or foreign. */
  isFocused(id: string): boolean;
  /** Un-done from inside the Focus lens: reopen each id in its doc and
   *  re-pin it at the Focus slot it vacated (closing drops the ref,
   *  `spec/focus.md`, so "un-done" in Focus is the deliberate re-add).
   *  Ids without a captured slot land at the top. */
  undoneIntoFocus(ids: string[]): void;
  /** Pin an item into Focus at entry position `index` (default: the
   *  top). No-op if it is already focused or isn't Open. */
  addToFocus(id: string, index?: number): void;
  /** Batch add: pin each id (prepended to the top in order). */
  addToFocusMany(ids: string[]): void;
  removeFromFocus(id: string): void;
  removeFromFocusMany(ids: string[]): void;
  moveInFocus(id: string, index: number): void;
}

export interface DocApp extends DocItemActions {
  replica: Replica;
  /** The doc this app projects; what every `ItemView` it emits carries. */
  docId: string;
  state: WorkspaceState;
  /** Query surface over the replica's search index (items + lists in the
   *  active account). The index lives in core and tracks the doc itself;
   *  the store neither builds nor maintains it. See `spec/search.md`. */
  search: SearchEngine;
  /** This doc's raw place uses as JSON (`spec/place-plan.md` "Reuse"),
   *  one input to the cross-doc layer's `placeSuggestions`: known
   *  places are account-scoped, so no doc answers the question alone.
   *  Computed by core on each call (a scan, not an index), so call it
   *  on demand, not reactively. */
  placeUsesJson(): string;
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
  /** Called for each item about to leave the Open projection by being
   *  closed (done or cancelled; local or remote), *before* the store
   *  reflects it, with the closing transition's timestamp. The personal
   *  layer uses it to snapshot the item's Focus slot while the row is
   *  still in place. Returns an unsubscribe. */
  onItemClosing(cb: (id: string, doneAt: number) => void): () => void;
  /** Pump the replica's AppEvent queue into the store. The WS bridge
   *  calls this after every server frame; mutation methods call it
   *  inline so local writes flow through the same dispatcher path. */
  drainEvents(): void;
  /** Hook the WS bridge installs to push outbox bytes immediately
   *  after a local mutation rather than waiting for a server frame. */
  setOnFlush(cb: () => void): void;
  /** Hook run *before* `replica.flush()` on every local commit. The web
   *  host wires `replica.captureLocalOps()` here so the just-committed
   *  mutation is a durable op-log row before `flush()` triggers the
   *  outbox-driven push (which reads `storage.outbox()`). Without this
   *  the push would see an empty outbox and fall back to the legacy
   *  `pending_export` path. */
  setBeforeFlush(cb: () => void): void;
  // Reads
  getItem(id: string): ItemView | undefined;
  // Mutations
  /** Seed a brand-new doc's Inbox with the onboarding item the core
   *  defines (`Doc::seed_welcome`); every client seeds the same way. */
  seedWelcome(): string;
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
   *  idle timer so a typing burst is one update blob; `flushNotes` runs that
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
  /** Write (`when`-shaped) or clear (`null`) an item's pending date
   *  suggestion. Clearing is the user's "no". */
  setItemWhenSuggested(id: string, when: string | null): void;
  /** The user's "yes": the pending suggestion becomes the `when`, one
   *  undo step. No-op when none is pending. */
  acceptItemWhenSuggestion(id: string): void;
  /** Set (whole minutes, 1..=525600) or clear (`null`) an item's duration.
   *  Clearing `when` clears it in the core as well. */
  setItemDuration(id: string, minutes: number | null): void;
  /** Set or clear (`null`) an item's place. A label-only place is
   *  complete; coordinates come as a pair. Validation is the core's. */
  setItemPlace(id: string, place: Place | null): void;
  /** Done toggle: `true` is the Done transition; `false` is un-done — a
   *  plain write to Backlog, applied only to currently-closed (Done or
   *  Cancelled) items. */
  setDone(id: string, done: boolean): void;
  setDoneMany(ids: string[], done: boolean): void;
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
  /** Hard deletes are permanent: they clear this doc's undo / redo
   *  history (core rearms its UndoManager on the same commit). */
  deleteBinned(id: string): void;
  deleteBinnedMany(ids: string[]): void;
  emptyBin(): number;
  /** Cross-doc move, the giving doc's read (`spec/sharing-plan.md`
   *  "Cross-doc move"): the export form of `ids` as a JSON array, the
   *  payload another doc's `adoptItems` takes. Throws on an unknown id. */
  exportItemsJson(ids: string[]): string;
  /** Cross-doc move, the receiving half: insert another doc's
   *  `exportItemsJson` payload as a block at `indexInList` of the list's
   *  Open projection, ids preserved, binned rows restored. One undo
   *  step. Throws when the list is unknown or an id is already here. */
  adoptItems(itemsJson: string, listId: string, indexInList: number): void;
  /** Cross-doc move, the giving half: remove `ids` outright, whatever
   *  their lifecycle. Unlike the Bin's hard delete this is an ordinary
   *  undo step: the items live on in the doc that adopted them, and the
   *  personal layer undoes both halves together. */
  releaseItems(ids: string[]): void;
  /** Drop this doc's undo / redo history, as a hard delete does. The
   *  personal layer calls it when a cross-doc move's other half can no
   *  longer be undone, so this half is never undone alone. */
  forgetUndoHistory(): void;
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
  /** Name the doc (`""` un-names it). Doc content: every member sees it. */
  setDocName(name: string): void;
  /** Hide / show a doc on the merged calendar. Personal: Home doc only. */
  setCalendarDocHidden(docId: string, hidden: boolean): void;
  /** Per-session local undo. Returns whether a step was applied so the
   *  caller can decide whether to `preventDefault()` the keybinding.
   *  Remote-applied ops are excluded by origin tag — see
   *  `spec/sync-protocol.md` "Commit origin tagging". */
  undo(): boolean;
  redo(): boolean;
  canUndo(): boolean;
  canRedo(): boolean;
  /** Stamp of the entry on top of the undo stack, or 0 when empty. A
   *  process-wide monotonic counter (not wall-clock, so two docs never
   *  tie), written when the entry is recorded or redone. The cross-doc
   *  layer undoes the doc whose top entry is newest, which interleaves
   *  every doc's history chronologically while undo itself stays per
   *  doc (`spec/sharing-plan.md`). */
  topUndoStamp(): number;
  /** Stamp of the entry on top of the redo stack (written when it was
   *  undone), or 0 — the layer redoes the doc whose top is newest. */
  topRedoStamp(): number;
  /** Subscribe to applied undo / redo steps. The outcome names the items
   *  and lists whose events the step produced, in event order, so the
   *  view can land the user on what just changed. An id may no longer
   *  resolve (an undone add removes the item outright). */
  onUndoRedo(cb: (outcome: UndoOutcome) => void): () => void;
  withActionBatch<T>(fn: () => T): T;
  /** Additive, idempotent JSON import: lists and items keep their
   *  exported ids, and a row whose id is already in the doc is skipped
   *  untouched (`listsExisting` / `itemsExisting`); rows without a
   *  well-formed id are minted fresh. Single Loro commit → one undo
   *  step. */
  importJson(json: string): ImportSummary;
}

/** The account's Home doc as an app: a `DocApp` plus the Focus
 *  primitives. Focus is a personal-workspace concept with no shared
 *  counterpart: the Home doc's `focus` container holds every entry,
 *  its own items and foreign refs to items of the shared docs alike
 *  (`spec/focus.md`). These address the container only; the personal
 *  layer (`docs.ts`) builds the lens the UI shows on top of them. */
export interface HomeApp extends DocApp {
  state: HomeState;
  /** Pin one of Home's own items at entry position `index` (default:
   *  the top — new focus items are "what am I working on now"). No-op
   *  if the item is already focused or isn't Open; throws if the item
   *  is unknown. */
  addToFocus(id: string, index?: number): void;
  /** Batch add: pin each of Home's own ids (prepended to the top in
   *  order) in a single commit. Unknown / not-Open / already-focused
   *  ids are skipped. */
  addToFocusMany(ids: string[]): void;
  /** Remove an item's ref(s), local or foreign. The item is untouched. */
  removeFromFocus(id: string): void;
  /** Batch remove: drop each id's ref(s) in one commit. */
  removeFromFocusMany(ids: string[]): void;
  /** Reorder an entry, local or foreign, to entry position `index`. */
  moveInFocus(id: string, index: number): void;
  /** Add a foreign FocusRef (`docId:itemId`, an item in another open
   *  doc) at entry position `index`. No Open check — the personal
   *  layer resolved the item and prunes the ref when it closes. No-op
   *  if the item is already focused. */
  addForeignToFocus(docId: string, itemId: string, index: number): void;
  /** Point the entries of `ids` at `docId` (`null` = Home itself), each
   *  in its slot: what a cross-doc move does to a starred item. Ids
   *  with no entry are skipped. A local ref must resolve when this
   *  runs: before Home releases the item, after Home adopts it. */
  retargetFocus(ids: string[], docId: string | null): void;
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
  /** Rows matched by id to one already in the doc and left untouched. */
  listsExisting: number;
  itemsExisting: number;
}

/** Process-wide stamp for undo / redo entries: strictly increasing
 *  across every `DocApp`, so the cross-doc layer can order entries
 *  between docs without wall-clock ties. */
let actionClock = 0;
const nextActionStamp = (): number => ++actionClock;

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
  items: (Omit<ItemView, "state" | "docId"> & { state?: string })[];
}

function materializeReplicaSnapshot(replica: Replica, home: boolean): HomeState {
  const payload = JSON.parse(replica.workspaceSnapshotJson()) as WorkspaceSnapshotPayload;
  const docId = replica.docId();
  const itemsById: Record<string, ItemView> = {};
  const listOpen: Record<string, string[]> = {};
  let binCount = 0;
  for (const raw of payload.items) {
    const item: ItemView = { ...raw, docId, state: parseWorkflowState(raw.state) };
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
    // Home only. Not part of the workspace snapshot JSON (which is
    // items + lists + settings only) — read straight from the replica's
    // focus container. Covers the coarse / fullResync reconcile path
    // too, since both rematerialize through here.
    focusEntries: home ? readFocusEntries(replica) : [],
    listsOrder: payload.lists.map((list) => list.id),
    listsById,
    settings: {
      inboxView: payload.settings.inboxView ?? null,
      docName: payload.settings.docName ?? null,
      calendarHiddenDocs: payload.settings.calendarHiddenDocs ?? [],
    },
  };
}

function readFocusEntries(replica: Replica): FocusEntry[] {
  try {
    const rows = JSON.parse(replica.focusEntriesJson()) as FocusEntry[];
    return rows.map((r) => ({ docId: r.docId ?? null, itemId: r.itemId }));
  } catch (e) {
    console.error("focusEntriesJson failed:", e);
    return [];
  }
}

function shouldUseCoarseProjection(events: readonly AppEventJs[]): boolean {
  if (events.length < COARSE_BATCH_THRESHOLD) return false;
  let coarseCandidates = 0;
  for (const ev of events) {
    if (COARSE_EVENT_KINDS.has(ev.kind)) coarseCandidates++;
  }
  return coarseCandidates >= events.length / 2;
}

/** The app over a shared (non-Home) doc. It has no Focus. */
export function createSyncedApp(replica: Replica): DocApp {
  return buildApp(replica, false);
}

/** The app over the account's Home doc, which owns Focus. */
export function createHomeApp(replica: Replica): HomeApp {
  return buildApp(replica, true);
}

// One builder for both: `home` decides whether the Focus container is
// projected. The Focus members exist on every built object but only
// `createHomeApp` types them in.
function buildApp(replica: Replica, home: boolean): HomeApp {
  const [state, setState] = createStore<HomeState>({
    itemsById: {},
    listOpen: {},
    binCount: 0,
    focusEntries: [],
    listsOrder: [],
    listsById: {},
    settings: {
      inboxView: null,
      docName: null,
      calendarHiddenDocs: [],
    },
  });
  const [version, setVersion] = createSignal(0);
  const [recentDone, setRecentDone] = createSignal<readonly RecentDoneEntry[]>(
    [],
  );
  const search = createSearchEngine(replica);
  const docId = replica.docId();
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
  // `at` is the entry's stamp: when it was recorded (undo stack) or
  // when it was undone (redo stack), so the top of each stack says how
  // recent this doc's next undo / redo is relative to the other docs'.
  type UndoEntry = { steps: number; notes: NotesStep[]; at: number };
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
  const closingListeners = new Set<(id: string, doneAt: number) => void>();
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
          docId,
          listId: ev.listId ?? "",
          text: ev.text ?? "",
          notes: ev.notes ?? "",
          state: parseWorkflowState(ev.state),
          lifecycleAt: Number(ev.lifecycleAt ?? ev.createdAt ?? 0),
          deadline: ev.deadline ?? undefined,
          when: ev.when ?? undefined,
          whenSuggested: ev.whenSuggested ?? undefined,
          duration: ev.duration != null ? Number(ev.duration) : undefined,
          place: parsePlaceJson(ev.place),
          createdAt: Number(ev.createdAt ?? 0),
          createdBy: ev.createdBy ?? undefined,
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
        const nowOpen = binnedAt == null && !isClosedState(nextState);
        if (wasOpen && !nowOpen && binnedAt == null) {
          // Leaving the Open projection by being closed (done or
          // cancelled): snapshot
          // the vacated position (before the removal below) for the
          // linger re-insert.
          const idx = state.listOpen[prev.listId]?.indexOf(ev.id) ?? -1;
          captureRecentDone({
            id: ev.id,
            listId: prev.listId,
            index: idx >= 0 ? idx : 0,
            doneAt: lifecycleAt,
          });
          // The item may also hold a Focus slot, which only the personal
          // layer can see; let it snapshot that while the row is in place.
          for (const cb of closingListeners) cb(ev.id, lifecycleAt);
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
      case "itemWhenSuggestedChanged": {
        if (state.itemsById[ev.id]) {
          setState("itemsById", ev.id, "whenSuggested", ev.whenSuggested ?? undefined);
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
      case "itemPlaceChanged": {
        if (state.itemsById[ev.id]) {
          setState("itemsById", ev.id, "place", parsePlaceJson(ev.place));
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
          inboxView: ev.inboxView ?? null,
          docName: ev.docName ?? null,
          calendarHiddenDocs: ev.calendarHiddenDocs ?? [],
        });
        break;
      }
    }
  };

  const drainEvents = (): void => {
    const events: AppEventJs[] = [];
    while (true) {
      const ev = replica.popAppEvent();
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
        const next = materializeReplicaSnapshot(replica, home);
        setState(reconcile(next));
        // Any per-item delta in the bucket is lost with it; subscribed
        // editors reload from the fresh state.
        for (const cb of notesListeners) cb("", null);
      } else {
        for (const ev of events) {
          dispatch(ev);
        }
        // Home's Focus entries depend on both focus-container mutations
        // (`focusChanged`) and item lifecycle events elsewhere (Done
        // auto-removes a local ref, a sweep drops a binned one). One
        // wholesale re-derive per non-empty drain is the cheapest correct
        // approach — `reconcile` no-ops when the list is unchanged. See
        // spec/focus.md B.8.
        if (home && events.length > 0) {
          setState("focusEntries", reconcile(readFocusEntries(replica)));
        }
      }
      if (events.length > 0) setVersion((v) => v + 1);
    });
  };

  // Initial attach is explicit materialization, not a live event replay.
  // A single compact JSON snapshot crosses the wasm boundary and the
  // historical event queue remains empty.
  const initialState = materializeReplicaSnapshot(replica, home);
  setState(reconcile(initialState));

  let onFlush: () => void = () => {};
  let beforeFlush: () => void = () => {};
  const flush = (): void => {
    if (actionBatchDepth > 0) {
      flushDeferred = true;
      return;
    }
    beforeFlush();
    replica.flush();
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
    undoStack.push({ steps, notes, at: nextActionStamp() });
    redoStack.length = 0;
  };

  // A hard delete (deleteBinned / emptyBin) is permanent: the core
  // rearms its UndoManager on that commit, and this doc's stacks are
  // dropped to match, so ⌘Z cannot walk back to before the delete. An
  // enclosing action batch restarts from here, so only steps after the
  // delete land on the stack when it closes.
  const forgetUndoHistory = (): void => {
    undoStack.length = 0;
    redoStack.length = 0;
    if (actionBatchDepth > 0) {
      pendingActionSteps = 0;
      pendingBatchNotes = [];
      actionBatchStartVersion = version();
    }
  };

  // Run a hard delete: commit, flush, then forget the undo history
  // (only if something was actually removed).
  const hardDelete = <T>(fn: () => T): T => {
    const before = version();
    const result = fn();
    flush();
    if (version() !== before) forgetUndoHistory();
    return result;
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
      replica.applyNotesDelta(id, JSON.stringify(ops));
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

  // A display-setting write (saved default view, calendar
  // hidden docs). The core commits these under its
  // undo-excluded `view` origin, so no step is counted and no entry
  // recorded: undo after a view change reverses the last edit instead.
  const mutateView = (fn: () => void): void => {
    fn();
    flush();
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
        if (!replica.undo()) break;
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
      redoStack.push({ steps: applied, notes, at: nextActionStamp() });
      return true;
    }
  };

  const redoStep = (): boolean => {
    for (;;) {
      const entry = redoStack.pop();
      if (entry == null) return false;
      let applied = 0;
      for (let i = 0; i < entry.steps; i++) {
        if (!replica.redo()) break;
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
      undoStack.push({ steps: applied, notes, at: nextActionStamp() });
      return true;
    }
  };

  return {
    replica,
    docId,
    state,
    version,
    recentDone,
    search,
    placeUsesJson() {
      return replica.placeUsesJson();
    },
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
    onItemClosing(cb) {
      closingListeners.add(cb);
      return () => {
        closingListeners.delete(cb);
      };
    },
    seedWelcome() {
      return mutate(() => replica.seedWelcome());
    },
    addItem(listId, text) {
      return mutate(() => replica.addItem(listId, text));
    },
    addItemAt(listId, text, indexInList) {
      return mutate(() => replica.addItemAt(listId, text, indexInList));
    },
    addItemsAt(listId, texts, indexInList) {
      return mutate(() => replica.addItemsAt(listId, texts, indexInList));
    },
    editItemText(id, text) {
      mutate(() => replica.editItemText(id, text));
    },
    setItemNotes(id, notes) {
      const before = state.itemsById[id]?.notes ?? "";
      const ops = diffToDelta(before, notes);
      if (!ops) return;
      replica.applyNotesDelta(id, JSON.stringify(ops));
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
      const text = replica.subscribeNotes(id);
      notesSession = { id, base: text };
      return text;
    },
    unsubscribeNotes(id) {
      endNotesSession();
      if (notesSession?.id === id) notesSession = null;
      replica.unsubscribeNotes(id);
    },
    applyNotesDelta(id, ops) {
      if (ops.length === 0) return;
      // Commits into the doc now (the store sees the new string on this
      // drain; core's search index tracks the doc directly) without
      // recording a workspace undo step; the durable capture + push
      // waits for the idle timer.
      replica.applyNotesDelta(id, JSON.stringify(ops));
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
      mutate(() => replica.setItemDeadline(id, deadline ?? undefined));
    },
    setItemWhen(id, when) {
      mutate(() => replica.setItemWhen(id, when ?? undefined));
    },
    setItemWhenSuggested(id, when) {
      // A write that changes nothing makes no commit; skipped here so it
      // is not counted as an undo step inside a batch.
      if ((state.itemsById[id]?.whenSuggested ?? null) === when) return;
      mutate(() => replica.setItemWhenSuggested(id, when ?? undefined));
    },
    acceptItemWhenSuggestion(id) {
      if (!state.itemsById[id]?.whenSuggested) return;
      mutate(() => replica.acceptItemWhenSuggestion(id));
    },
    setItemDuration(id, minutes) {
      // The core rejects zero, negative, and over-a-year values; refuse
      // anything that is not a positive whole number here so a bad value
      // never reaches the replica (where it would throw).
      if (minutes != null && (!Number.isInteger(minutes) || minutes <= 0)) return;
      mutate(() => replica.setItemDuration(id, minutes ?? undefined));
    },
    setItemPlace(id, place) {
      // `JSON.stringify` drops `undefined` members, so a label-only place
      // crosses as `{"label":...}`; the core canonicalises the rest.
      mutate(() => replica.setItemPlace(id, place ? JSON.stringify(place) : undefined));
    },
    setDone(id, done) {
      mutate(() => replica.setItemDone(id, done));
    },
    setDoneMany(ids, done) {
      mutate(() => replica.setItemsDone(ids, done));
    },
    setBinned(id, binned) {
      mutate(() => replica.setItemBinned(id, binned));
    },
    setBinnedMany(ids, binned) {
      mutate(() => replica.setItemsBinned(ids, binned));
    },
    setLifecycle(id, lifecycle) {
      mutate(() => replica.setItemLifecycle(id, lifecycleEnum(lifecycle)));
    },
    setLifecycleMany(ids, lifecycle) {
      mutate(() => replica.setItemsLifecycle(ids, lifecycleEnum(lifecycle)));
    },
    addItemInState(listId, text, state) {
      return mutate(() => replica.addItemInState(listId, text, lifecycleEnum(state)));
    },
    addItemInStateAt(listId, text, state, indexInList) {
      return mutate(() =>
        replica.addItemInStateAt(listId, text, lifecycleEnum(state), indexInList),
      );
    },
    moveItem(id, listId, indexInList) {
      mutate(() => replica.moveItem(id, listId, indexInList));
    },
    deleteBinned(id) {
      hardDelete(() => replica.deleteBinned(id));
    },
    deleteBinnedMany(ids) {
      hardDelete(() => replica.deleteBinnedItems(ids));
    },
    emptyBin() {
      return hardDelete(() => replica.emptyBin());
    },
    exportItemsJson(ids) {
      return replica.exportItemsJson(ids);
    },
    adoptItems(itemsJson, listId, indexInList) {
      mutate(() => replica.adoptItems(itemsJson, listId, indexInList));
    },
    releaseItems(ids) {
      mutate(() => replica.releaseItems(ids));
    },
    forgetUndoHistory() {
      replica.forgetUndo();
      forgetUndoHistory();
    },
    addList(name) {
      return mutate(() => replica.addList(name));
    },
    renameList(id, name) {
      mutate(() => replica.renameList(id, name));
    },
    setListIcon(id, icon) {
      mutate(() => replica.setListIcon(id, icon));
    },
    setDefaultView(id, view) {
      mutateView(() => replica.setDefaultView(id, view ?? ""));
    },
    importJson(json) {
      return mutate(() => {
        const summaryJson = replica.importJson(json);
        return JSON.parse(summaryJson) as ImportSummary;
      });
    },
    moveList(id, index) {
      mutate(() => replica.moveList(id, index));
    },
    setListArchived(id, archived) {
      mutate(() => replica.setListArchived(id, archived));
    },
    addToFocus(id, index) {
      // Default to the top: newly-focused items are "what am I working on
      // now" and belong at the head of the lens. An explicit index (e.g.
      // drag-to-reorder) still wins.
      mutate(() => replica.addToFocus(id, index ?? 0));
    },
    addToFocusMany(ids) {
      mutate(() => replica.addToFocusMany(ids));
    },
    removeFromFocus(id) {
      mutate(() => replica.removeFromFocus(id));
    },
    removeFromFocusMany(ids) {
      mutate(() => replica.removeFromFocusMany(ids));
    },
    moveInFocus(id, index) {
      mutate(() => replica.moveInFocus(id, index));
    },
    addForeignToFocus(docId, itemId, index) {
      mutate(() => replica.addForeignToFocus(docId, itemId, index));
    },
    retargetFocus(ids, docId) {
      mutate(() => replica.retargetFocus(ids, docId ?? undefined));
    },
    setDocName(name) {
      mutate(() => replica.setDocName(name));
    },
    setCalendarDocHidden(docId, hidden) {
      mutateView(() => replica.setCalendarDocHidden(docId, hidden));
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
    topUndoStamp() {
      return undoStack[undoStack.length - 1]?.at ?? 0;
    },
    topRedoStamp() {
      return redoStack[redoStack.length - 1]?.at ?? 0;
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
