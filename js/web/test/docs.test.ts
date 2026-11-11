// The cross-doc personal layer (`src/sync/docs.ts`): the reverse index,
// mutations routed by the item's doc, the merged lenses, foreign Focus
// refs (spec/focus.md §FocusRef grammar) and per-doc undo. Two in-memory
// replicas stand in for Home and one other doc.

import { describe, expect, test } from "bun:test";
import { createRoot } from "solid-js";
import { Dek, Doc, Replica } from "@monoplan/core/wasm";
import type { ReplicaStorage } from "@monoplan/core/wasm";
import { MemReplicaStorage } from "../../core/test/mem-replica-storage.ts";
import {
  createHomeApp,
  createSyncedApp,
  type DocApp,
  type HomeApp,
} from "../src/sync/store.ts";
import { createDocsLayer, type DocsLayer } from "../src/sync/docs.ts";
import { restoreCapturedPositions } from "../src/linger.ts";

const HOME = "00000000-0000-0000-0000-00000000000a";
const OTHER = "00000000-0000-0000-0000-00000000000b";

function replicaFor(docId: string): Replica {
  return new Replica(
    Doc.create(),
    docId,
    Dek.generate(),
    0n,
    new MemReplicaStorage() as unknown as ReplicaStorage,
  );
}

function setup(): { home: HomeApp; other: DocApp; layer: DocsLayer; dispose: () => void } {
  return createRoot((dispose) => {
    const home = createHomeApp(replicaFor(HOME));
    const other = createSyncedApp(replicaFor(OTHER));
    const layer = createDocsLayer(home, [other]);
    return { home, other, layer, dispose };
  });
}

