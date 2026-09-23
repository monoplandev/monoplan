import { expect, test } from "bun:test";

import { Dek, Doc, EncryptedBlob, SyncEngine } from "@monoplan/core/wasm";
import type { EngineStorage } from "@monoplan/core/wasm";
import { MemEngineStorage } from "../../core/test/mem-engine-storage.ts";
import { createSyncedApp } from "../src/sync/store.ts";

const DOC_ID = "00000000-0000-0000-0000-000000000000";
const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));

// The notes delta path bypasses `mutate`, so its durability rides on the
// store's idle timer: a burst of `applyNotesDelta` calls must become a
// captured WAL row without any other mutation or an explicit flush.
test("typed notes deltas are captured to the WAL on the idle timer", async () => {
  const dek = Dek.generate();
  const storage = new MemEngineStorage();
  const engine = new SyncEngine(
    Doc.create(),
    DOC_ID,
    dek.clone(),
    0n,
    "test",
    "0",
    storage as unknown as EngineStorage,
  );
  const app = createSyncedApp(engine);
  // Same wiring as `createWorkspaceRuntime`.
  app.setBeforeFlush(() => {
    engine.captureLocalOps();
  });

  const id = app.addItem("inbox", "item");
  const rowsAfterAdd = storage.ops.length;
  expect(rowsAfterAdd).toBeGreaterThan(0);

  expect(app.subscribeNotes(id)).toBe("");
  app.applyNotesDelta(id, [{ insert: "h" }]);
  app.applyNotesDelta(id, [{ retain: 1 }, { insert: "e" }]);
  app.applyNotesDelta(id, [{ retain: 2 }, { insert: "y" }]);
  // Store and search see it immediately; nothing captured yet.
  expect(app.state.itemsById[id]?.notes).toBe("hey");
  expect(storage.ops.length).toBe(rowsAfterAdd);

  await sleep(450);
  expect(storage.ops.length).toBe(rowsAfterAdd + 1);

  // The captured rows rebuild the notes on a fresh doc (what boot does).
  const fresh = Doc.empty();
  for (const op of storage.ops) {
    fresh.applyRemote(dek, new EncryptedBlob(op.nonce, op.ciphertext));
  }
  expect(JSON.parse(fresh.getItemJson(id) ?? "{}").notes).toBe("hey");

  // A follow-up burst is its own row; `flushNotes` runs it now.
  app.applyNotesDelta(id, [{ retain: 3 }, { insert: "!" }]);
  app.flushNotes();
  expect(storage.ops.length).toBe(rowsAfterAdd + 2);
  app.unsubscribeNotes(id);
});

// Keystrokes never reach the core's UndoManager (origin-excluded), so
// once the editor blurs the store records the session's net change as
// one workspace undo step of its own. It must survive moving to another
// item, revert exactly once, and redo.
test("a notes editing session is one workspace undo step after blur", () => {
  const engine = new SyncEngine(
    Doc.create(),
    DOC_ID,
    Dek.generate(),
    0n,
    "test",
    "0",
    new MemEngineStorage() as unknown as EngineStorage,
  );
  const app = createSyncedApp(engine);
  const a = app.addItem("inbox", "a");
  const b = app.addItem("inbox", "b");
  // An earlier whole-value write put "og" there: one workspace step.
  app.setItemNotes(a, "og");
  expect(app.state.itemsById[a]?.notes).toBe("og");

  // Type "e1" over "og" in a few edits, then blur.
  expect(app.subscribeNotes(a)).toBe("og");
  app.applyNotesDelta(a, [{ delete: 2 }]);
  app.applyNotesDelta(a, [{ insert: "e" }]);
  app.applyNotesDelta(a, [{ retain: 1 }, { insert: "1" }]);
  expect(app.state.itemsById[a]?.notes).toBe("e1");
  app.endNotesSession();

  // Move to another item: the session is gone, the step is not.
  app.unsubscribeNotes(a);
  expect(app.subscribeNotes(b)).toBe("");

  const inbound: Array<[string, unknown]> = [];
  app.onNotesDelta((id, ops) => inbound.push([id, ops]));

  expect(app.undo()).toBe(true);
  expect(app.state.itemsById[a]?.notes).toBe("og");
  // Subscribed editors hear about it as a delta on that item.
  expect(inbound).toEqual([[a, [{ delete: 2 }, { insert: "og" }]]]);
  // The next undo is the earlier whole-value write, not this one again.
  expect(app.undo()).toBe(true);
  expect(app.state.itemsById[a]?.notes).toBe("");

  expect(app.redo()).toBe(true);
  expect(app.state.itemsById[a]?.notes).toBe("og");
  expect(app.redo()).toBe(true);
  expect(app.state.itemsById[a]?.notes).toBe("e1");
  expect(app.canRedo()).toBe(false);
  app.unsubscribeNotes(b);
});

