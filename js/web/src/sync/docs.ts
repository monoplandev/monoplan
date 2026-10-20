// The personal layer above the per-doc apps (`spec/sharing-plan.md`
// "Decisions", 2026-10-20: one `DocApp` per doc plus a thin layer, not a
// merged store). Everything under a `DocApp` is per doc and cannot
// merge — core `Doc`, UndoManager, search index, notes bridge, event
// dispatcher, storage, engine — so this layer holds only what spans
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
//     a binned row must stay reachable to restore or delete).
//
// Focus is personal and lives in the Home doc's `focus` container. A
// foreign ref is `<doc_id>:<item_id>`; the core keeps every one through
// every sweep and leaves judging them dead to this layer, which prunes
// a ref — by item id — once it *observes* the item closed or binned in
// its doc. A ref whose item is missing is never pruned here: the doc
// may simply not have synced yet.
//
// Personal cross-doc preferences (calendar toggles, Done amalgamation)
// live in the Home doc's `settings` map, which syncs across the
// account's devices and is never shared.
//
// Undo stays per doc. `undo()` goes to the doc that recorded a local
// action most recently; `redo()` to the one undone most recently. A
// routed call that spans docs therefore undoes one doc at a time.
//
// The merged reads are plain functions over the docs' Solid stores, not
// memos: a caller's own memo tracks exactly the store paths they touch,
// and they stay honest under bun's test resolution of Solid (the server
// build, where memos compute once). The one reactive computation here
// is the foreign-ref prune effect, which the tests drive directly.

import { createEffect, createSignal, type Accessor } from "solid-js";
import type { ViewKey } from "../prefs.ts";
import {
  isBinned,
  isClosed,
  isOpen,
  type DocApp,
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

export interface DocsLayer {
  readonly homeDocId: string;
  readonly home: DocApp;
  /** Every open doc, Home first. */
  docs: Accessor<DocEntry[]>;
  /** Register a doc opened after boot (`createDoc` in the runtime). */
  addDoc(app: DocApp): void;
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
  /** The Done view: closed, not-binned items of Home, or of every doc
   *  when amalgamated, newest transition first. */
  doneItems(): ItemView[];
  doneAmalgamate(): boolean;
  /** The Bin: every doc's binned items, newest first. */
  binnedItems(): ItemView[];
  binCount(): number;
  /** Drop every foreign Focus ref whose item is closed or binned in its
   *  doc. Runs on its own from an effect; exposed for tests. */
  pruneDeadForeignRefs(): void;
  // ---- mutations
  routed: ItemActions;
  /** Empty every doc's bin. Returns the number of items removed. */
  emptyBin(): number;
  undo(): boolean;
  redo(): boolean;
  canUndo(): boolean;
  canRedo(): boolean;
  onUndoRedo(cb: (outcome: UndoOutcome, docId: string) => void): () => void;
}

export function createDocsLayer(home: DocApp, initial: readonly DocApp[]): DocsLayer {
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
      // Home's own path re-pins at the captured slot; a foreign item is
      // reopened in its doc and pinned fresh at the top (its doc's
      // store captured no Focus slot).
      for (const [app, group] of groupByDoc(ids)) {
        if (app === home) {
          home.undoneIntoFocus(group);
          continue;
        }
        app.setDoneMany(group, false);
        home.withActionBatch(() => {
          for (const id of group) home.addForeignToFocus(app.docId, id, 0);
        });
      }
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

  // Undo goes to the doc that acted last; redo to the one undone last.
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
  const undo = (): boolean =>
    latest((a) => a.lastActionAt(), (a) => a.canUndo())?.undo() ?? false;
  const redo = (): boolean =>
    latest((a) => a.lastUndoAt(), (a) => a.canRedo())?.redo() ?? false;
  const canUndo = (): boolean => docs().some((d) => d.app.canUndo());
  const canRedo = (): boolean => docs().some((d) => d.app.canRedo());
  // Undo / redo listeners span every doc, including ones added later
  // (`addDoc` subscribes them as they arrive).
  const undoListeners = new Set<(outcome: UndoOutcome, docId: string) => void>();
  const docOffs: (() => void)[] = [];
  const subscribeDoc = (d: DocEntry): void => {
    docOffs.push(
      d.app.onUndoRedo((outcome) => {
        for (const cb of undoListeners) cb(outcome, d.docId);
      }),
    );
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
    calendarItems,
    calendarHiddenDocs,
    focusItems,
    isFocused,
    focusEntryIndex,
    doneItems,
    doneAmalgamate,
    binnedItems,
    binCount,
    pruneDeadForeignRefs,
    routed,
    emptyBin,
    undo,
    redo,
    canUndo,
    canRedo,
    onUndoRedo,
  };
}
