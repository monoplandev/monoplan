// Local search engine. Spec coverage from spec/search.md "Testing":
// tokenization examples, incremental updates, multi-token AND, prefix
// on the last token, list-rename context propagation, and the
// live/done/binned ranking tiebreaker.
//
// Events are sourced from a real SyncEngine so the index is exercised
// against the same AppEvent stream the workspace dispatches in
// production — no bespoke test-only mutation paths.

import { describe, expect, test } from "bun:test";

import { Dek, Doc, SyncEngine } from "@monoplan/core/wasm";
import type { EngineStorage } from "@monoplan/core/wasm";
import { MemEngineStorage } from "../../core/test/mem-engine-storage.ts";
import {
  createSearchEngine,
  tokenize,
  type SearchEngine,
} from "../src/search.ts";

const LIST_MAIN = "inbox";

function newSearch(): { eng: SyncEngine; search: SearchEngine } {
  // Search only consumes the engine's AppEvent stream — it never
  // captures or syncs — but the constructor now requires a storage.
  const eng = new SyncEngine(
    Doc.create(),
    "00000000-0000-0000-0000-000000000000",
    Dek.generate(),
    0n,
    "t",
    "0",
    new MemEngineStorage() as unknown as EngineStorage,
  );
  const search = createSearchEngine();
  // Same path the web store uses on attach: feed the synthetic burst
  // describing current doc state through the dispatcher.
  for (const ev of eng.snapshotEvents()) search.apply(ev);
  return { eng, search };
}

function pump(eng: SyncEngine, search: SearchEngine): void {
  while (true) {
    const ev = eng.popAppEvent();
    if (!ev) break;
    search.apply(ev);
  }
}

describe("tokenize", () => {
  test("spec examples", () => {
    expect(tokenize("Buy groceries")).toEqual(["buy", "groceries"]);
    expect(tokenize("PR #142")).toEqual(["pr", "142"]);
    expect(tokenize("Q3 roadmap")).toEqual(["q3", "roadmap"]);
  });

  test("lowercases and de-duplicates within a doc", () => {
    expect(tokenize("Foo FOO foo Bar")).toEqual(["foo", "bar"]);
  });

  test("punctuation-only / whitespace-only / empty input → []", () => {
    expect(tokenize("")).toEqual([]);
    expect(tokenize("   ")).toEqual([]);
    expect(tokenize("--- !!! ?? ")).toEqual([]);
  });

  test("NFKD normalizes fullwidth digits / letters", () => {
    expect(tokenize("Ｑ３")).toEqual(["q3"]);
  });

  test("folds accents so accent-free queries match accented text", () => {
    expect(tokenize("artículo")).toEqual(["articulo"]);
    expect(tokenize("Café Niño")).toEqual(["cafe", "nino"]);
    // Both sides fold to the same token, so either direction matches.
    expect(tokenize("articulo")).toEqual(tokenize("artículo"));
  });
});

describe("add → query", () => {
  test("a fresh item is reachable by one of its tokens", () => {
    const { eng, search } = newSearch();
    eng.addItem(LIST_MAIN, "Buy groceries");
    pump(eng, search);
    const r = search.query("groceries");
    expect(r.length).toBe(1);
    expect(r[0].kind).toBe("item");
    expect(r[0].title).toBe("Buy groceries");
  });
});

describe("edits", () => {
  test("text edit removes stale tokens and indexes the new ones", () => {
    const { eng, search } = newSearch();
    const id = eng.addItem(LIST_MAIN, "Buy groceries");
    pump(eng, search);
    eng.editItemText(id, "Read book");
    pump(eng, search);

    expect(search.query("groceries").length).toBe(0);
    const r = search.query("read");
    expect(r.length).toBe(1);
    expect(r[0].id).toBe(id);
    expect(r[0].title).toBe("Read book");
  });

  test("notes match at lower rank than title match for the same query", () => {
    const { eng, search } = newSearch();
    const titleHit = eng.addItem(LIST_MAIN, "phoenix kickoff");
    const notesHit = eng.addItem(LIST_MAIN, "another item");
    eng.applyNotesDelta(notesHit, JSON.stringify([{ insert: "see also: phoenix" }]));
    pump(eng, search);

    const r = search.query("phoenix");
    expect(r.map((x) => x.id)).toEqual([titleHit, notesHit]);
  });
});

describe("list rename", () => {
  test("updates the list result AND items that referenced its name", () => {
    const { eng, search } = newSearch();
    const listId = eng.addList("Work");
    const itemId = eng.addItem(listId, "ship feature");
    pump(eng, search);

    // Items in "Work" inherit "work" as a context token.
    let r = search.query("work");
    expect(r.some((x) => x.id === listId)).toBe(true);
    expect(r.some((x) => x.id === itemId)).toBe(true);

    eng.renameList(listId, "Personal");
    pump(eng, search);

    // Old name is gone from both the list result and item context.
    expect(search.query("work").length).toBe(0);

    r = search.query("personal");
    expect(r.some((x) => x.id === listId)).toBe(true);
    expect(r.some((x) => x.id === itemId)).toBe(true);
  });
});

describe("delete", () => {
  test("hard-delete removes the item from queries", () => {
    const { eng, search } = newSearch();
    const id = eng.addItem(LIST_MAIN, "Buy groceries");
    pump(eng, search);
    expect(search.query("groceries").length).toBe(1);

    eng.setItemBinned(id, true);
    eng.deleteBinned(id);
    pump(eng, search);
    expect(search.query("groceries").length).toBe(0);
  });
});

describe("multi-token AND", () => {
  test("every query token must match the same doc", () => {
    const { eng, search } = newSearch();
    eng.addItem(LIST_MAIN, "Buy groceries");
    eng.addItem(LIST_MAIN, "Read book");
    pump(eng, search);

    expect(search.query("buy groceries").length).toBe(1);
    // Tokens hit different docs — AND semantics → no result.
    expect(search.query("buy book").length).toBe(0);
  });
});

describe("last-token prefix", () => {
  test("partial trailing token matches by prefix on title", () => {
    const { eng, search } = newSearch();
    eng.addItem(LIST_MAIN, "Buy groceries");
    eng.addItem(LIST_MAIN, "Read Phoenix spec");
    eng.addItem(LIST_MAIN, "Plan team offsite");
    pump(eng, search);

    expect(search.query("buy gro").map((r) => r.title)).toEqual([
      "Buy groceries",
    ]);
    expect(search.query("pho").map((r) => r.title)).toEqual([
      "Read Phoenix spec",
    ]);
    expect(search.query("off").map((r) => r.title)).toEqual([
      "Plan team offsite",
    ]);
  });
});

describe("lifecycle tiebreak", () => {
  test("live > done > binned when textual buckets are equal", () => {
    const { eng, search } = newSearch();
    const liveId = eng.addItem(LIST_MAIN, "Apple");
    const doneId = eng.addItem(LIST_MAIN, "Apple");
    const binnedId = eng.addItem(LIST_MAIN, "Apple");
    eng.setItemDone(doneId, true);
    eng.setItemBinned(binnedId, true);
    pump(eng, search);

    const r = search.query("apple");
    expect(r.map((x) => x.id)).toEqual([liveId, doneId, binnedId]);
  });
});