// A session that ends where it began (undone natively while focused)
// records nothing; a blur without any edit records nothing either.
test("an unchanged notes session records no undo step", () => {
  const engine = new SyncEngine(
    Doc.create(),
    DOC_ID,
    Dek.generate(),
    0n,
    "test",
    "0",
    new MemEngineStorage() as unknown as EngineStorage,
  );
  const app = createSyncedApp(engine);
  const a = app.addItem("inbox", "a");
  // Consume the add so the stack is empty.
  expect(app.undo()).toBe(true);
  expect(app.redo()).toBe(true);
  expect(app.canUndo()).toBe(true);
  const stackHas = () => {
    // Probe: undoing pops exactly one entry; put it back with redo.
    const did = app.undo();
    if (did) app.redo();
    return did;
  };

  app.subscribeNotes(a);
  app.endNotesSession();
  app.applyNotesDelta(a, [{ insert: "x" }]);
  app.applyNotesDelta(a, [{ delete: 1 }]);
  app.endNotesSession();
  app.unsubscribeNotes(a);
  // Only the add is undoable: one undo empties the stack.
  expect(app.undo()).toBe(true);
  expect(app.canUndo()).toBe(false);
  expect(stackHas()).toBe(false);
});

// A whole-value notes write inside an action batch (duplicate, or a
// capture that sets notes with its add) joins the batch's step: one undo
// removes the item, one redo brings it back with its notes.
test("notes written inside an action batch ride the batch's undo step", () => {
  const engine = new SyncEngine(
    Doc.create(),
    DOC_ID,
    Dek.generate(),
    0n,
    "test",
    "0",
    new MemEngineStorage() as unknown as EngineStorage,
  );
  const app = createSyncedApp(engine);
  const a = app.withActionBatch(() => {
    const id = app.addItem("inbox", "a");
    app.setItemNotes(id, "og");
    return id;
  });
  expect(app.state.itemsById[a]?.notes).toBe("og");
  expect(app.undo()).toBe(true);
  expect(app.state.itemsById[a]).toBeUndefined();
  expect(app.redo()).toBe(true);
  expect(app.state.itemsById[a]?.notes).toBe("og");
  expect(app.canUndo()).toBe(true);
  expect(app.canRedo()).toBe(false);

  // Outside a batch, a whole-value write on an existing item is its own
  // step, and the capture form's separate add stays a separate step.
  const b = app.addItem("inbox", "b");
  app.setItemNotes(b, "hello");
  expect(app.undo()).toBe(true);
  expect(app.state.itemsById[b]?.notes).toBe("");
  expect(app.undo()).toBe(true);
  expect(app.state.itemsById[b]).toBeUndefined();
  expect(app.redo()).toBe(true);
  expect(app.redo()).toBe(true);
  expect(app.state.itemsById[b]?.notes).toBe("hello");
});

// The capture form's shape: add, notes, deadline, focus, done inside one
// batch. One undo removes the item; one redo restores every field.
test("a full capture is one undo step and redo restores its fields", () => {
  const engine = new SyncEngine(
    Doc.create(),
    DOC_ID,
    Dek.generate(),
    0n,
    "test",
    "0",
    new MemEngineStorage() as unknown as EngineStorage,
  );
  const app = createSyncedApp(engine);
  const id = app.withActionBatch(() => {
    const id = app.addItemAt("inbox", "capture", 0);
    app.setItemNotes(id, "some notes");
    app.setItemDeadline(id, "2026-10-01");
    app.addToFocus(id);
    return id;
  });
  expect(app.state.itemsById[id]?.deadline).toBe("2026-10-01");
  expect(app.undo()).toBe(true);
  expect(app.state.itemsById[id]).toBeUndefined();
  expect(app.canUndo()).toBe(false);
  expect(app.redo()).toBe(true);
  const it = app.state.itemsById[id];
  expect(it?.text).toBe("capture");
  expect(it?.notes).toBe("some notes");
  expect(it?.deadline).toBe("2026-10-01");
  expect(app.state.focusOrder).toContain(id);
});
