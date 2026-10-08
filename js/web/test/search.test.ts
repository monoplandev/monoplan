// Search boundary. The engine itself lives in core (`core/src/search.rs`,
// covered by `core/tests/search.rs`); this file checks that the wasm
// exports the web consumes round-trip: the shared tokenizer, the name
// filter, and the JSON query surface the store wraps. Mutations go
// through a real SyncEngine, as in production.

import { describe, expect, test } from "bun:test";

import { Dek, Doc, SyncEngine } from "@monoplan/core/wasm";
import type { EngineStorage } from "@monoplan/core/wasm";
import { MemEngineStorage } from "../../core/test/mem-engine-storage.ts";
import {
  createSearchEngine,
  matchesName,
  tokenize,
  type SearchEngine,
} from "../src/search.ts";

const LIST_MAIN = "inbox";

function newSearch(): { eng: SyncEngine; search: SearchEngine } {
  const eng = new SyncEngine(
    Doc.create(),
    "00000000-0000-0000-0000-000000000000",
    Dek.generate(),
    0n,
    "t",
    "0",
    new MemEngineStorage() as unknown as EngineStorage,
  );
  return { eng, search: createSearchEngine(eng) };
}

describe("tokenize (wasm)", () => {
  test("spec examples", () => {
    expect(tokenize("Buy groceries")).toEqual(["buy", "groceries"]);
    expect(tokenize("PR #142")).toEqual(["pr", "142"]);
    expect(tokenize("Q3 roadmap")).toEqual(["q3", "roadmap"]);
  });

  test("folds case, width and accents; drops empties", () => {
    expect(tokenize("Foo FOO foo Bar")).toEqual(["foo", "bar"]);
    expect(tokenize("ＰＲ １４２")).toEqual(["pr", "142"]);
    expect(tokenize("artículo")).toEqual(["articulo"]);
    expect(tokenize("")).toEqual([]);
    expect(tokenize("  #! ")).toEqual([]);
  });
});

describe("matchesName (wasm)", () => {
  test("every query token prefixes some name token, any order", () => {
    expect(matchesName("Work projects", "")).toBe(true);
    expect(matchesName("Work projects", "pro wo")).toBe(true);
    expect(matchesName("Récits", "rec")).toBe(true);
    expect(matchesName("Work projects", "home")).toBe(false);
  });
});

describe("query (wasm JSON surface)", () => {
  test("result shape round-trips for items and lists", () => {
    const { eng, search } = newSearch();
    const listId = eng.addList("Work");
    const itemId = eng.addItem(listId, "ship feature");
    eng.applyNotesDelta(itemId, JSON.stringify([{ insert: "by friday" }]));

    const r = search.query("ship");
    expect(r).toEqual([
      {
        id: itemId,
        kind: "item",
        title: "ship feature",
        body: "by friday",
        listId,
        lifecycle: "backlog",
        score: expect.any(Number),
      },
    ]);

    const lists = search.query("work").filter((x) => x.kind === "list");
    expect(lists).toEqual([
      { id: listId, kind: "list", title: "Work", score: expect.any(Number) },
    ]);
  });

  test("tracks the doc without the store draining events", () => {
    const { eng, search } = newSearch();
    const id = eng.addItem(LIST_MAIN, "Buy groceries");
    expect(search.query("groceries").map((x) => x.id)).toEqual([id]);
    eng.editItemText(id, "Read book");
    expect(search.query("groceries")).toEqual([]);
    expect(search.query("read").map((x) => x.id)).toEqual([id]);
  });

  test("lifecycle names and ordering cross the boundary", () => {
    const { eng, search } = newSearch();
    const live = eng.addItem(LIST_MAIN, "Apple");
    const done = eng.addItem(LIST_MAIN, "Apple");
    const binned = eng.addItem(LIST_MAIN, "Apple");
    eng.setItemDone(done, true);
    eng.setItemBinned(binned, true);
    const r = search.query("apple");
    expect(r.map((x) => [x.id, x.lifecycle])).toEqual([
      [live, "backlog"],
      [done, "done"],
      [binned, "binned"],
    ]);
  });

  test("blank input short-circuits; limit is honoured", () => {
    const { eng, search } = newSearch();
    for (let i = 0; i < 5; i++) eng.addItem(LIST_MAIN, `Apple ${i}`);
    expect(search.query("   ")).toEqual([]);
    expect(search.query("apple", 2).length).toBe(2);
  });
});