describe("docs layer", () => {
  test("placeSuggestions spans every open doc, and forgets a removed one", () => {
    const { home, other, layer, dispose } = setup();
    const a = home.addItem("inbox", "a");
    home.setItemPlace(a, { label: "gym" });
    const b = other.addItem("inbox", "b");
    other.setItemPlace(b, { label: "Gym", lat: -33.8688, lon: 151.2093 });
    const c = other.addItem("inbox", "c");
    other.setItemPlace(c, { label: "Office" });
    other.setItemWhen(c, "2099-01-01");

    const rows = layer.placeSuggestions();
    expect(rows.map((r) => r.place.label)).toEqual(["Office", "Gym"]);
    // Home's label-only "gym" and the other doc's geocoded Gym are one
    // place, the one with coordinates.
    expect(rows[1].count).toBe(2);
    expect(rows[1].place.lat).toBe(-33.8688);
    expect(layer.placeSuggestions("off").map((r) => r.place.label)).toEqual(["Office"]);
    expect(layer.placeSuggestions("", 1)).toHaveLength(1);

    layer.removeDoc(OTHER);
    expect(layer.placeSuggestions().map((r) => r.place.label)).toEqual(["gym"]);
    dispose();
  });

  test("removeDoc forgets a non-Home doc and drops its Focus pins; Home stays", () => {
    const { home, other, layer, dispose } = setup();
    const mine = home.addItem("inbox", "mine");
    const theirs = other.addItem("inbox", "theirs");
    layer.routed.addToFocus(mine);
    layer.routed.addToFocus(theirs);
    expect(layer.focusItems().map((i) => i.id)).toEqual([theirs, mine]);

    layer.removeDoc(OTHER);
    expect(layer.docs().map((d) => d.docId)).toEqual([HOME]);
    expect(layer.appOf(OTHER)).toBeUndefined();
    expect(layer.docIdOf(theirs)).toBeUndefined();
    // The foreign pin is gone for good, the local one untouched.
    expect(home.state.focusEntries.map((e) => e.itemId)).toEqual([mine]);
    // A view scoped to the gone doc falls back to Home.
    expect(layer.viewDocId({ kind: "list", id: "x", docId: OTHER })).toBe(HOME);
    // Home is never removed; a second removal is a no-op.
    layer.removeDoc(HOME);
    layer.removeDoc(OTHER);
    expect(layer.docs().map((d) => d.docId)).toEqual([HOME]);
    dispose();
  });

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
    // Focus is Home's alone: the other doc's app has no Focus state.
    expect("focusEntries" in other.state && (other.state as HomeApp["state"]).focusEntries).toEqual([]);
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
    // Un-done into Focus reopens the foreign item and re-pins it at the
    // slot it held.
    layer.routed.undoneIntoFocus([b]);
    expect(other.getItem(b)?.state).toBe("backlog");
    expect(layer.focusItems().map((it) => it.id)).toEqual([b, a]);
    // Done through the routed path prunes in the same call.
    layer.routed.setDone(b, true);
    expect(layer.isFocused(b)).toBe(false);
    dispose();
  });

  test("Focus linger captures slots in the merged order, foreign rows included", () => {
    const { home, other, layer, dispose } = setup();
    const h1 = home.addItem("inbox", "h1");
    const h2 = home.addItem("inbox", "h2");
    const f1 = other.addItem("inbox", "f1");
    const f2 = other.addItem("inbox", "f2");
    const unstarred = home.addItem("inbox", "unstarred");
    layer.routed.addToFocusMany([f1, h1, f2, h2]);
    const lens = (): string[] => layer.focusItems().map((it) => it.id);
    expect(lens()).toEqual([f1, h1, f2, h2]);

    // A Home row below a foreign one: its slot counts the foreign row.
    layer.routed.setDone(h1, true);
    // A foreign row lingers too, at its slot once h1 has left.
    layer.routed.setDone(f2, true);
    // A row outside Focus records nothing.
    layer.routed.setDone(unstarred, true);
    expect(layer.focusRecentDone().map((e) => [e.id, e.index])).toEqual([
      [h1, 1],
      [f2, 1],
    ]);
    expect(lens()).toEqual([f1, h2]);

    // The overlay's replay rebuilds the lens as it stood.
    const rows = lens();
    restoreCapturedPositions(
      rows,
      layer.focusRecentDone().map((e) => ({ index: e.index, value: e.id })),
    );
    expect(rows).toEqual([f1, h1, f2, h2]);

    // Un-done re-pins both where they sat, in Home's entries.
    layer.routed.undoneIntoFocus([h1, f2]);
    expect(lens()).toEqual([f1, h1, f2, h2]);
    expect(home.state.focusEntries).toEqual([
      { docId: OTHER, itemId: f1 },
      { docId: null, itemId: h1 },
      { docId: OTHER, itemId: f2 },
      { docId: null, itemId: h2 },
    ]);
    expect(layer.focusRecentDone()).toEqual([]);

    // A close made in the item's own doc (a peer's, say) is captured the
    // same way, before the ref is pruned.
    other.setDone(f1, true);
    expect(layer.focusRecentDone().map((e) => [e.id, e.index])).toEqual([[f1, 0]]);
    dispose();
  });

  test("Done spans every doc; the calendar hides docs by pref", () => {
    const { home, other, layer, dispose } = setup();
    const a = home.addItem("inbox", "a");
    const b = other.addItem("inbox", "b");
    home.setDone(a, true);
    other.setDone(b, true);
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

  test("a hard delete is permanent: it clears that doc's undo / redo history", () => {
    const { home, other, layer, dispose } = setup();
    const kept = home.addItem("inbox", "kept");
    const gone = home.addItem("inbox", "gone");
    home.setBinned(gone, true);
    const b = other.addItem("inbox", "b");
    // Leave Home with a redo entry as well.
    const extra = home.addItem("inbox", "extra");
    expect(layer.undo()).toBe(true);
    expect(home.getItem(extra)).toBeUndefined();
    expect(home.canUndo()).toBe(true);
    expect(home.canRedo()).toBe(true);

    layer.routed.deleteBinnedMany([gone]);
    expect(home.getItem(gone)).toBeUndefined();
    expect(home.canUndo()).toBe(false);
    expect(home.canRedo()).toBe(false);
    // The other doc's history is untouched, and undo goes there now.
    expect(layer.canUndo()).toBe(true);
    expect(layer.undo()).toBe(true);
    expect(other.getItem(b)).toBeUndefined();
    expect(layer.undo()).toBe(false);
    expect(home.getItem(gone)).toBeUndefined();
    expect(home.getItem(kept)).toBeDefined();

    // Steps after the delete are undoable as usual.
    const after = home.addItem("inbox", "after");
    expect(layer.undo()).toBe(true);
    expect(home.getItem(after)).toBeUndefined();
    expect(home.canUndo()).toBe(false);

    // emptyBin behaves the same, across every doc.
    home.setBinned(kept, true);
    const c = other.addItem("inbox", "c");
    other.setBinned(c, true);
    expect(layer.emptyBin()).toBe(2);
    expect(layer.canUndo()).toBe(false);
    expect(layer.undo()).toBe(false);
    expect(home.getItem(kept)).toBeUndefined();
    expect(other.getItem(c)).toBeUndefined();
    // An empty bin is a no-op that keeps history.
    const d = home.addItem("inbox", "d");
    expect(layer.emptyBin()).toBe(0);
    expect(layer.undo()).toBe(true);
    expect(home.getItem(d)).toBeUndefined();
    dispose();
  });

  test("a doc added later joins the map and the merged reads", () => {
    const { layer, dispose } = createRoot((dispose) => {
      const home = createHomeApp(replicaFor(HOME));
      return { layer: createDocsLayer(home, []), dispose };
    });
    expect(layer.docs().length).toBe(1);
    const third = createSyncedApp(replicaFor(OTHER));
    const x = third.addItem("inbox", "x");
    layer.addDoc(third);
    expect(layer.docs().map((d) => d.docId)).toEqual([HOME, OTHER]);
    expect(layer.getItem(x)?.text).toBe("x");
    layer.addDoc(third);
    expect(layer.docs().length).toBe(2);
    dispose();
  });
});

