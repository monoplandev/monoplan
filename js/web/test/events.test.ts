// The `event` flag in the web store (`spec/events-plan.md`): a dated item
// marked as an event leaves its list's Open array (it shows on the
// calendar instead), keeps its list and state, and the status controls
// collapse its open states into one choice.

import { describe, expect, test } from "bun:test";

import { Dek, Doc, Replica } from "@monoplan/core/wasm";
import type { ReplicaStorage } from "@monoplan/core/wasm";
import { MemReplicaStorage } from "../../core/test/mem-replica-storage.ts";
import {
  createSyncedApp,
  EVENT_STATES,
  isEvent,
  isListed,
  isOpen,
  statusValue,
} from "../src/sync/store.ts";

const DOC_ID = "00000000-0000-0000-0000-000000000000";

function replicaFrom(doc: Doc): Replica {
  return new Replica(
    doc,
    DOC_ID,
    Dek.generate(),
    0n,
    new MemReplicaStorage() as unknown as ReplicaStorage,
  );
}

describe("store events", () => {
  test("marking a dated item an event takes it out of its list and back", () => {
    const app = createSyncedApp(replicaFrom(Doc.create()));
    const list = app.addList("Trip");
    const a = app.addItem(list, "pack");
    const b = app.addItem(list, "flight out");
    const c = app.addItem(list, "book hotel");
    app.setItemWhen(b, "2026-12-01T09:30");
    app.setLifecycle(b, "in_progress");

    app.setItemEvent(b, true);
    const event = app.getItem(b)!;
    expect(isEvent(event)).toBe(true);
    expect(isOpen(event)).toBe(true);
    expect(isListed(event)).toBe(false);
    // It keeps its list; only the list's rows lose it.
    expect(event.listId).toBe(list);
    expect(app.state.listOpen[list]).toEqual([a, c]);
    // The open state rides along underneath; the picker shows one choice.
    expect(event.state).toBe("in_progress");
    expect(statusValue(event)).toBe("backlog");

    app.setItemEvent(b, false);
    const task = app.getItem(b)!;
    expect(isEvent(task)).toBe(false);
    expect(statusValue(task)).toBe("in_progress");
    expect(task.when).toBe("2026-12-01T09:30");
    expect(app.state.listOpen[list]).toEqual([a, b, c]);
  });

  test("an event needs a date, and loses the flag with it", () => {
    const app = createSyncedApp(replicaFrom(Doc.create()));
    const id = app.addItem("inbox", "dinner, date tbc");
    // No date: nothing to mark. Not an error, not an undo step.
    app.setItemEvent(id, true);
    expect(isEvent(app.getItem(id)!)).toBe(false);
    expect(app.state.listOpen["inbox"]).toEqual([id]);

    app.setItemWhen(id, "2026-12-01");
    app.setItemEvent(id, true);
    expect(isEvent(app.getItem(id)!)).toBe(true);
    expect(app.state.listOpen["inbox"] ?? []).toEqual([]);

    // Clearing the date brings it back as a task, and a new date does
    // not make it an event again.
    app.setItemWhen(id, null);
    expect(isEvent(app.getItem(id)!)).toBe(false);
    expect(app.state.listOpen["inbox"]).toEqual([id]);
    app.setItemWhen(id, "2026-12-02");
    expect(isEvent(app.getItem(id)!)).toBe(false);
  });

  test("undo puts an event back in its list, at its place", () => {
    const app = createSyncedApp(replicaFrom(Doc.create()));
    const list = app.addList("Trip");
    const a = app.addItem(list, "pack");
    const b = app.addItem(list, "gig");
    const c = app.addItem(list, "book hotel");
    app.setItemWhen(b, "2026-12-01");
    app.setItemEvent(b, true);
    expect(app.state.listOpen[list]).toEqual([a, c]);
    app.undo();
    expect(isEvent(app.getItem(b)!)).toBe(false);
    expect(app.state.listOpen[list]).toEqual([a, b, c]);
    app.redo();
    expect(isEvent(app.getItem(b)!)).toBe(true);
    expect(app.state.listOpen[list]).toEqual([a, c]);
    // Undo the flag and the date together: still a row, in place.
    app.undo();
    app.undo();
    expect(app.getItem(b)!.when).toBeUndefined();
    expect(app.state.listOpen[list]).toEqual([a, b, c]);
  });

  test("closing and reopening an event are ordinary lifecycle writes", () => {
    const app = createSyncedApp(replicaFrom(Doc.create()));
    const id = app.addItem("inbox", "gig");
    app.setItemWhen(id, "2026-12-01");
    app.setItemEvent(id, true);
    app.setDone(id, true);
    expect(statusValue(app.getItem(id)!)).toBe("done");
    app.setLifecycle(id, "cancelled");
    expect(statusValue(app.getItem(id)!)).toBe("cancelled");
    // Reopen: back to the single open choice (Backlog underneath), still
    // an event, still out of the list.
    app.setDone(id, false);
    const reopened = app.getItem(id)!;
    expect(reopened.state).toBe("backlog");
    expect(statusValue(reopened)).toBe("backlog");
    expect(isEvent(reopened)).toBe(true);
    expect(app.state.listOpen["inbox"] ?? []).toEqual([]);
  });

  test("the event status choices are the open stand-in plus the closed states", () => {
    expect(EVENT_STATES).toEqual(["backlog", "done", "cancelled"]);
  });
});
