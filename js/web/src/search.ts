// Search surface over the core engine (spec/search.md). The index itself
// lives in Rust: core keeps it current from its own event funnel and
// reconciles against the doc on every query, so this module is only the
// typed boundary. `tokenize` / `matchesName` are re-exported from wasm so
// pickers and the emoji index fold text exactly the way the index does.

import {
  matchesName as wasmMatchesName,
  tokenize as wasmTokenize,
  type SyncEngine,
} from "@monoplan/core/wasm";
import type { Lifecycle } from "./sync/store.ts";

export type SearchKind = "item" | "list";
/** Resolved lifecycle used for ranking/filtering (`spec/search.md`). */
export type SearchLifecycle = Lifecycle;

export interface SearchResult {
  id: string;
  kind: SearchKind;
  title: string;
  body?: string;
  listId?: string;
  lifecycle?: SearchLifecycle;
  score: number;
}

export interface SearchEngine {
  query(input: string, limit?: number): SearchResult[];
}

/** Fold text into search tokens: NFKD, accents stripped, lowercased,
 *  split on anything that is not a letter or number, de-duplicated. */
export function tokenize(input: string): string[] {
  if (!input) return [];
  return wasmTokenize(input);
}

/** Name-only filter predicate, for pickers that narrow a short list of
 *  names rather than querying the index (the task dialog's move-to-list
 *  popover). Every query token must prefix some token of `name`, in any
 *  order; an empty query matches everything. */
export function matchesName(name: string, query: string): boolean {
  return wasmMatchesName(name, query);
}

/** Query surface bound to an engine. Each call crosses the wasm boundary
 *  once and parses one JSON array; the engine does the rest. */
export function createSearchEngine(engine: SyncEngine): SearchEngine {
  return {
    query(input, limit = 50) {
      if (!input.trim()) return [];
      return JSON.parse(engine.searchJson(input, limit)) as SearchResult[];
    },
  };
}