describe("cross-doc move", () => {
  const THIRD = "00000000-0000-0000-0000-00000000000c";

  test("items move under their own ids, as a block at the index", () => {
    const { home, other, layer, dispose } = setup();
    const list = other.addList("Target");
    const x = other.addItem(list, "x");
    const y = other.addItem(list, "y");
    const a = home.addItem("inbox", "a");
    const done = home.addItem("inbox", "done");
    const binned = home.addItem("inbox", "binned");
    const stays = home.addItem("inbox", "stays");
    home.setItemDeadline(a, "2026-12-01");
    home.setItemNotes(a, "some notes");
    home.setDone(done, true);
    home.setBinned(binned, true);

    expect(layer.moveToDoc([a, done, binned], OTHER, list, 1)).toEqual([a, done, binned]);

    expect(home.getItem(a)).toBeUndefined();
    expect(home.getItem(done)).toBeUndefined();
    expect(home.getItem(binned)).toBeUndefined();
    expect(home.state.binCount).toBe(0);
    expect(home.getItem(stays)).toBeDefined();
    expect(layer.docIdOf(a)).toBe(OTHER);
    const moved = other.getItem(a)!;
    expect(moved.docId).toBe(OTHER);
    expect(moved.listId).toBe(list);
    expect(moved.text).toBe("a");
    expect(moved.deadline).toBe("2026-12-01");
    expect(moved.notes).toBe("some notes");
    // Done stays Done; a binned row lands restored.
    expect(other.getItem(done)?.state).toBe("done");
    expect(other.getItem(binned)?.binnedAt ?? null).toBeNull();
    expect(other.state.listOpen[list]).toEqual([x, a, binned, y]);
    dispose();
  });

  test("ids already in the target, unknown ids and unknown lists move nothing", () => {
    const { home, other, layer, dispose } = setup();
    const a = home.addItem("inbox", "a");
    const b = other.addItem("inbox", "b");
    expect(layer.moveToDoc([b, "nope"], OTHER, "inbox", 0)).toEqual([]);
    expect(layer.moveToDoc([a], OTHER, "no-such-list", 0)).toEqual([]);
    expect(layer.moveToDoc([a], "no-such-doc", "inbox", 0)).toEqual([]);
    expect(home.getItem(a)).toBeDefined();
    // Nothing happened, so nothing was recorded beyond the two adds.
    expect(layer.undo()).toBe(true);
    expect(layer.undo()).toBe(true);
    expect(layer.undo()).toBe(false);
    dispose();
  });

  test("a starred item keeps its Focus slot, in and out of Home", () => {
    const { home, layer, dispose } = setup();
    const a = home.addItem("inbox", "a");
    const b = home.addItem("inbox", "b");
    const c = home.addItem("inbox", "c");
    layer.routed.addToFocusMany([a, b, c]);

    layer.moveToDoc([b], OTHER, "inbox", 0);
    expect(home.state.focusEntries).toEqual([
      { docId: null, itemId: a },
      { docId: OTHER, itemId: b },
      { docId: null, itemId: c },
    ]);
    expect(layer.focusItems().map((it) => it.id)).toEqual([a, b, c]);
    layer.pruneDeadForeignRefs();
    expect(layer.isFocused(b)).toBe(true);

    layer.moveToDoc([b], HOME, "inbox", 0);
    expect(home.state.focusEntries).toEqual([
      { docId: null, itemId: a },
      { docId: null, itemId: b },
      { docId: null, itemId: c },
    ]);
    expect(layer.focusItems().map((it) => it.id)).toEqual([a, b, c]);
    dispose();
  });

  test("between two shared docs Home only re-points the ref", () => {
    const { home, other, layer, dispose } = setup();
    const third = createSyncedApp(replicaFor(THIRD));
    layer.addDoc(third);
    const b = other.addItem("inbox", "b");
    layer.routed.addToFocus(b);
    layer.moveToDoc([b], THIRD, "inbox", 0);
    expect(home.state.focusEntries).toEqual([{ docId: THIRD, itemId: b }]);
    expect(layer.focusItems().map((it) => it.id)).toEqual([b]);
    // One undo walks all three docs back; one redo, forward again.
    expect(layer.undo()).toBe(true);
    expect(other.getItem(b)).toBeDefined();
    expect(third.getItem(b)).toBeUndefined();
    expect(home.state.focusEntries).toEqual([{ docId: OTHER, itemId: b }]);
    expect(layer.redo()).toBe(true);
    expect(third.getItem(b)).toBeDefined();
    expect(other.getItem(b)).toBeUndefined();
    expect(home.state.focusEntries).toEqual([{ docId: THIRD, itemId: b }]);
    dispose();
  });

  test("one undo reverts both halves, one redo reapplies them", () => {
    const { home, other, layer, dispose } = setup();
    const a = home.addItem("inbox", "a");
    const b = home.addItem("inbox", "b");
    const c = home.addItem("inbox", "c");
    const o = other.addItem("inbox", "o");
    layer.routed.addToFocus(b);
    const touched: string[] = [];
    const off = layer.onUndoRedo((_outcome, docId) => touched.push(docId));

    layer.moveToDoc([b], OTHER, "inbox", 0);
    expect(other.state.listOpen["inbox"]).toEqual([b, o]);

    expect(layer.undo()).toBe(true);
    expect(other.getItem(b)).toBeUndefined();
    expect(home.getItem(b)?.docId).toBe(HOME);
    expect(home.state.listOpen["inbox"]).toEqual([a, b, c]);
    expect(home.state.focusEntries).toEqual([{ docId: null, itemId: b }]);
    // The giving doc reports last: it is where the rows are again.
    expect(touched).toEqual([OTHER, HOME]);

    expect(layer.redo()).toBe(true);
    expect(home.getItem(b)).toBeUndefined();
    expect(other.state.listOpen["inbox"]).toEqual([b, o]);
    expect(home.state.focusEntries).toEqual([{ docId: OTHER, itemId: b }]);

    // And back once more, then on to the steps before the move.
    expect(layer.undo()).toBe(true);
    expect(home.getItem(b)).toBeDefined();
    expect(layer.undo()).toBe(true);
    expect(layer.isFocused(b)).toBe(false);
    off();
    dispose();
  });

  test("a move whose other half lost its history is not undone alone", () => {
    const { home, other, layer, dispose } = setup();
    const a = home.addItem("inbox", "a");
    const junk = home.addItem("inbox", "junk");
    const o = other.addItem("inbox", "o");
    expect(layer.moveToDoc([a], OTHER, "inbox", 0)).toEqual([a]);
    // A hard delete in the giving doc drops its history, the release
    // included.
    home.setBinned(junk, true);
    layer.routed.deleteBinnedMany([junk]);
    expect(home.canUndo()).toBe(false);

    // Undoing the adopt alone would lose the item: the move stands and
    // the receiving doc's history goes instead.
    expect(layer.undo()).toBe(false);
    expect(other.getItem(a)).toBeDefined();
    expect(other.getItem(o)).toBeDefined();
    expect(layer.canUndo()).toBe(false);
    // Later steps undo as usual.
    const later = other.addItem("inbox", "later");
    expect(layer.undo()).toBe(true);
    expect(other.getItem(later)).toBeUndefined();
    expect(other.getItem(a)).toBeDefined();
    dispose();
  });

  test("a move out of a doc that then closes cannot be undone", () => {
    const { home, other, layer, dispose } = setup();
    const b = other.addItem("inbox", "b");
    layer.moveToDoc([b], HOME, "inbox", 0);
    expect(other.getItem(b)).toBeUndefined();
    layer.removeDoc(OTHER);
    expect(layer.undo()).toBe(false);
    expect(home.getItem(b)).toBeDefined();
    dispose();
  });
});
