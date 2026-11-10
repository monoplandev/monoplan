// The personal layer above the per-doc apps (`spec/sharing-plan.md`
// "Decisions", 2026-10-20: one `DocApp` per doc plus a thin layer, not a
// merged store). Everything under a `DocApp` is per doc and cannot
// merge — core `Doc`, UndoManager, search index, notes bridge, event
// dispatcher, storage, replica — so this layer holds only what spans
// docs:
//
//   - the doc map (Home first, then boot / creation order; reactive, so
//     a doc created mid-session appears in the nav without a reload),
//   - the `itemId → docId` reverse index (item ids are uuid-v7, globally
//     unique, so a lookup resolves by probing each doc's store; N is the
//     handful of docs an account can open),
//   - mutations routed by `ItemView.docId` (`routed`, an `ItemActions`):
//     a multi-id call is grouped by doc and committed once per doc,
//   - the merged read views the cross-doc lenses render from: the
//     calendar (every doc, minus the ones hidden by a personal pref),
//     Focus (the Home doc's entries, foreign refs resolved against their
//     docs, `spec/focus.md`), Done (Home alone, or every doc when the
//     personal amalgamate pref is on) and the Bin (always every doc:
//     a binned row must stay reachable to restore or delete), and the
//     known places a place field suggests (always every doc: they are
//     the account's, `spec/place-plan.md` "Reuse").
//
// Focus is personal and lives in the Home doc's `focus` container:
// there is no shared Focus, and a shared doc's `DocApp` knows nothing
// of it. That is the one asymmetry between Home and the other docs
// (`HomeApp` vs `DocApp`). Home's Focus takes items of any open doc,
// so everything the lens *means* is here, not in a store: resolving
// entries to rows, `isFocused`, the done-linger capture
// (`focusRecentDone`) and un-done's re-pin (`undoneIntoFocus`), all in
// the one merged order the lens shows. A foreign ref is
// `<doc_id>:<item_id>`; the core keeps every one through
// every sweep and leaves judging them dead to this layer, which prunes
// a ref — by item id — once it *observes* the item closed or binned in
// its doc. A ref whose item is missing is never pruned here: the doc
// may simply not have synced yet.
//
// Personal cross-doc preferences (calendar toggles, Done amalgamation)
// live in the Home doc's `settings` map, which syncs across the
// account's devices and is never shared.
//
// Undo stays per doc, interleaved chronologically: every entry on a
// doc's undo / redo stack carries a process-wide stamp, and `undo()`
// goes to the doc whose top entry is newest, `redo()` to the doc whose
// top redo entry is newest. Across docs that reads as one history; a
// routed call that spans docs is still undone one doc at a time.
//
// The one action that is not per doc is a move between docs
// (`moveToDoc`, `spec/sharing-plan.md` "Cross-doc move"): the target
// adopts the items under their own ids, Home's Focus entries follow,
// the source releases them. That is one undo entry per doc involved,
// and this layer links them so one undo (or redo) applies them all. If
// one half's history is gone by then (a hard delete, a closed doc) the
// other halves' histories are dropped rather than undone alone, which
// would lose or duplicate the items.
//
// The merged reads are plain functions over the docs' Solid stores, not
// memos: a caller's own memo tracks exactly the store paths they touch,
// and they stay honest under bun's test resolution of Solid (the server
// build, where memos compute once). The one reactive computation here
// is the foreign-ref prune effect, which the tests drive directly.

import { placeSuggestionsJson } from "@monoplan/core/wasm";
import { createEffect, createSignal, type Accessor } from "solid-js";
import { restoreCapturedPositions } from "../linger.ts";
import type { PlaceSuggestion } from "../placeSuggest.ts";
import type { ViewKey } from "../prefs.ts";
import {
  isBinned,
  isClosed,
  isOpen,
  type DocApp,
  type HomeApp,
  type ItemActions,
  type ItemView,
  type Lifecycle,
  type ListView,
  type UndoOutcome,
} from "./store.ts";

