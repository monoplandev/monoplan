// The `event` flag and the Inbox date rule in the web store
// (`spec/events-plan.md`, `spec/inbox-plan.md`): the flag is a plain
// register that only changes what a passed date means and how the status
// reads; a date is what takes an item with no list out of Inbox.

import { describe, expect, test } from "bun:test";

import { Dek, Doc, Replica } from "@monoplan/core/wasm";
import type { ReplicaStorage } from "@monoplan/core/wasm";
import { MemReplicaStorage } from "../../core/test/mem-replica-storage.ts";
import { createRoot } from "solid-js";
import { findResultListLabel } from "../src/findResults.tsx";
import { createDocsLayer } from "../src/sync/docs.ts";
import {
  createHomeApp,
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
  test("an event in a list stays a row there", () => {
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
    expect(isOpen(event) && isListed(event)).toBe(true);
    expect(app.state.listOpen[list]).toEqual([a, b, c]);
    // The open state rides along underneath; the picker shows one choice.
    expect(event.state).toBe("in_progress");
    expect(statusValue(event)).toBe("backlog");

    app.setItemEvent(b, false);
    const task = app.getItem(b)!;
    expect(isEvent(task)).toBe(false);
    expect(statusValue(task)).toBe("in_progress");
    expect(app.state.listOpen[list]).toEqual([a, b, c]);
  });

  test("an event needs a date, and loses the flag with it", () => {
    const app = createSyncedApp(replicaFrom(Doc.create()));
    const list = app.addList("Trip");
    const id = app.addItem(list, "dinner, date tbc");
    // No date: nothing to mark. Not an error, not an undo step.
    app.setItemEvent(id, true);
    expect(isEvent(app.getItem(id)!)).toBe(false);

    app.setItemWhen(id, "2026-12-01");
    app.setItemEvent(id, true);
    expect(isEvent(app.getItem(id)!)).toBe(true);

    // The flag goes with the date: a new date makes a task.
    app.setItemWhen(id, null);
    expect(isEvent(app.getItem(id)!)).toBe(false);
    app.setItemWhen(id, "2026-12-02");
    expect(isEvent(app.getItem(id)!)).toBe(false);
    expect(app.state.listOpen[list]).toEqual([id]);
  });

  test("closing and reopening an event are ordinary lifecycle writes", () => {
    const app = createSyncedApp(replicaFrom(Doc.create()));
    const list = app.addList("Trip");
    const id = app.addItem(list, "gig");
    app.setItemWhen(id, "2026-12-01");
    app.setItemEvent(id, true);
    app.setDone(id, true);
    expect(statusValue(app.getItem(id)!)).toBe("done");
    expect(app.state.listOpen[list] ?? []).toEqual([]);
    app.setLifecycle(id, "cancelled");
    expect(statusValue(app.getItem(id)!)).toBe("cancelled");
    // Reopen: back to the single open choice (Backlog underneath).
    app.setDone(id, false);
    const reopened = app.getItem(id)!;
    expect(reopened.state).toBe("backlog");
    expect(statusValue(reopened)).toBe("backlog");
    expect(isEvent(reopened)).toBe(true);
    expect(app.state.listOpen[list]).toEqual([id]);
  });

  test("the event status choices are the open stand-in plus the closed states", () => {
    expect(EVENT_STATES).toEqual(["backlog", "done", "cancelled"]);
  });
});

describe("Inbox holds no dated items", () => {
  test("a date takes an Inbox item out and clearing it puts it back in place", () => {
    const app = createSyncedApp(replicaFrom(Doc.create()));
    const a = app.addItem("inbox", "a");
    const b = app.addItem("inbox", "b");
    const c = app.addItem("inbox", "c");

    app.setItemWhen(b, "2026-12-01");
    expect(isOpen(app.getItem(b)!)).toBe(true);
    expect(isListed(app.getItem(b)!)).toBe(false);
    expect(app.state.listOpen["inbox"]).toEqual([a, c]);
    // Another date, an event or not: still out.
    app.setItemWhen(b, "2026-12-02T10:00");
    app.setItemEvent(b, true);
    app.setItemEvent(b, false);
    expect(app.state.listOpen["inbox"]).toEqual([a, c]);

    app.setItemWhen(b, null);
    expect(app.state.listOpen["inbox"]).toEqual([a, b, c]);
  });

  test("filing a dated item gives it a row; moving it to Inbox takes it away", () => {
    const app = createSyncedApp(replicaFrom(Doc.create()));
    const list = app.addList("Trip");
    const x = app.addItem(list, "x");
    const id = app.addItem("inbox", "gig");
    app.setItemWhen(id, "2026-12-01");
    expect(app.state.listOpen["inbox"] ?? []).toEqual([]);

    app.moveItem(id, list, 0);
    expect(app.state.listOpen[list]).toEqual([id, x]);
    app.moveItem(id, "inbox", 0);
    expect(app.state.listOpen[list]).toEqual([x]);
    expect(app.state.listOpen["inbox"] ?? []).toEqual([]);
  });

  test("closing and reopening a dated Inbox item never lists it", () => {
    const app = createSyncedApp(replicaFrom(Doc.create()));
    const id = app.addItem("inbox", "gig");
    app.setItemWhen(id, "2026-12-01");
    app.setDone(id, true);
    app.setDone(id, false);
    expect(app.state.listOpen["inbox"] ?? []).toEqual([]);
  });

  test("undo and redo of a date move the Inbox row out and back, in place", () => {
    const app = createSyncedApp(replicaFrom(Doc.create()));
    const a = app.addItem("inbox", "a");
    const b = app.addItem("inbox", "b");
    const c = app.addItem("inbox", "c");
    app.setItemWhen(b, "2026-12-01");
    expect(app.state.listOpen["inbox"]).toEqual([a, c]);
    app.undo();
    expect(app.getItem(b)!.when).toBeUndefined();
    expect(app.state.listOpen["inbox"]).toEqual([a, b, c]);
    app.redo();
    expect(app.state.listOpen["inbox"]).toEqual([a, c]);
  });

  test("a dated item with no list is labelled Calendar, not Inbox", () => {
    const app = createHomeApp(replicaFrom(Doc.create()));
    const dated = app.addItem("inbox", "gig");
    const plain = app.addItem("inbox", "buy tickets");
    app.setItemWhen(dated, "2026-12-01");
    const layer = createRoot(() => createDocsLayer(app, []));
    const label = (id: string) =>
      findResultListLabel(
        layer,
        { inbox: "Inbox", upcoming: "Calendar" },
        { kind: "item", id, title: "", score: 0, listId: "inbox" },
      );
    expect(label(dated)).toBe("Calendar");
    expect(label(plain)).toBe("Inbox");
  });
});
