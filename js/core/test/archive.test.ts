// Archived-list coverage for the wasm surface (`spec/data-model.md`
// "Archived lists"): setListArchived on both binding surfaces, archive
// state riding the JSON reads (getListMetaJson / allListsJson /
// workspaceSnapshotJson), and the listArchivedChanged / listAdded
// AppEvent payloads the web store consumes.

import { describe, expect, test } from "bun:test";

import { Dek, Doc, SyncEngine } from "../wasm/monoplan_core_web.js";
import type { AppEventJs, EngineStorage } from "../wasm/monoplan_core_web.js";
import { MemEngineStorage } from "./mem-engine-storage.ts";

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

function drainAppEvents(eng: SyncEngine): AppEventJs[] {
  const out: AppEventJs[] = [];
  while (true) {
    const ev = eng.popAppEvent();
    if (!ev) break;
    out.push(ev);
  }
  return out;
}

describe("Doc.setListArchived", () => {
  test("archive state round-trips through the list JSON reads", () => {
    const doc = Doc.create();
    const id = doc.addList("Old");

    doc.setListArchived(id, true);
    const meta = JSON.parse(doc.getListMetaJson(id)!) as {
      name: string;
      archivedAt?: number;
    };
    expect(meta.archivedAt).toBeGreaterThan(0);
    expect(meta.name).toBe("Old");

    const lists = JSON.parse(doc.allListsJson()) as {
      id: string;
      archivedAt?: number;
    }[];
    expect(lists).toHaveLength(1);
    expect(lists[0].id).toBe(id);
    expect(lists[0].archivedAt).toBe(meta.archivedAt!);

    doc.setListArchived(id, false);
    const restored = JSON.parse(doc.getListMetaJson(id)!) as {
      archivedAt?: number;
    };
    expect(restored.archivedAt).toBeUndefined();
  });

  test("archiving is metadata-only: items and open order untouched", () => {
    const doc = Doc.create();
    const id = doc.addList("Old");
    const a = doc.addItem(id, "a");
    const b = doc.addItem(id, "b");
    doc.setItemDone(b, true);

    doc.setListArchived(id, true);
    expect(doc.openItemIds(id)).toEqual([a]);
    expect(doc.doneItemIds()).toEqual([b]);
    const item = JSON.parse(doc.getItemJson(a)!) as { listId: string };
    expect(item.listId).toBe(id);
  });

  test("refuses the reserved inbox", () => {
    const doc = Doc.create();
    expect(() => doc.setListArchived("inbox", true)).toThrow();
  });
});

describe("SyncEngine archive surface", () => {
  test("setListArchived emits listArchivedChanged both ways", () => {
    const eng = engineFrom(Doc.create());
    const id = eng.addList("Old");
    drainAppEvents(eng);

    eng.setListArchived(id, true);
    const archived = drainAppEvents(eng);
    expect(archived).toHaveLength(1);
    expect(archived[0].kind).toBe("listArchivedChanged");
    expect(archived[0].id).toBe(id);
    expect(Number(archived[0].archivedAt)).toBeGreaterThan(0);

    eng.setListArchived(id, false);
    const restored = drainAppEvents(eng);
    expect(restored).toHaveLength(1);
    expect(restored[0].kind).toBe("listArchivedChanged");
    expect(restored[0].archivedAt).toBeUndefined();
  });

  test("workspaceSnapshotJson and snapshotEvents carry archive state", () => {
    const doc = Doc.create();
    const active = doc.addList("Active");
    const old = doc.addList("Old");
    doc.setListArchived(old, true);
    const eng = engineFrom(doc);

    const snap = JSON.parse(eng.workspaceSnapshotJson()) as {
      lists: { id: string; archivedAt?: number }[];
    };
    expect(snap.lists.map((l) => l.id)).toEqual([active, old]);
    expect(snap.lists[0].archivedAt).toBeUndefined();
    expect(snap.lists[1].archivedAt).toBeGreaterThan(0);

    const adds = eng
      .snapshotEvents()
      .filter((ev) => ev.kind === "listAdded");
    expect(adds).toHaveLength(2);
    expect(adds[0].archivedAt).toBeUndefined();
    expect(Number(adds[1].archivedAt)).toBeGreaterThan(0);
  });
});
