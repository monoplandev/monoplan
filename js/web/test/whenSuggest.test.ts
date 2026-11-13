// Date suggestions (`src/whenSuggest.ts`): which title edits raise one,
// which drop one, and the register in the store: written, answered yes
// (it becomes the `when`), answered no (it goes), cleared by a date set
// any other way. "Now" is Tue 2026-11-03 12:00.

import { describe, expect, test } from "bun:test";

import { Dek, Doc, Replica } from "@monoplan/core/wasm";
import type { ReplicaStorage } from "@monoplan/core/wasm";
import { MemReplicaStorage } from "../../core/test/mem-replica-storage.ts";
import { createSyncedApp } from "../src/sync/store.ts";
import {
  applySuggestionChange,
  raisedDate,
  suggestionChange,
  suggestionLabel,
  titleDate,
} from "../src/whenSuggest.ts";

const opts = { now: new Date(2026, 10, 3, 12, 0, 0).getTime(), locale: "en-AU" };
const keyOf = (title: string) => titleDate(title, opts)?.key ?? null;

describe("the rule", () => {
  test("a title's date phrase resolves once, with its wording as the key", () => {
    expect(titleDate("Dentist Friday 8:30", opts)).toEqual({
      when: "2026-11-06T08:30",
      key: "friday 8:30",
    });
    expect(titleDate("Dentist", opts)).toBeNull();
  });

  test("only a new wording raises a suggestion", () => {
    expect(raisedDate("dentist friday", null, opts)?.when).toBe("2026-11-06");
    // The phrase the title already had: asked, whatever the answer was.
    expect(raisedDate("dentist appt friday", keyOf("dentist friday"), opts)).toBeNull();
    expect(raisedDate("dentist saturday", keyOf("dentist friday"), opts)?.when).toBe(
      "2026-11-07",
    );
    expect(raisedDate("dentist friday night", keyOf("dentist friday"), opts)?.when).toBe(
      "2026-11-06T20:00",
    );
  });

  test("a title change sets, clears or leaves the suggestion", () => {
    expect(suggestionChange("call mum tomorrow", null, opts)).toEqual({ set: "2026-11-04" });
    expect(suggestionChange("call mum", keyOf("call mum tomorrow"), opts)).toBe("clear");
    expect(suggestionChange("ring mum tomorrow", keyOf("call mum tomorrow"), opts)).toBeNull();
    expect(suggestionChange("call mum", null, opts)).toBeNull();
  });

  test("the label reads as the date badge does", () => {
    const labels = { today: "Today", tomorrow: "Tomorrow" };
    expect(suggestionLabel("2026-11-04", opts.now, "en-AU", labels)).toBe("Tomorrow");
    expect(suggestionLabel("2026-11-04T08:30", opts.now, "en-AU", labels)).toContain(
      "Tomorrow",
    );
  });
});

const DOC_ID = "00000000-0000-0000-0000-000000000000";

function appWith(title: string) {
  const doc = Doc.create();
  const id = doc.addItem("inbox", title);
  const replica = new Replica(
    doc,
    DOC_ID,
    Dek.generate(),
    0n,
    new MemReplicaStorage() as unknown as ReplicaStorage,
  );
  return { app: createSyncedApp(replica), id };
}

describe("the register", () => {
  test("hydrates from the attach burst", () => {
    const doc = Doc.create();
    const id = doc.addItem("inbox", "dentist friday");
    doc.setItemWhenSuggested(id, "2026-11-06");
    const replica = new Replica(
      doc,
      DOC_ID,
      Dek.generate(),
      0n,
      new MemReplicaStorage() as unknown as ReplicaStorage,
    );
    const app = createSyncedApp(replica);
    expect(app.state.itemsById[id]?.whenSuggested).toBe("2026-11-06");
  });

  test("yes makes it the date, in one undo step", () => {
    const { app, id } = appWith("dentist friday 8:30");
    app.setItemWhenSuggested(id, "2026-11-06T08:30");
    expect(app.state.itemsById[id]?.whenSuggested).toBe("2026-11-06T08:30");
    expect(app.state.itemsById[id]?.when).toBeUndefined();

    app.acceptItemWhenSuggestion(id);
    expect(app.state.itemsById[id]?.when).toBe("2026-11-06T08:30");
    expect(app.state.itemsById[id]?.whenSuggested).toBeUndefined();

    app.undo();
    expect(app.state.itemsById[id]?.when).toBeUndefined();
    expect(app.state.itemsById[id]?.whenSuggested).toBe("2026-11-06T08:30");
  });

  test("no clears it", () => {
    const { app, id } = appWith("dentist friday");
    app.setItemWhenSuggested(id, "2026-11-06");
    app.setItemWhenSuggested(id, null);
    expect(app.state.itemsById[id]?.whenSuggested).toBeUndefined();
    expect(app.state.itemsById[id]?.when).toBeUndefined();
  });

  test("a date set any other way answers it", () => {
    const { app, id } = appWith("dentist friday");
    app.setItemWhenSuggested(id, "2026-11-06");
    app.setItemWhen(id, "2026-12-01");
    expect(app.state.itemsById[id]?.when).toBe("2026-12-01");
    expect(app.state.itemsById[id]?.whenSuggested).toBeUndefined();
  });

  test("the core rejects a malformed value", () => {
    const doc = Doc.create();
    const id = doc.addItem("inbox", "task");
    expect(() => doc.setItemWhenSuggested(id, "friday")).toThrow();
  });
});

describe("applying a title change", () => {
  const view = (app: ReturnType<typeof appWith>["app"], id: string) => {
    const it = app.state.itemsById[id]!;
    return { when: it.when, whenSuggested: it.whenSuggested, closed: false };
  };

  test("a new phrase is written, a declined one is not asked again", () => {
    const { app, id } = appWith("dentist");
    applySuggestionChange(app, id, view(app, id), "dentist friday", null, opts);
    expect(app.state.itemsById[id]?.whenSuggested).toBe("2026-11-06");

    // No, then a typo fix elsewhere in the title.
    app.setItemWhenSuggested(id, null);
    applySuggestionChange(
      app,
      id,
      view(app, id),
      "the dentist friday",
      keyOf("dentist friday"),
      opts,
    );
    expect(app.state.itemsById[id]?.whenSuggested).toBeUndefined();
  });

  test("a phrase typed out takes its suggestion along", () => {
    const { app, id } = appWith("dentist friday");
    app.setItemWhenSuggested(id, "2026-11-06");
    applySuggestionChange(app, id, view(app, id), "dentist", keyOf("dentist friday"), opts);
    expect(app.state.itemsById[id]?.whenSuggested).toBeUndefined();
  });

  test("an item with a date, or a closed one, takes none", () => {
    const { app, id } = appWith("dentist");
    app.setItemWhen(id, "2026-12-01");
    applySuggestionChange(app, id, view(app, id), "dentist friday", null, opts);
    expect(app.state.itemsById[id]?.whenSuggested).toBeUndefined();

    const other = appWith("dentist");
    applySuggestionChange(
      other.app,
      other.id,
      { closed: true },
      "dentist friday",
      null,
      opts,
    );
    expect(other.app.state.itemsById[other.id]?.whenSuggested).toBeUndefined();
  });
});