export interface DocEntry {
  docId: string;
  app: DocApp;
  /** The account's Home doc: labelled "Personal", never named, never
   *  closable; owns Focus and the personal prefs. */
  home: boolean;
}

/** Snapshot of the Focus slot an item held at the moment it was closed
 *  (done or cancelled), in whichever doc it lives. The Focus lens's
 *  twin of a store's `RecentDoneEntry`: closing drops the ref, so this
 *  is the only record of where the row sat. */
export interface FocusDoneEntry {
  id: string;
  /** Visible slot in the merged lens (`focusItems`) just before the
   *  row left it, earlier captures already gone. */
  index: number;
  /** The closing transition's register timestamp (`lifecycleAt`). */
  doneAt: number;
}

export interface DocsLayer {
  readonly homeDocId: string;
  readonly home: HomeApp;
  /** Every open doc, Home first. */
  docs: Accessor<DocEntry[]>;
  /** Register a doc opened after boot (`createDoc` in the runtime). */
  addDoc(app: DocApp): void;
  /** Forget a non-Home doc (`deleteDoc` in the runtime): it leaves the
   *  map and its Focus pins are dropped from Home. Home is never
   *  removed. */
  removeDoc(docId: string): void;
  appOf(docId: string): DocApp | undefined;
  /** The doc holding `itemId`, or undefined when no open doc has it. */
  docIdOf(itemId: string): string | undefined;
  appForItem(itemId: string): DocApp | undefined;
  /** Merged `getItem`: the item from whichever doc holds it. */
  getItem(itemId: string): ItemView | undefined;
  /** The doc holding a `ListMeta` row with this id (user list ids are
   *  unique across docs; the reserved `inbox` / `events` resolve to
   *  Home). */
  docIdOfList(listId: string): string | undefined;
  listOf(docId: string, listId: string): ListView | undefined;
  /** The doc a view is scoped to: a list view's doc (Home when the key
   *  carries none), Home for the cross-doc lenses. */
  viewDocId(v: ViewKey): string;
  appForView(v: ViewKey): DocApp;
  /** Canonical list view key: `docId` is omitted for Home so equality,
   *  prefs and URLs agree on one shape. */
  listView(docId: string, listId: string): ViewKey;
  /** Bumps when any doc's `version` does. */
  version(): number;
  // ---- merged reads
  /** Every item of every open doc. */
  allItems(): ItemView[];
  /** The calendar's input: every doc not hidden by the personal pref. */
  calendarItems(): ItemView[];
  /** Doc ids the calendar hides (the Home doc's pref). */
  calendarHiddenDocs(): string[];
  /** The Focus lens: Home's entries in curated order, each resolved to
   *  its doc's item, non-Open and unresolvable ones left out. */
  focusItems(): ItemView[];
  /** Whether `itemId` has a live Focus entry (local or foreign). */
  isFocused(itemId: string): boolean;
  /** Entry index (the space `addToFocus` / `moveInFocus` address) of a
   *  visible Focus slot; past-end slots map past the last entry. */
  focusEntryIndex(visibleIndex: number): number;
  /** Rolling capture of rows that just left the Focus lens by being
   *  closed, Home's and foreign alike, in capture order. Feeds the
   *  lens's linger overlay, which applies its own (shorter) window. */
  focusRecentDone: Accessor<readonly FocusDoneEntry[]>;
  /** The Done view: closed, not-binned items of Home, or of every doc
   *  when amalgamated, newest transition first. */
  doneItems(): ItemView[];
  doneAmalgamate(): boolean;
  /** The account's known places (`spec/place-plan.md` "Reuse"): every
   *  open doc's uses merged by core, deduped and ranked newest use
   *  first, so a place used in one workspace is offered in another.
   *  `query` narrows by label or address prefix, `limit` 0 means all.
   *  A scan per call: call it on demand, not reactively. */
  placeSuggestions(query?: string, limit?: number): PlaceSuggestion[];
  /** The Bin: every doc's binned items, newest first. */
  binnedItems(): ItemView[];
  binCount(): number;
  /** Drop every foreign Focus ref whose item is closed or binned in its
   *  doc. Runs on its own from an effect; exposed for tests. */
  pruneDeadForeignRefs(): void;
  // ---- mutations
  routed: ItemActions;
  /** Move items into a list of another doc, as a block at `indexInList`
   *  of its Open projection, in the given order (grouped by source doc
   *  when they come from several). Ids are preserved; binned rows land
   *  restored, closed rows stay closed; a starred row keeps its Focus
   *  slot. Ids already in the target doc, or in no open doc, are
   *  skipped. One undo step across the docs involved. Returns the ids
   *  that moved. */
  moveToDoc(
    ids: readonly string[],
    targetDocId: string,
    listId: string,
    indexInList: number,
  ): string[];
  /** Empty every doc's bin. Returns the number of items removed. */
  emptyBin(): number;
  undo(): boolean;
  redo(): boolean;
  canUndo(): boolean;
  canRedo(): boolean;
  onUndoRedo(cb: (outcome: UndoOutcome, docId: string) => void): () => void;
}

