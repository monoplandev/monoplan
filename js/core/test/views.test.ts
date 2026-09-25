// The view-id helpers (open / done / binned) plus get_item /
// get_list_meta as JS-side primitives. Rust unit tests cover
// correctness; this file pins the wasm-bindgen surface so a rename or
// signature drift is caught before the web client breaks.

import { describe, expect, test } from "bun:test";

import { Doc, ItemLifecycle } from "../wasm/monoplan_core_web.js";

const LIST_MAIN = "inbox";

async function sleep(ms: number) {
  await new Promise((r) => setTimeout(r, ms));
}

describe("Doc view helpers", () => {
  test("empty doc returns empty arrays for every view", () => {
    const doc = Doc.create();
    expect(doc.openItemIds(LIST_MAIN)).toEqual([]);
    expect(doc.doneItemIds()).toEqual([]);
    expect(doc.binnedItemIds()).toEqual([]);
  });

  test("openItemIds returns ids in MovableList order, scoped to list and view", async () => {
    const doc = Doc.create();
    const other = doc.addList("Other");
    const a = doc.addItem(LIST_MAIN, "a");
    const b = doc.addItem(LIST_MAIN, "b");
    const c = doc.addItem(LIST_MAIN, "c");
    doc.addItem(other, "h"); // must not appear in main's view
    doc.setItemDone(b, true);
    const d = doc.addItem(LIST_MAIN, "d");
    doc.setItemBinned(d, true);

    expect(doc.openItemIds(LIST_MAIN)).toEqual([a, c]);
    expect(doc.openItemIds(other).length).toBe(1);
  });

  test("doneItemIds sorted by doneAt desc with id tiebreaker", async () => {
    const doc = Doc.create();
    const first = doc.addItem(LIST_MAIN, "first");
    const second = doc.addItem(LIST_MAIN, "second");
    const third = doc.addItem(LIST_MAIN, "third");
    doc.setItemDone(first, true);
    await sleep(2);
    doc.setItemDone(second, true);
    await sleep(2);
    doc.setItemDone(third, true);

    expect(doc.doneItemIds()).toEqual([third, second, first]);
  });

  test("cancelled items join the Done view, sorted with done ones", async () => {
    const doc = Doc.create();
    const a = doc.addItem(LIST_MAIN, "a");
    const b = doc.addItem(LIST_MAIN, "b");
    doc.setItemLifecycle(a, ItemLifecycle.Cancelled);
    await sleep(2);
    doc.setItemDone(b, true);
    expect(doc.openItemIds(LIST_MAIN)).toEqual([]);
    expect(doc.doneItemIds()).toEqual([b, a]);
    const view = JSON.parse(doc.getItemJson(a)!);
    expect(view.state).toBe("cancelled");
    expect(view.doneAt).toBeUndefined();
    // Un-done reopens a cancelled item into Backlog.
    doc.setItemDone(a, false);
    expect(doc.openItemIds(LIST_MAIN)).toEqual([a]);
    expect(JSON.parse(doc.getItemJson(a)!).state).toBe("backlog");
  });

  test("done-and-binned item appears in bin only", async () => {
    const doc = Doc.create();
    const a = doc.addItem(LIST_MAIN, "a");
    doc.setItemDone(a, true);
    doc.setItemBinned(a, true);
    expect(doc.doneItemIds()).toEqual([]);
    expect(doc.binnedItemIds()).toEqual([a]);
    // Done state survives the bin: the JSON view still has doneAt.
    const view = JSON.parse(doc.getItemJson(a)!);
    expect(view.doneAt).toBeDefined();
    expect(view.binnedAt).toBeDefined();
  });

  test("binnedItemIds sorted by binnedAt desc", async () => {
    const doc = Doc.create();
    const a = doc.addItem(LIST_MAIN, "a");
    const b = doc.addItem(LIST_MAIN, "b");
    doc.setItemBinned(a, true);
    await sleep(2);
    doc.setItemBinned(b, true);
    expect(doc.binnedItemIds()).toEqual([b, a]);
  });

  test("getItemJson and getListMetaJson round-trip through JSON.parse", () => {
    const doc = Doc.create();
    const id = doc.addItem(LIST_MAIN, "xyz");

    const item = JSON.parse(doc.getItemJson(id)!);
    expect(item.id).toBe(id);
    expect(item.text).toBe("xyz");
    expect(item.listId).toBe(LIST_MAIN);
    expect(item.doneAt).toBeUndefined();
    expect(item.binnedAt).toBeUndefined();

    expect(doc.getItemJson("does-not-exist")).toBeUndefined();

    // `main` is a reserved id with no MovableList entry — clients render
    // its label themselves, so getListMetaJson(LIST_MAIN) is undefined.
    expect(doc.getListMetaJson(LIST_MAIN)).toBeUndefined();

    const userListId = doc.addList("Groceries");
    const list = JSON.parse(doc.getListMetaJson(userListId)!);
    expect(list.id).toBe(userListId);
    expect(list.name).toBe("Groceries");

    expect(doc.getListMetaJson("nope")).toBeUndefined();
  });
});
