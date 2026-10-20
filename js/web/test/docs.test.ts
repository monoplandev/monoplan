// The cross-doc personal layer (`src/sync/docs.ts`): the reverse index,
// mutations routed by the item's doc, the merged lenses, foreign Focus
// refs (spec/focus.md §FocusRef grammar) and per-doc undo. Two in-memory
// engines stand in for Home and one other doc.

import { describe, expect, test } from "bun:test";
import { createRoot } from "solid-js";
import { Dek, Doc, SyncEngine } from "@monoplan/core/wasm";
import type { EngineStorage } from "@monoplan/core/wasm";
import { MemEngineStorage } from "../../core/test/mem-engine-storage.ts";
import { createSyncedApp, type DocApp } from "../src/sync/store.ts";
import { createDocsLayer, type DocsLayer } from "../src/sync/docs.ts";

const HOME = "00000000-0000-0000-0000-00000000000a";
const OTHER = "00000000-0000-0000-0000-00000000000b";

function appFor(docId: string): DocApp {
  return createSyncedApp(
    new SyncEngine(
      Doc.create(),
      docId,
      Dek.generate(),
      0n,
      new MemEngineStorage() as unknown as EngineStorage,
    ),
  );
}

function setup(): { home: DocApp; other: DocApp; layer: DocsLayer; dispose: () => void } {
  return createRoot((dispose) => {
    const home = appFor(HOME);
    const other = appFor(OTHER);
    const layer = createDocsLayer(home, [other]);
    return { home, other, layer, dispose };
  });
}