export function createDocsLayer(home: HomeApp, initial: readonly DocApp[]): DocsLayer {
  const homeDocId = home.docId;
  const seed: DocEntry[] = [{ docId: homeDocId, app: home, home: true }];
  for (const app of initial) {
    if (app.docId === homeDocId || seed.some((d) => d.docId === app.docId)) continue;
    seed.push({ docId: app.docId, app, home: false });
  }
  const [docs, setDocs] = createSignal<DocEntry[]>(seed);
  // Non-reactive lookup for the hot paths (routing a mutation); the
  // signal above is what the UI iterates.
  const byId = new Map<string, DocApp>(seed.map((d) => [d.docId, d.app]));

  const addDoc = (app: DocApp): void => {
    if (byId.has(app.docId)) return;
    byId.set(app.docId, app);
    const entry: DocEntry = { docId: app.docId, app, home: false };
    setDocs((prev) => [...prev, entry]);
    subscribeDoc(entry);
  };
  const removeDoc = (docId: string): void => {
    if (docId === homeDocId || !byId.has(docId)) return;
    // Its items are gone everywhere, so its foreign Focus pins are dead
    // for good; the prune effect would never see them resolve again.
    const pinned = home.state.focusEntries
      .filter((e) => e.docId === docId)
      .map((e) => e.itemId);
    if (pinned.length > 0) home.removeFromFocusMany(pinned);
    docOffs.get(docId)?.();
    docOffs.delete(docId);
    const gone = byId.get(docId);
    if (gone) breakLinksOf(gone);
    byId.delete(docId);
    setDocs((prev) => prev.filter((d) => d.docId !== docId));
  };
  const appOf = (docId: string): DocApp | undefined => byId.get(docId);

  // Reverse index by probing: every doc's `itemsById` is a Solid store
  // path, so a reactive caller tracks exactly the entries it touched.
  const docIdOf = (itemId: string): string | undefined => {
    if (home.state.itemsById[itemId]) return homeDocId;
    for (const d of docs()) {
      if (!d.home && d.app.state.itemsById[itemId]) return d.docId;
    }
    return undefined;
  };
  const appForItem = (itemId: string): DocApp | undefined => {
    const id = docIdOf(itemId);
    return id === undefined ? undefined : byId.get(id);
  };
  const getItem = (itemId: string): ItemView | undefined => {
    const it = home.state.itemsById[itemId];
    if (it) return it;
    for (const d of docs()) {
      if (d.home) continue;
      const found = d.app.state.itemsById[itemId];
      if (found) return found;
    }
    return undefined;
  };
  const docIdOfList = (listId: string): string | undefined => {
    if (listId === "inbox" || listId === "events") return homeDocId;
    for (const d of docs()) {
      if (d.app.state.listsById[listId]) return d.docId;
    }
    return undefined;
  };
  const listOf = (docId: string, listId: string): ListView | undefined =>
    byId.get(docId)?.state.listsById[listId];
  const viewDocId = (v: ViewKey): string =>
    v.kind === "list" && v.docId !== undefined && byId.has(v.docId) ? v.docId : homeDocId;
  const appForView = (v: ViewKey): DocApp => byId.get(viewDocId(v)) ?? home;
  const listView = (docId: string, listId: string): ViewKey =>
    docId === homeDocId
      ? { kind: "list", id: listId }
      : { kind: "list", id: listId, docId };

  const version = (): number => {
    let n = 0;
    for (const d of docs()) n += d.app.version();
    return n;
  };

  // ---- merged reads
  const allItems = (): ItemView[] => {
    const out: ItemView[] = [];
    for (const d of docs()) {
      for (const it of Object.values(d.app.state.itemsById)) out.push(it);
    }
    return out;
  };
  const placeSuggestions = (query = "", limit = 0): PlaceSuggestion[] =>
    JSON.parse(
      placeSuggestionsJson(
        docs().map((d) => d.app.placeUsesJson()),
        query,
        limit,
      ),
    ) as PlaceSuggestion[];
  const calendarHiddenDocs = (): string[] => home.state.settings.calendarHiddenDocs;
  const calendarItems = (): ItemView[] => {
    const hidden = new Set(calendarHiddenDocs());
    if (hidden.size === 0) return allItems();
    const out: ItemView[] = [];
    for (const d of docs()) {
      if (hidden.has(d.docId)) continue;
      for (const it of Object.values(d.app.state.itemsById)) out.push(it);
    }
    return out;
  };

  // Focus: Home's entry list resolved across docs. An entry's item comes
  // from the named doc (null = Home); it shows while Open. Dedup by item
  // id, first wins, mirroring the core's own rule.
  const focusResolved = (): { entryIndex: number; item: ItemView }[] => {
    const out: { entryIndex: number; item: ItemView }[] = [];
    const seen = new Set<string>();
    home.state.focusEntries.forEach((e, entryIndex) => {
      if (seen.has(e.itemId)) return;
      const app = e.docId === null ? home : byId.get(e.docId);
      const it = app?.state.itemsById[e.itemId];
      if (!it || !isOpen(it)) return;
      seen.add(e.itemId);
      out.push({ entryIndex, item: it });
    });
    return out;
  };
  const focusItems = (): ItemView[] => focusResolved().map((r) => r.item);

  // Done-linger capture for the lens. Every doc's store announces an
  // item about to close (`onItemClosing`, before its state changes), so
  // the row is still in `focusResolved()` and its index is its slot in
  // the merged order, foreign rows counted. Later closes in the same
  // batch see the earlier ones already gone, giving the sequential
  // indices `restoreCapturedPositions` replays. Entries older than the
  // TTL are unreachable by any linger chain and pruned on write.
  const FOCUS_RECENT_DONE_TTL_MS = 15_000;
  const [focusRecentDone, setFocusRecentDone] = createSignal<readonly FocusDoneEntry[]>(
    [],
  );
  const captureFocusDone = (id: string, doneAt: number): void => {
    const index = focusResolved().findIndex((r) => r.item.id === id);
    setFocusRecentDone((prev) => {
      const kept = prev.filter(
        (e) => e.id !== id && doneAt - e.doneAt < FOCUS_RECENT_DONE_TTL_MS,
      );
      if (index < 0) return kept.length === prev.length ? prev : kept;
      return [...kept, { id, index, doneAt }];
    });
  };
  const isFocused = (itemId: string): boolean =>
    home.state.focusEntries.some((e) => e.itemId === itemId);
  const focusEntryIndex = (visibleIndex: number): number => {
    const rows = focusResolved();
    if (visibleIndex < rows.length) return rows[visibleIndex]!.entryIndex;
    return home.state.focusEntries.length;
  };

  // Prune a foreign ref once its item is observed closed or binned in
  // its doc (the core cannot judge foreign refs; `spec/focus.md`). Never
  // on a missing item — the doc may not have synced yet. Runs through
  // the normal mutation path, so it is one Home undo step: undoing the
  // close in the other doc does not restore the pin, exactly as Done
  // auto-removal works for a local item.
  const pruneDeadForeignRefs = (): void => {
    const dead: string[] = [];
    for (const e of home.state.focusEntries) {
      if (e.docId === null) continue;
      const it = byId.get(e.docId)?.state.itemsById[e.itemId];
      if (it && (isClosed(it) || isBinned(it))) dead.push(e.itemId);
    }
    if (dead.length > 0) home.removeFromFocusMany(dead);
  };
  createEffect(pruneDeadForeignRefs);

  const doneAmalgamate = (): boolean => home.state.settings.doneAmalgamate;
  const doneItems = (): ItemView[] => {
    const source = doneAmalgamate() ? allItems() : Object.values(home.state.itemsById);
    return source
      .filter((it) => isClosed(it) && !isBinned(it))
      .sort((a, b) => b.lifecycleAt - a.lifecycleAt);
  };
  const binnedItems = (): ItemView[] =>
    allItems()
      .filter(isBinned)
      .sort((a, b) => (b.binnedAt ?? 0) - (a.binnedAt ?? 0));
  const binCount = (): number => {
    let n = 0;
    for (const d of docs()) n += d.app.state.binCount;
    return n;
  };

  // ---- routing
  /** `ids` grouped by owning doc, in first-seen doc order; ids no open
   *  doc holds are dropped (a stale selection key). */
  const groupByDoc = (ids: readonly string[]): [DocApp, string[]][] => {
    const groups = new Map<string, string[]>();
    for (const id of ids) {
      const docId = docIdOf(id);
      if (docId === undefined) continue;
      let g = groups.get(docId);
      if (!g) {
        g = [];
        groups.set(docId, g);
      }
      g.push(id);
    }
    return [...groups].map(([docId, group]) => [byId.get(docId)!, group]);
  };
  const forItem = (id: string, fn: (app: DocApp) => void): void => {
    const app = appForItem(id);
    if (app) fn(app);
  };
  // Closing or binning a foreign item in its doc also drops its Focus
  // ref right away (the observe effect above would catch it one tick
  // later; doing it here keeps the user's action and the prune in the
  // same breath).
  const pruneForeignFocus = (ids: readonly string[]): void => {
    const foreign = ids.filter((id) =>
      home.state.focusEntries.some((e) => e.docId !== null && e.itemId === id),
    );
    if (foreign.length > 0) home.removeFromFocusMany(foreign);
  };
  const closing = (lifecycle: Lifecycle): boolean =>
    lifecycle === "done" || lifecycle === "cancelled" || lifecycle === "binned";

  const routed: ItemActions = {
    getItem,
    isFocused,
    editItemText(id, text) {
      forItem(id, (app) => app.editItemText(id, text));
    },
    setItemNotes(id, notes) {
      forItem(id, (app) => app.setItemNotes(id, notes));
    },
    setItemDeadline(id, deadline) {
      forItem(id, (app) => app.setItemDeadline(id, deadline));
    },
    setItemWhen(id, when) {
      forItem(id, (app) => app.setItemWhen(id, when));
    },
    setItemDuration(id, minutes) {
      forItem(id, (app) => app.setItemDuration(id, minutes));
    },
    setItemPlace(id, place) {
      forItem(id, (app) => app.setItemPlace(id, place));
    },
    setDone(id, done) {
      forItem(id, (app) => app.setDone(id, done));
      if (done) pruneForeignFocus([id]);
    },
    setDoneMany(ids, done) {
      for (const [app, group] of groupByDoc(ids)) app.setDoneMany(group, done);
      if (done) pruneForeignFocus(ids);
    },
    undoneIntoFocus(ids) {
      if (ids.length === 0) return;
      // Rebuild the lens as it stood before the closes: the live rows
      // with the captured ones put back at their slots (sequential
      // captures, replayed newest-first as the linger overlay does),
      // ids without a capture on top. Each returning row then goes in
      // just before the live row that follows it, in entry positions
      // (the core's index space; an entry the lens cannot show still
      // occupies one).
      const wanted = new Set(ids);
      const layout: Array<{ id: string; entryIndex?: number }> = focusResolved().map(
        (r) => ({ id: r.item.id, entryIndex: r.entryIndex }),
      );
      const live = new Set(layout.map((r) => r.id));
      const captured = focusRecentDone().filter((e) => wanted.has(e.id) && !live.has(e.id));
      restoreCapturedPositions(
        layout,
        captured.map((e) => ({ index: e.index, value: { id: e.id } })),
      );
      const placed = new Set(captured.map((e) => e.id));
      const fresh = ids.filter((id, i) => !placed.has(id) && !live.has(id) && ids.indexOf(id) === i);
      layout.unshift(...fresh.map((id) => ({ id })));
      // Bottom-up, so an insert never shifts a position still to be used.
      const pins: Array<{ id: string; at: number }> = [];
      let at = home.state.focusEntries.length;
      for (let i = layout.length - 1; i >= 0; i--) {
        const row = layout[i]!;
        if (row.entryIndex !== undefined) at = row.entryIndex;
        else pins.push({ id: row.id, at });
      }
      setFocusRecentDone((prev) => prev.filter((e) => !wanted.has(e.id)));
      routed.withActionBatch(() => {
        for (const [app, group] of groupByDoc(ids)) app.setDoneMany(group, false);
        for (const pin of pins) {
          const app = appForItem(pin.id);
          if (!app) continue;
          if (app === home) home.addToFocus(pin.id, pin.at);
          else home.addForeignToFocus(app.docId, pin.id, pin.at);
        }
      });
    },
    setBinned(id, binned) {
      forItem(id, (app) => app.setBinned(id, binned));
      if (binned) pruneForeignFocus([id]);
    },
    setBinnedMany(ids, binned) {
      for (const [app, group] of groupByDoc(ids)) app.setBinnedMany(group, binned);
      if (binned) pruneForeignFocus(ids);
    },
    setLifecycle(id, lifecycle) {
      forItem(id, (app) => app.setLifecycle(id, lifecycle));
      if (closing(lifecycle)) pruneForeignFocus([id]);
    },
    setLifecycleMany(ids, lifecycle) {
      for (const [app, group] of groupByDoc(ids)) app.setLifecycleMany(group, lifecycle);
      if (closing(lifecycle)) pruneForeignFocus(ids);
    },
    moveItem(id, listId, indexInList) {
      forItem(id, (app) => {
        const known =
          listId === "inbox" || listId === "events" || app.state.listsById[listId];
        if (known) app.moveItem(id, listId, indexInList);
      });
    },
    deleteBinned(id) {
      forItem(id, (app) => app.deleteBinned(id));
      pruneForeignFocus([id]);
    },
    deleteBinnedMany(ids) {
      for (const [app, group] of groupByDoc(ids)) app.deleteBinnedMany(group);
      pruneForeignFocus(ids);
    },
    addToFocus(id, index) {
      const app = appForItem(id);
      if (!app) return;
      if (app === home) {
        home.addToFocus(id, index);
        return;
      }
      const it = app.state.itemsById[id];
      if (!it || !isOpen(it) || isFocused(id)) return;
      home.addForeignToFocus(app.docId, id, index ?? 0);
    },
    addToFocusMany(ids) {
      // Prepend each in reverse so the batch ends up on top in the given
      // order, as the core's own batch add does; the core skips an id
      // already focused, local or foreign. One Home undo step.
      home.withActionBatch(() => {
        for (let i = ids.length - 1; i >= 0; i--) {
          const id = ids[i]!;
          const app = appForItem(id);
          if (!app) continue;
          const it = app.state.itemsById[id];
          if (!it || !isOpen(it)) continue;
          if (app === home) home.addToFocus(id, 0);
          else home.addForeignToFocus(app.docId, id, 0);
        }
      });
    },
    removeFromFocus(id) {
      home.removeFromFocus(id);
    },
    removeFromFocusMany(ids) {
      home.removeFromFocusMany(ids);
    },
    moveInFocus(id, index) {
      home.moveInFocus(id, index);
    },
    withActionBatch(fn) {
      // Open a batch on every doc so whichever ones the body touches
      // record one step each; an untouched doc records nothing.
      const run = (i: number): ReturnType<typeof fn> => {
        const d = docs()[i];
        if (!d) return fn();
        return d.app.withActionBatch(() => run(i + 1));
      };
      return run(0);
    },
  };

  const emptyBin = (): number => {
    let n = 0;
    for (const d of docs()) n += d.app.emptyBin();
    return n;
  };

  // ---- cross-doc move
  // A move's undo entries, one per doc it touched, in the order they
  // are applied: target first, source last. Adopt-before-release keeps
  // a redo on the safe side (a duplicate, never a loss), and the giving
  // doc reporting last is what lands the user on the rows after an undo.
  type MoveLink = {
    members: { app: DocApp; stamp: number }[];
    /** A doc of this move is gone: what is left must not be undone. */
    broken: boolean;
  };
  const undoLinks: MoveLink[] = [];
  const redoLinks: MoveLink[] = [];
  const breakLinksOf = (app: DocApp): void => {
    for (const link of [...undoLinks, ...redoLinks]) {
      const before = link.members.length;
      link.members = link.members.filter((mb) => mb.app !== app);
      if (link.members.length !== before) link.broken = true;
    }
  };
  const inBatches = (apps: readonly DocApp[], fn: () => void): void => {
    const run = (i: number): void => {
      const app = apps[i];
      if (!app) fn();
      else app.withActionBatch(() => run(i + 1));
    };
    run(0);
  };

  const moveToDoc = (
    ids: readonly string[],
    targetDocId: string,
    listId: string,
    indexInList: number,
  ): string[] => {
    const target = byId.get(targetDocId);
    if (!target) return [];
    const known =
      listId === "inbox" || listId === "events" || target.state.listsById[listId];
    if (!known) return [];
    // An id the target already holds is either not a cross-doc move or
    // a twin left by two devices moving one item apart; neither moves.
    const groups = groupByDoc([...new Set(ids)])
      .filter(([app]) => app !== target)
      .map(([app, group]): [DocApp, string[]] => [
        app,
        group.filter((id) => !target.state.itemsById[id]),
      ])
      .filter(([, group]) => group.length > 0);
    if (groups.length === 0) return [];
    const toDocId = target === home ? null : target.docId;
    const starred = (group: readonly string[]): string[] =>
      group.filter((id) =>
        home.state.focusEntries.some((e) => e.itemId === id && e.docId !== toDocId),
      );
    const involved: DocApp[] = [target];
    for (const app of [home, ...groups.map(([app]) => app)]) {
      if (!involved.includes(app)) involved.push(app);
    }
    const stampsBefore = involved.map((app) => app.topUndoStamp());
    const moved: string[] = [];
    inBatches(involved, () => {
      let at = indexInList;
      for (const [src, group] of groups) {
        // The stores do not drain inside a batch, so read the rows now.
        const landsOpen = group.filter((id) => {
          const it = src.state.itemsById[id];
          return it !== undefined && !isClosed(it);
        }).length;
        const pins = starred(group);
        target.adoptItems(src.exportItemsJson(group), listId, at);
        // While a local ref still resolves: after Home adopts, before
        // Home releases. Skipped when nothing is starred, since a no-op
        // would still count as a step of the batch.
        if (pins.length > 0) home.retargetFocus(pins, toDocId);
        src.releaseItems(group);
        at += landsOpen;
        moved.push(...group);
      }
    });
    const members = involved
      .map((app, i) => ({ app, stamp: app.topUndoStamp(), was: stampsBefore[i] }))
      .filter((mb) => mb.stamp !== mb.was)
      .map(({ app, stamp }) => ({ app, stamp }));
    if (members.length > 1) undoLinks.push({ members, broken: false });
    return moved;
  };

  /** One undo / redo step for `app`, following a move's link when its
   *  top entry is half of one. */
  const stepLinked = (app: DocApp, dir: "undo" | "redo"): boolean => {
    const top = (a: DocApp): number => (dir === "undo" ? a.topUndoStamp() : a.topRedoStamp());
    const landed = (a: DocApp): number => (dir === "undo" ? a.topRedoStamp() : a.topUndoStamp());
    const from = dir === "undo" ? undoLinks : redoLinks;
    const at = from.findIndex((l) => l.members.some((mb) => mb.app === app && mb.stamp === top(app)));
    if (at < 0) return dir === "undo" ? app.undo() : app.redo();
    const link = from.splice(at, 1)[0]!;
    const onTop = link.members.filter((mb) => top(mb.app) === mb.stamp);
    if (link.broken || onTop.length !== link.members.length) {
      // Half the move can no longer be stepped. Stepping the rest would
      // lose the items (adopt undone, release not) or duplicate them,
      // so the move stands and those docs' histories go, as after a
      // hard delete.
      for (const mb of onTop) mb.app.forgetUndoHistory();
      return false;
    }
    const done: { app: DocApp; stamp: number }[] = [];
    for (const mb of link.members) {
      const did = dir === "undo" ? mb.app.undo() : mb.app.redo();
      if (did) done.push({ app: mb.app, stamp: landed(mb.app) });
    }
    if (done.length > 1) {
      (dir === "undo" ? redoLinks : undoLinks).push({ members: done, broken: false });
    }
    return done.length > 0;
  };

  // Undo goes to the doc whose next step is newest; redo likewise.
  const latest = (
    stamp: (app: DocApp) => number,
    can: (app: DocApp) => boolean,
  ): DocApp | null => {
    let best: DocApp | null = null;
    let bestAt = -1;
    for (const d of docs()) {
      if (!can(d.app)) continue;
      const at = stamp(d.app);
      if (at > bestAt) {
        bestAt = at;
        best = d.app;
      }
    }
    return best;
  };
  const undo = (): boolean => {
    const app = latest((a) => a.topUndoStamp(), (a) => a.canUndo());
    return app ? stepLinked(app, "undo") : false;
  };
  const redo = (): boolean => {
    const app = latest((a) => a.topRedoStamp(), (a) => a.canRedo());
    return app ? stepLinked(app, "redo") : false;
  };
  const canUndo = (): boolean => docs().some((d) => d.app.canUndo());
  const canRedo = (): boolean => docs().some((d) => d.app.canRedo());
  // Undo / redo listeners span every doc, including ones added later
  // (`addDoc` subscribes them as they arrive).
  const undoListeners = new Set<(outcome: UndoOutcome, docId: string) => void>();
  const docOffs = new Map<string, () => void>();
  const subscribeDoc = (d: DocEntry): void => {
    const offUndo = d.app.onUndoRedo((outcome) => {
      for (const cb of undoListeners) cb(outcome, d.docId);
    });
    const offClosing = d.app.onItemClosing(captureFocusDone);
    docOffs.set(d.docId, () => {
      offUndo();
      offClosing();
    });
  };
  for (const d of seed) subscribeDoc(d);
  const onUndoRedo = (cb: (outcome: UndoOutcome, docId: string) => void): (() => void) => {
    undoListeners.add(cb);
    return () => {
      undoListeners.delete(cb);
    };
  };

  return {
    homeDocId,
    home,
    docs,
    addDoc,
    removeDoc,
    appOf,
    docIdOf,
    appForItem,
    getItem,
    docIdOfList,
    listOf,
    viewDocId,
    appForView,
    listView,
    version,
    allItems,
    placeSuggestions,
    calendarItems,
    calendarHiddenDocs,
    focusItems,
    isFocused,
    focusEntryIndex,
    focusRecentDone,
    doneItems,
    doneAmalgamate,
    binnedItems,
    binCount,
    pruneDeadForeignRefs,
    routed,
    moveToDoc,
    emptyBin,
    undo,
    redo,
    canUndo,
    canRedo,
    onUndoRedo,
  };
}
