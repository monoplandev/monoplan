import { expect, test } from "bun:test";

import { Dek, Doc, Replica } from "@monoplan/core/wasm";
import type { ReplicaStorage } from "@monoplan/core/wasm";
import { MemReplicaStorage } from "../../core/test/mem-replica-storage.ts";
import { createSyncedApp } from "../src/sync/store.ts";

const DOC_ID = "00000000-0000-0000-0000-000000000000";

test("bulk bootstrap crosses the app boundary as one full resync", () => {
  const dek = Dek.generate();
  const source = Doc.create();
  source.addItemsAt(
    "inbox",
    Array.from({ length: 100 }, (_, i) => `bootstrap ${i}`),
    0,
  );
  const blob = source.pendingExport(dek);
  expect(blob).not.toBeNull();

  const target = Doc.empty();
  target.applyRemote(dek, blob!);
  const targetReplica = new Replica(
    target,
    DOC_ID,
    dek.clone(),
    0n,
    new MemReplicaStorage() as unknown as ReplicaStorage,
  );
  const first = targetReplica.popAppEvent();
  expect(first?.kind).toBe("fullResync");
  expect(targetReplica.popAppEvent()).toBeUndefined();

  // Recreate the same queued control event for the app-level assertion.
  const secondTarget = Doc.empty();
  secondTarget.applyRemote(dek, blob!);
  const replica = new Replica(
    secondTarget,
    DOC_ID,
    dek.clone(),
    0n,
    new MemReplicaStorage() as unknown as ReplicaStorage,
  );
  const app = createSyncedApp(replica);
  expect(Object.keys(app.state.itemsById)).toHaveLength(100);
  app.drainEvents();
  expect(Object.keys(app.state.itemsById)).toHaveLength(100);
  expect(app.state.listOpen.inbox).toHaveLength(100);
});

// Saved default views live in the doc, so a fresh boot has to read them
// back off the workspace snapshot — not just off the live event stream
// that carried the original save. Regression: the snapshot materializer
// dropped `defaultView`, so every reload fell back to the list lens.
test("a saved default view survives materializing a fresh store", () => {
  const newReplica = () =>
    new Replica(
      Doc.create(),
      DOC_ID,
      Dek.generate(),
      0n,
      new MemReplicaStorage() as unknown as ReplicaStorage,
    );

  const replica = newReplica();
  const listId = replica.addList("Work");
  replica.setDefaultView(listId, "board:backlog,todo,in_progress,review");
  replica.setDefaultView("inbox", "board");
  const saved = replica.save();

  // Reload the way a returning session does: rebuild the doc from the
  // persisted snapshot, then materialize the store off the fresh replica.
  const reloaded = new Replica(
    Doc.load(saved),
    DOC_ID,
    Dek.generate(),
    0n,
    new MemReplicaStorage() as unknown as ReplicaStorage,
  );
  const app = createSyncedApp(reloaded);
  expect(app.state.listsById[listId]?.defaultView).toBe("board:backlog,todo,in_progress,review");
  expect(app.state.settings.inboxView).toBe("board");

  // Clearing the default has to survive the same round trip as absence,
  // not linger as a stale register.
  reloaded.setDefaultView(listId, "");
  app.drainEvents();
  expect(app.state.listsById[listId]?.defaultView).toBeUndefined();
});
