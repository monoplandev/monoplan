// Subtasks end to end through the web store (`spec/subtasks.md`): each
// write crosses the wasm boundary as a surgical `itemSubtasksChanged`
// carrying the whole list, each is one undo step, and subtasks already
// in the doc arrive with the boot snapshot.

import { describe, expect, test } from "bun:test";
import { createRoot } from "solid-js";
import { Dek, Doc, Replica } from "@monoplan/core/wasm";
import type { ReplicaStorage } from "@monoplan/core/wasm";
import { MemReplicaStorage } from "../../core/test/mem-replica-storage.ts";
import { createSyncedApp } from "../src/sync/store.ts";
import { dragTarget, subtaskCount } from "../src/SubtasksField.tsx";

const DOC_ID = "00000000-0000-0000-0000-00000000000c";

function appOver(doc: Doc) {
  return createSyncedApp(
    new Replica(
      doc,
      DOC_ID,
      Dek.generate(),
      0n,
      new MemReplicaStorage() as unknown as ReplicaStorage,
    ),
  );
}

describe("subtasks", () => {
  test("add, tick, rename, move and remove through the store", () => {
    createRoot((dispose) => {
      const app = appOver(Doc.create());
      const id = app.addItem("inbox", "shopping");
      const shape = () =>
        (app.getItem(id)?.subtasks ?? []).map((s) => (s.done ? "x " : "") + s.text);
      expect(app.getItem(id)?.subtasks).toBeUndefined();
      expect(subtaskCount(app.getItem(id)?.subtasks)).toBeNull();

      const milk = app.addSubtask(id, " milk ")!;
      const eggs = app.addSubtask(id, "eggs")!;
      app.addSubtask(id, "bread", 0);
      expect(app.addSubtask(id, "   ")).toBeUndefined();
      expect(shape()).toEqual(["bread", "milk", "eggs"]);

      app.setSubtaskDone(id, milk, true);
      app.setSubtaskText(id, eggs, "a dozen eggs");
      expect(shape()).toEqual(["bread", "x milk", "a dozen eggs"]);
      expect(subtaskCount(app.getItem(id)?.subtasks)).toEqual({ done: 1, total: 3 });

      app.moveSubtask(id, eggs, 0);
      expect(shape()).toEqual(["a dozen eggs", "bread", "x milk"]);

      // A tick is one undo step; a write that changes nothing is none.
      app.setSubtaskDone(id, milk, true);
      app.undo();
      expect(shape()).toEqual(["bread", "x milk", "a dozen eggs"]);
      app.undo();
      app.undo();
      expect(shape()).toEqual(["bread", "milk", "eggs"]);

      app.removeSubtask(id, milk);
      app.removeSubtask(id, eggs);
      expect(shape()).toEqual(["bread"]);
      dispose();
    });
  });

  test("a grip drag lands half a row past each neighbour and stays in the list", () => {
    // Row 1 of 4, rows 28 px tall.
    expect(dragTarget(1, 0, 28, 4)).toEqual({ dy: 0, to: 1 });
    expect(dragTarget(1, 13, 28, 4)).toEqual({ dy: 13, to: 1 });
    expect(dragTarget(1, 15, 28, 4)).toEqual({ dy: 15, to: 2 });
    expect(dragTarget(1, -15, 28, 4)).toEqual({ dy: -15, to: 0 });
    expect(dragTarget(1, 500, 28, 4)).toEqual({ dy: 56, to: 3 });
    expect(dragTarget(1, -500, 28, 4)).toEqual({ dy: -28, to: 0 });
    expect(dragTarget(0, 40, 0, 4)).toEqual({ dy: 0, to: 0 });
  });

  test("subtasks survive the boot snapshot path", () => {
    createRoot((dispose) => {
      const doc = Doc.create();
      const id = doc.addItem("inbox", "from before boot");
      const sub = doc.addSubtask(id, "passport");
      doc.addSubtask(id, "charger");
      doc.setSubtaskDone(id, sub, true);
      const app = appOver(doc);
      expect(app.getItem(id)?.subtasks?.map((s) => [s.text, s.done === true])).toEqual([
        ["passport", true],
        ["charger", false],
      ]);
      dispose();
    });
  });
});
