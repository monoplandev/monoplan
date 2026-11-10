// Search surface over the core replica (spec/search.md). The index itself
// lives in Rust: core keeps it current from its own event funnel and
// reconciles against the doc on every query, so this module is only the
// typed boundary. `tokenize` / `matchesName` are re-exported from wasm so
// pickers and the emoji index fold text exactly the way the index does.

import {
  matchesName as wasmMatchesName,
  tokenize as wasmTokenize,
  type Replica,
} from "@monoplan/core/wasm";
import type { Lifecycle } from "./sync/store.ts";

export type SearchKind = "item" | "list";
/** Resolved lifecycle used for ranking/filtering (`spec/search.md`). */
export type SearchLifecycle = Lifecycle;

export interface SearchResult {
  id: string;
  kind: SearchKind;
  /** The doc the result came from. Stamped by the Find surfaces' fan-out
   *  (`findResults.ts`), never by the replica, whose index is per doc. */
  docId?: string;
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

/** Query surface bound to a replica. Each call crosses the wasm boundary
 *  once and parses one JSON array; the replica does the rest. */
export function createSearchEngine(replica: Replica): SearchEngine {
  return {
    query(input, limit = 50) {
      if (!input.trim()) return [];
      return JSON.parse(replica.searchJson(input, limit)) as SearchResult[];
    },
  };
}
