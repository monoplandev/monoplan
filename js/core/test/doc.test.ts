// Headless smoke for the wasm Doc surface: build a doc, mutate it, run
// it through `save()`/`load()`, and prove the logical fingerprint
// survives the round trip.

import { describe, expect, test } from "bun:test";

import { Dek, Doc, EncryptedBlob } from "../wasm/monoplan_core_web.js";

const LIST_MAIN = "inbox";

function bytesEqual(a: Uint8Array, b: Uint8Array): boolean {
  if (a.length !== b.length) return false;
  for (let i = 0; i < a.length; i++) {
    if (a[i] !== b[i]) return false;
  }
  return true;
}

describe("Doc save/load", () => {
  test("addItem → save → load → fingerprint matches", () => {
    const doc = Doc.create();
    const itemId = doc.addItem(LIST_MAIN, "buy milk");

    const before = doc.fingerprint();
    const bytes = doc.save();

    const restored = Doc.load(bytes);
    const after = restored.fingerprint();

    expect(bytesEqual(before, after)).toBe(true);

    const items = JSON.parse(restored.itemsInListJson(LIST_MAIN, false));
    expect(items).toHaveLength(1);
    expect(items[0].id).toBe(itemId);
    expect(items[0].text).toBe("buy milk");
  });

  test("fresh doc has no user lists; main is not a ListMeta", () => {
    const doc = Doc.create();
    const lists = JSON.parse(doc.allListsJson()) as Array<{
      id: string;
      name: string;
    }>;
    // Per spec/data-model.md: `main` is a reserved id, not a MovableList
    // entry, and there are no seeded user lists.
    expect(lists).toHaveLength(0);
    expect(lists.some((l) => l.id === LIST_MAIN)).toBe(false);
  });
});

describe("Doc JSON import (additive)", () => {
  test("round-trip: fresh-list IDs, main routes locally, content preserved", () => {
    const src = Doc.create();
    const otherId = src.addList("Other");
    const alpha = src.addItem(LIST_MAIN, "alpha");
    const beta = src.addItem(LIST_MAIN, "beta");
    const gamma = src.addItem(otherId, "gamma");
    src.editItemNotes(alpha, "alpha notes");
    src.setItemDone(beta, true);
    src.setItemBinned(gamma, true);

    const srcMainTs = JSON.parse(src.itemsInListJson(LIST_MAIN, true)) as Array<{
      id: string;
      createdAt: number;
      doneAt?: number;
      binnedAt?: number;
    }>;
    const alphaSrc = srcMainTs.find((i) => i.id === alpha)!;
    const betaSrc = srcMainTs.find((i) => i.id === beta)!;
    expect(alphaSrc).toBeDefined();
    expect(betaSrc.doneAt).toBeDefined();

    const json = src.exportJson();

    const dst = Doc.create();
    const summary = JSON.parse(dst.importJson(json)) as {
      listsAdded: number;
      itemsAdded: number;
      itemsSkipped: number;
      focusAdded: number;
    };
    expect(summary).toEqual({
      listsAdded: 1,
      itemsAdded: 3,
      itemsSkipped: 0,
      focusAdded: 0,
    });

    // One fresh user list with a NEW id but the same name.
    const dstLists = JSON.parse(dst.allListsJson()) as Array<{
      id: string;
      name: string;
    }>;
    expect(dstLists).toHaveLength(1);
    expect(dstLists[0].name).toBe("Other");
    expect(dstLists[0].id).not.toBe(otherId);

    // alpha + beta in main (beta is done — include_binned true also
    // includes done in itemsInListJson? actually include_binned is just
    // the bin flag; done items still show in the list view).
    const mainItems = JSON.parse(dst.itemsInListJson(LIST_MAIN, true)) as Array<{
      text: string;
      notes: string;
      createdAt: number;
      doneAt?: number;
      binnedAt?: number;
    }>;
    const mainByText = (t: string) => mainItems.find((i) => i.text === t)!;
    expect(mainByText("alpha").notes).toBe("alpha notes");
    expect(mainByText("alpha").createdAt).toBe(alphaSrc.createdAt);
    expect(mainByText("beta").doneAt).toBe(betaSrc.doneAt);

    // gamma is binned — show up in the bin view, list_id matches the
    // newly-created user list.
    const binned = JSON.parse(dst.binnedItemsJson()) as Array<{
      text: string;
      listId: string;
    }>;
    expect(binned).toHaveLength(1);
    expect(binned[0].text).toBe("gamma");
    expect(binned[0].listId).toBe(dstLists[0].id);
  });

  test("additive: existing local content is untouched", () => {
    const dst = Doc.create();
    const keepList = dst.addList("LocalKeep");
    dst.addItem(LIST_MAIN, "local-main");
    dst.addItem(keepList, "local-other");

    const src = Doc.create();
    src.addList("Imported");
    src.addItem(LIST_MAIN, "src-main");

    const summary = JSON.parse(dst.importJson(src.exportJson())) as {
      listsAdded: number;
      itemsAdded: number;
    };
    expect(summary.listsAdded).toBe(1);
    expect(summary.itemsAdded).toBe(1);

    const lists = JSON.parse(dst.allListsJson()) as Array<{ name: string }>;
    const names = lists.map((l) => l.name).sort();
    expect(names).toEqual(["Imported", "LocalKeep"]);

    const mainTexts = (
      JSON.parse(dst.itemsInListJson(LIST_MAIN, false)) as Array<{ text: string }>
    )
      .map((i) => i.text)
      .sort();
    expect(mainTexts).toEqual(["local-main", "src-main"]);
  });

  test("importJson throws on invalid JSON", () => {
    const dst = Doc.create();
    expect(() => dst.importJson("not json {{{")).toThrow();
  });
});

