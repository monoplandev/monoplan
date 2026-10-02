// The reserved `events` list in the web store (`spec/events-plan.md`):
// an event is an item located in `events`, moving is converting, and the
// status controls collapse an event's open states into one choice.

import { describe, expect, test } from "bun:test";

import { Dek, Doc, SyncEngine } from "@monoplan/core/wasm";
import type { EngineStorage } from "@monoplan/core/wasm";
import { MemEngineStorage } from "../../core/test/mem-engine-storage.ts";
import { findResultListLabel } from "../src/findResults.tsx";
import {
  createSyncedApp,
  EVENT_STATES,
  isEvent,
  isOpen,
  LIST_EVENTS,
  statusValue,
  type ItemView,
} from "../src/sync/store.ts";

const DOC_ID = "00000000-0000-0000-0000-000000000000";

function engineFrom(doc: Doc): SyncEngine {
  return new SyncEngine(
    doc,
    DOC_ID,
    Dek.generate(),
    0n,
    "test",
    "0",
    new MemEngineStorage() as unknown as EngineStorage,
  );
}

describe("store events", () => {
  test("an item added to `events` is an event with no ListMeta row", () => {
    const app = createSyncedApp(engineFrom(Doc.create()));
    const id = app.addItem(LIST_EVENTS, "gig");
    const it = app.getItem(id)!;
    expect(isEvent(it)).toBe(true);
    expect(isOpen(it)).toBe(true);
    // No `when` required: it is unscheduled, not rejected.
    expect(it.when).toBeUndefined();
    expect(app.state.listOpen[LIST_EVENTS]).toEqual([id]);
    expect(app.state.listsOrder).toEqual([]);
    expect(app.state.listsById[LIST_EVENTS]).toBeUndefined();
  });

  test("moving is converting, both ways, with dates and state intact", () => {
    const app = createSyncedApp(engineFrom(Doc.create()));
    const id = app.addItem("inbox", "dentist");
    app.setItemWhen(id, "2026-10-09T09:30");
    app.setLifecycle(id, "in_progress");
    expect(isEvent(app.getItem(id)!)).toBe(false);

    app.moveItem(id, LIST_EVENTS, 0);
    const event = app.getItem(id)!;
    expect(isEvent(event)).toBe(true);
    expect(event.when).toBe("2026-10-09T09:30");
    // The open state rides along underneath; the picker shows one choice.
    expect(event.state).toBe("in_progress");
    expect(statusValue(event)).toBe("backlog");
    expect(app.state.listOpen["inbox"] ?? []).toEqual([]);

    app.moveItem(id, "inbox", 0);
    const task = app.getItem(id)!;
    expect(isEvent(task)).toBe(false);
    expect(task.state).toBe("in_progress");
    expect(statusValue(task)).toBe("in_progress");
    expect(task.when).toBe("2026-10-09T09:30");
  });

  test("closing and reopening an event are ordinary lifecycle writes", () => {
    const app = createSyncedApp(engineFrom(Doc.create()));
    const id = app.addItem(LIST_EVENTS, "gig");
    app.setDone(id, true);
    expect(statusValue(app.getItem(id)!)).toBe("done");
    app.setLifecycle(id, "cancelled");
    expect(statusValue(app.getItem(id)!)).toBe("cancelled");
    // Reopen: back to the single open choice (Backlog underneath).
    app.setDone(id, false);
    const reopened = app.getItem(id)!;
    expect(reopened.state).toBe("backlog");
    expect(statusValue(reopened)).toBe("backlog");
    expect(isEvent(reopened)).toBe(true);
  });

  test("the event status choices are the open stand-in plus the closed states", () => {
    expect(EVENT_STATES).toEqual(["backlog", "done", "cancelled"]);
  });

  test("find results label an event's list with the built-in name", () => {
    const app = createSyncedApp(engineFrom(Doc.create()));
    const id = app.addItem(LIST_EVENTS, "gig");
    const it: ItemView = app.getItem(id)!;
    expect(
      findResultListLabel(
        app,
        { inbox: "Inbox", events: "Events" },
        { kind: "item", id, title: it.text, score: 0, listId: it.listId },
      ),
    ).toBe("Events");
  });
});
