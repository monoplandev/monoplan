// Archived-list coverage for the web store (`spec/data-model.md`
// "Archived lists"): boot materialization, incremental
// listArchivedChanged dispatch, the retained canonical projection
// (archived lists keep labelling their items in Search / Done / Bin),
// and the absence of any user-facing list delete.

import { describe, expect, test } from "bun:test";

import { Dek, Doc, SyncEngine } from "@monoplan/core/wasm";
import type { EngineStorage } from "@monoplan/core/wasm";
import { MemEngineStorage } from "../../core/test/mem-engine-storage.ts";
import { createRoot } from "solid-js";
import { createSyncedApp } from "../src/sync/store.ts";
import { createDocsLayer } from "../src/sync/docs.ts";
import { findResultArchived } from "../src/findResults.tsx";

const DOC_ID = "00000000-0000-0000-0000-000000000000";

function engineFrom(doc: Doc): SyncEngine {
  return new SyncEngine(
    doc,
    DOC_ID,
    Dek.generate(),
    0n,
    new MemEngineStorage() as unknown as EngineStorage,
  );
}

describe("store archived lists", () => {
  test("workspace snapshot materializes archived lists", () => {
    const doc = Doc.create();
    const active = doc.addList("Active");
    const old = doc.addList("Old");
    doc.setListArchived(old, true);

    const app = createSyncedApp(engineFrom(doc));
    // Both stay in the canonical order/byId projections.
    expect(app.state.listsOrder).toEqual([active, old]);
    expect(app.state.listsById[active]?.archivedAt).toBeUndefined();
    expect(app.state.listsById[old]?.archivedAt).toBeGreaterThan(0);
  });

  test("incremental archive events update projections without losing metadata", () => {
    const doc = Doc.create();
    const id = doc.addList("Work");
    doc.setListIcon(id, "🎯");
    const app = createSyncedApp(engineFrom(doc));
    expect(app.state.listsById[id]?.archivedAt).toBeUndefined();

    app.setListArchived(id, true);
    const archived = app.state.listsById[id];
    expect(archived?.archivedAt).toBeGreaterThan(0);
    expect(archived?.name).toBe("Work");
    expect(archived?.icon).toBe("🎯");
    // Archived ≠ removed: the list keeps its slot in the order.
    expect(app.state.listsOrder).toContain(id);

    // Unarchive returns it to the active projection, metadata intact.
    app.setListArchived(id, false);
    const restored = app.state.listsById[id];
    expect(restored?.archivedAt).toBeUndefined();
    expect(restored?.name).toBe("Work");
    expect(restored?.icon).toBe("🎯");
  });

  test("archiving leaves the list's items and open order untouched", () => {
    const doc = Doc.create();
    const id = doc.addList("Errands");
    const a = doc.addItem(id, "a");
    const b = doc.addItem(id, "b");
    const app = createSyncedApp(engineFrom(doc));

    app.setListArchived(id, true);
    expect(app.state.listOpen[id]).toEqual([a, b]);
    expect(app.state.itemsById[a]?.listId).toBe(id);
    expect(app.state.itemsById[b]?.listId).toBe(id);
  });

  test("archived list names still label their items in search / Done / Bin", () => {
    const doc = Doc.create();
    const id = doc.addList("Errands");
    const open = doc.addItem(id, "buy milk");
    const done = doc.addItem(id, "done thing");
    const binned = doc.addItem(id, "binned thing");
    doc.setItemDone(done, true);
    doc.setItemBinned(binned, true);
    const app = createSyncedApp(engineFrom(doc));

    app.setListArchived(id, true);
    // Done / Bin rows resolve the origin-list label via listsById — the
    // archived entry must still be there.
    expect(app.state.listsById[id]?.name).toBe("Errands");
    expect(app.state.itemsById[done]?.listId).toBe(id);
    expect(app.state.itemsById[binned]?.listId).toBe(id);

    // Search keeps the list indexed and keeps supplying its name as
    // item context (spec/search.md).
    const results = app.search.query("errands");
    expect(results.some((r) => r.kind === "list" && r.id === id)).toBe(true);
    expect(results.some((r) => r.kind === "item" && r.id === open)).toBe(true);
  });

  test("find rows flag an archived list and the items living in it", () => {
    const doc = Doc.create();
    const work = doc.addList("Work");
    const home = doc.addList("Home");
    const inWork = doc.addItem(work, "ship it");
    const inHome = doc.addItem(home, "ship it too");
    const inInbox = doc.addItem("inbox", "ship it three");
    const app = createSyncedApp(engineFrom(doc));
    // The Find surfaces resolve lists through the cross-doc layer; one
    // doc here, so it is just a view over `app`.
    const layer = createRoot(() => createDocsLayer(app, []));

    const rowFor = (id: string) => {
      const r = app.search.query("ship").find((x) => x.id === id);
      if (!r) throw new Error(`no result for ${id}`);
      return r;
    };
    const listRow = (id: string) => {
      const r = app.search.query(id === work ? "work" : "home").find((x) => x.id === id);
      if (!r) throw new Error(`no result for ${id}`);
      return r;
    };

    // Nothing archived yet: no row carries the marker.
    expect(findResultArchived(layer, listRow(work))).toBe(false);
    expect(findResultArchived(layer, rowFor(inWork))).toBe(false);

    app.setListArchived(work, true);
    // The list's own row and every item it owns are flagged; unrelated
    // lists, inbox items and built-in views are not.
    expect(findResultArchived(layer, listRow(work))).toBe(true);
    expect(findResultArchived(layer, rowFor(inWork))).toBe(true);
    expect(findResultArchived(layer, listRow(home))).toBe(false);
    expect(findResultArchived(layer, rowFor(inHome))).toBe(false);
    expect(findResultArchived(layer, rowFor(inInbox))).toBe(false);
    expect(
      findResultArchived(layer, { kind: "view", id: "focus", title: "Focus" }),
    ).toBe(false);

    // Unarchiving clears the marker again.
    app.setListArchived(work, false);
    expect(findResultArchived(layer, listRow(work))).toBe(false);
    expect(findResultArchived(layer, rowFor(inWork))).toBe(false);
  });

  test("no user-facing list delete remains on the store API", () => {
    const app = createSyncedApp(engineFrom(Doc.create()));
    expect(
      (app as unknown as Record<string, unknown>).deleteList,
    ).toBeUndefined();
  });
});