describe("docs layer", () => {
  test("doc map is Home first; the reverse index finds an item's doc", () => {
    const { home, other, layer, dispose } = setup();
    expect(layer.docs().map((d) => d.docId)).toEqual([HOME, OTHER]);
    expect(layer.docs()[0]!.home).toBe(true);
    const a = home.addItem("inbox", "home item");
    const b = other.addItem("inbox", "other item");
    expect(layer.docIdOf(a)).toBe(HOME);
    expect(layer.docIdOf(b)).toBe(OTHER);
    expect(layer.docIdOf("nope")).toBeUndefined();
    expect(layer.getItem(b)?.text).toBe("other item");
    expect(layer.getItem(b)?.docId).toBe(OTHER);
    // A user list resolves to its doc; the reserved lists to Home.
    const list = other.addList("Work");
    expect(layer.docIdOfList(list)).toBe(OTHER);
    expect(layer.docIdOfList("inbox")).toBe(HOME);
    // Canonical list views: Home carries no doc id.
    expect(layer.listView(HOME, "inbox")).toEqual({ kind: "list", id: "inbox" });
    expect(layer.listView(OTHER, list)).toEqual({ kind: "list", id: list, docId: OTHER });
    expect(layer.viewDocId({ kind: "list", id: list, docId: OTHER })).toBe(OTHER);
    expect(layer.viewDocId({ kind: "focus" })).toBe(HOME);
    dispose();
  });

  test("a multi-id mutation is routed to each item's doc", () => {
    const { home, other, layer, dispose } = setup();
    const a = home.addItem("inbox", "a");
    const b = other.addItem("inbox", "b");
    layer.routed.setDoneMany([a, b, "stale"], true);
    expect(home.getItem(a)?.state).toBe("done");
    expect(other.getItem(b)?.state).toBe("done");
    layer.routed.setBinnedMany([a, b], true);
    expect(layer.binnedItems().map((it) => it.id).sort()).toEqual([a, b].sort());
    expect(layer.binCount()).toBe(2);
    // A move stays inside the item's doc: a list the doc does not hold
    // is ignored.
    const c = other.addItem("inbox", "c");
    const homeList = home.addList("Home only");
    layer.routed.moveItem(c, homeList, 0);
    expect(other.getItem(c)?.listId).toBe("inbox");
    const otherList = other.addList("Other");
    layer.routed.moveItem(c, otherList, 0);
    expect(other.getItem(c)?.listId).toBe(otherList);
    dispose();
  });

  test("Focus resolves foreign refs and prunes them once the item closes", () => {
    const { home, other, layer, dispose } = setup();
    const a = home.addItem("inbox", "a");
    const b = other.addItem("inbox", "b");
    layer.routed.addToFocusMany([b, a]);
    // Home's entry list holds a foreign ref for b, a local one for a.
    expect(home.state.focusEntries).toEqual([
      { docId: OTHER, itemId: b },
      { docId: null, itemId: a },
    ]);
    expect(home.state.focusOrder).toEqual([a]);
    expect(layer.focusItems().map((it) => it.id)).toEqual([b, a]);
    expect(layer.isFocused(b)).toBe(true);
    expect(layer.routed.isFocused(b)).toBe(true);
    expect(layer.focusEntryIndex(1)).toBe(1);
    expect(layer.focusEntryIndex(5)).toBe(2);
    // Re-adding is a no-op.
    layer.routed.addToFocus(b);
    expect(home.state.focusEntries.length).toBe(2);
    // Closing b in its own doc (as a peer would) drops the ref once the
    // layer observes it (an effect in the app; driven by hand here).
    other.setDone(b, true);
    expect(home.state.focusEntries.length).toBe(2);
    layer.pruneDeadForeignRefs();
    expect(home.state.focusEntries).toEqual([{ docId: null, itemId: a }]);
    expect(layer.focusItems().map((it) => it.id)).toEqual([a]);
    // Un-done into Focus reopens the foreign item and pins it on top.
    layer.routed.undoneIntoFocus([b]);
    expect(other.getItem(b)?.state).toBe("backlog");
    expect(layer.focusItems().map((it) => it.id)).toEqual([b, a]);
    // Done through the routed path prunes in the same call.
    layer.routed.setDone(b, true);
    expect(layer.isFocused(b)).toBe(false);
    dispose();
  });

  test("Done is Home's unless amalgamated; the calendar hides docs by pref", () => {
    const { home, other, layer, dispose } = setup();
    const a = home.addItem("inbox", "a");
    const b = other.addItem("inbox", "b");
    home.setDone(a, true);
    other.setDone(b, true);
    expect(layer.doneAmalgamate()).toBe(false);
    expect(layer.doneItems().map((it) => it.id)).toEqual([a]);
    home.setDoneAmalgamate(true);
    expect(layer.doneItems().map((it) => it.id).sort()).toEqual([a, b].sort());

    const e1 = home.addItem("events", "home event");
    const e2 = other.addItem("events", "other event");
    expect(layer.calendarItems().map((it) => it.id)).toContain(e2);
    home.setCalendarDocHidden(OTHER, true);
    expect(layer.calendarHiddenDocs()).toEqual([OTHER]);
    const ids = layer.calendarItems().map((it) => it.id);
    expect(ids).toContain(e1);
    expect(ids).not.toContain(e2);
    home.setCalendarDocHidden(OTHER, false);
    expect(layer.calendarItems().map((it) => it.id)).toContain(e2);
    dispose();
  });

  test("undo and redo interleave the docs' histories chronologically", () => {
    const { home, other, layer, dispose } = setup();
    // History: Home, Other, Home.
    const a1 = home.addItem("inbox", "a1");
    const b = other.addItem("inbox", "b");
    const a2 = home.addItem("inbox", "a2");
    expect(layer.canUndo()).toBe(true);
    expect(layer.undo()).toBe(true);
    expect(home.getItem(a2)).toBeUndefined();
    expect(other.getItem(b)).toBeDefined();
    // The second undo is Other's step, not Home's older one.
    expect(layer.undo()).toBe(true);
    expect(other.getItem(b)).toBeUndefined();
    expect(home.getItem(a1)).toBeDefined();
    expect(layer.undo()).toBe(true);
    expect(home.getItem(a1)).toBeUndefined();
    expect(layer.canUndo()).toBe(false);
    expect(layer.undo()).toBe(false);
    // Redo replays in the original order: Home, Other, Home.
    expect(layer.redo()).toBe(true);
    expect(home.getItem(a1)).toBeDefined();
    expect(other.getItem(b)).toBeUndefined();
    expect(layer.redo()).toBe(true);
    expect(other.getItem(b)).toBeDefined();
    expect(layer.redo()).toBe(true);
    expect(home.getItem(a2)).toBeDefined();
    expect(layer.canRedo()).toBe(false);
    // A new action after a partial undo drops only that doc's redo
    // stack; the other doc's stays redoable.
    layer.undo();
    layer.undo();
    other.addItem("inbox", "c");
    expect(other.canRedo()).toBe(false);
    expect(home.canRedo()).toBe(true);
    expect(layer.redo()).toBe(true);
    expect(home.getItem(a2)).toBeDefined();
    dispose();
  });

  test("a doc added later joins the map and the merged reads", () => {
    const { layer, dispose } = createRoot((dispose) => {
      const home = appFor(HOME);
      return { layer: createDocsLayer(home, []), dispose };
    });
    expect(layer.docs().length).toBe(1);
    const third = appFor(OTHER);
    const x = third.addItem("inbox", "x");
    layer.addDoc(third);
    expect(layer.docs().map((d) => d.docId)).toEqual([HOME, OTHER]);
    expect(layer.getItem(x)?.text).toBe("x");
    layer.addDoc(third);
    expect(layer.docs().length).toBe(2);
    dispose();
  });
});
