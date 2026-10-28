// `createdBy` end to end through the web store: a doc with an author
// stamps items it creates; the stamp crosses the wasm boundary on both
// the live `itemAdded` event and the boot snapshot; a doc without an
// author stamps nothing (`spec/data-model.md` "Item").

import { describe, expect, test } from "bun:test";
import { createRoot } from "solid-js";
import { Dek, Doc, SyncEngine } from "@monoplan/core/wasm";
import type { EngineStorage } from "@monoplan/core/wasm";
import { MemEngineStorage } from "../../core/test/mem-engine-storage.ts";
import { createSyncedApp } from "../src/sync/store.ts";

const DOC_ID = "00000000-0000-0000-0000-00000000000b";

function appWith(author: string | undefined) {
  const doc = Doc.create();
  if (author !== undefined) doc.setAuthor(author);
  return createSyncedApp(
    new SyncEngine(
      doc,
      DOC_ID,
      Dek.generate(),
      0n,
      new MemEngineStorage() as unknown as EngineStorage,
    ),
  );
}

describe("createdBy", () => {
  test("an authored doc stamps the account id on items it creates", () => {
    createRoot((dispose) => {
      const app = appWith("acct-1");
      const id = app.addItem("inbox", "shared thing");
      expect(app.getItem(id)?.createdBy).toBe("acct-1");
      const [id2] = app.addItemsAt("inbox", ["bulk"], 0);
      expect(app.getItem(id2!)?.createdBy).toBe("acct-1");
      const id3 = app.addItemInState("inbox", "captured", "todo");
      expect(app.getItem(id3)?.createdBy).toBe("acct-1");
      dispose();
    });
  });

  test("an unauthored doc leaves createdBy absent", () => {
    createRoot((dispose) => {
      const app = appWith(undefined);
      const id = app.addItem("inbox", "anonymous thing");
      expect(app.getItem(id)?.createdBy).toBeUndefined();
      dispose();
    });
  });

  test("the stamp survives the boot snapshot path", () => {
    createRoot((dispose) => {
      const doc = Doc.create();
      doc.setAuthor("acct-9");
      const id = doc.addItem("inbox", "from before boot");
      // A store built over an engine whose doc already holds the item
      // materializes it from the snapshot JSON, not from an event.
      const app = createSyncedApp(
        new SyncEngine(
          doc,
          DOC_ID,
          Dek.generate(),
          0n,
          new MemEngineStorage() as unknown as EngineStorage,
        ),
      );
      expect(app.getItem(id)?.createdBy).toBe("acct-9");
      dispose();
    });
  });
});