describe("Dek round trip", () => {
  test("generate → toHex → fromHex preserves bytes", () => {
    const a = Dek.generate();
    const hex = a.toHex();
    expect(hex).toHaveLength(64);
    const b = Dek.fromHex(hex);
    expect(b.toHex()).toBe(hex);
  });
});

describe("Op stream round trip via two replicas", () => {
  test("replica B converges to A's fingerprint after applying A's blob", () => {
    const dek = Dek.generate();

    // A fresh doc has no commits (no seeded user lists), so
    // pendingExport is None until a real mutation lands.
    const a = Doc.create();
    expect(a.pendingExport(dek)).toBeUndefined();

    const itemId = a.addItem(LIST_MAIN, "from A");
    const opBlob = a.pendingExport(dek);
    expect(opBlob).toBeDefined();
    a.markPersisted();

    // B starts empty and replays the single op stream.
    const b = Doc.empty();
    b.applyRemote(dek, opBlob!);

    expect(bytesEqual(a.fingerprint(), b.fingerprint())).toBe(true);

    const items = JSON.parse(b.itemsInListJson(LIST_MAIN, false));
    expect(items.find((i: { id: string }) => i.id === itemId)).toBeDefined();
  });

  test("EncryptedBlob constructor + getters are symmetric", () => {
    const nonce = new Uint8Array(24).fill(7);
    const ciphertext = new Uint8Array([1, 2, 3, 4, 5]);
    const blob = new EncryptedBlob(nonce, ciphertext);
    expect(bytesEqual(blob.nonce, nonce)).toBe(true);
    expect(bytesEqual(blob.ciphertext, ciphertext)).toBe(true);
  });
});

describe("Doc duration", () => {
  test("negative, zero, fractional and oversized durations are rejected", () => {
    const doc = Doc.create();
    const id = doc.addItem(LIST_MAIN, "meeting");
    doc.setItemWhen(id, "2026-09-12T14:00");
    for (const bad of [-5, 0, -1, 7 * 24 * 60 + 1]) {
      expect(() => doc.setItemDuration(id, bad)).toThrow();
    }
    doc.setItemDuration(id, 90);
    const items = JSON.parse(doc.itemsInListJson(LIST_MAIN, false));
    expect(items[0].duration).toBe(90);
    // Clearing the date takes the length with it.
    doc.setItemWhen(id, undefined);
    const after = JSON.parse(doc.itemsInListJson(LIST_MAIN, false));
    expect(after[0].duration).toBeUndefined();
  });
});
