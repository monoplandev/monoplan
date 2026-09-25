// Local plaintext search index over the active account doc. See
// spec/search.md — an in-memory inverted index, AND-across query
// tokens with the last token treated as prefix. Built once after the
// doc materializes and maintained incrementally from the same AppEvent
// stream the store dispatches.

import type { AppEventJs } from "@monoplan/core/wasm";
import {
  lifecycleOf as itemLifecycleOf,
  parseWorkflowState,
  type ItemView,
  type Lifecycle,
  type ListView,
  type WorkspaceState,
} from "./sync/store.ts";

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
  rebuild(state: WorkspaceState): void;
  apply(event: AppEventJs): void;
  query(input: string, limit?: number): SearchResult[];
}

/** Per-doc indexed view. Token sets are kept around so a field-level
 *  edit can `removePosting` the old set before inserting the new one. */
interface SearchDoc {
  id: string;
  kind: SearchKind;
  title: string;
  body: string;
  listId?: string;
  lifecycle?: SearchLifecycle;
  updatedAt: number;
  titleTokens: Set<string>;
  bodyTokens: Set<string>;
  contextTokens: Set<string>;
  tokens: Set<string>;
}

// Split on anything that is not a Unicode letter or number. Mirrors the
// spec examples ("PR #142" -> ["pr","142"], "Q3 roadmap" -> ["q3",
// "roadmap"]). NFKC + lowercase first so width / case variants collapse.
const TOKEN_SPLIT = /[^\p{L}\p{N}]+/u;

// Combining marks left behind by decomposition. Stripping them folds
// accents so an accent-free query matches accented text and vice versa
// ("articulo" <-> "artículo"). Runs on both indexed text and queries via
// the shared tokenizer, so both sides meet at the folded form. NFKD
// (compatibility decomposition) also keeps the width/variant folding the
// prior NFKC pass provided.
const COMBINING_MARKS = /\p{M}/gu;

export function tokenize(input: string): string[] {
  if (!input) return [];
  const normalized = input
    .normalize("NFKD")
    .replace(COMBINING_MARKS, "")
    .toLowerCase()
    .trim();
  if (!normalized) return [];
  const out: string[] = [];
  const seen = new Set<string>();
  for (const part of normalized.split(TOKEN_SPLIT)) {
    if (!part || seen.has(part)) continue;
    seen.add(part);
    out.push(part);
  }
  return out;
}

/** Name-only filter predicate, for pickers that narrow a short list of
 *  names rather than querying the index (the task dialog's move-to-list
 *  popover). Every query token must prefix some token of `name`, in any
 *  order; an empty query matches everything. Shares `tokenize`, so the
 *  case / accent / punctuation folding in spec/search.md applies here too. */
export function matchesName(name: string, query: string): boolean {
  const wanted = tokenize(query);
  if (wanted.length === 0) return true;
  const have = tokenize(name);
  return wanted.every((w) => have.some((t) => t.startsWith(w)));
}

function lifecycleOf(item: ItemView): SearchLifecycle {
  return itemLifecycleOf(item);
}

function lifecycleFromEvent(ev: AppEventJs): SearchLifecycle {
  if (ev.binnedAt != null) return "binned";
  return parseWorkflowState(ev.state);
}

// Rank order for tie-breaking (spec/search.md "Ranking"): in_progress,
// then review, then todo, then backlog, then done / cancelled, then
// binned — active work first, then queued, then closed.
function lifecycleRankOf(s: SearchLifecycle | undefined): number {
  switch (s) {
    case "in_progress":
      return 5;
    case "review":
      return 4;
    case "todo":
      return 3;
    case "backlog":
      return 2;
    case "done":
    case "cancelled":
      return 1;
    default:
      return 0;
  }
}

function bigToNum(v: bigint | number | undefined | null): number | undefined {
  if (v == null) return undefined;
  return typeof v === "bigint" ? Number(v) : v;
}

export function createSearchEngine(): SearchEngine {
  // Canonical per-id view of every indexed doc.
  const docsById = new Map<string, SearchDoc>();
  // token -> doc ids that contain it (in any field).
  const postings = new Map<string, Set<string>>();
  // listId -> item ids in that list. Lets a list rename / remove
  // re-index just the affected items rather than scanning every doc.
  const itemsByList = new Map<string, Set<string>>();
  // listId -> current name. Read when an item is inserted / its listId
  // changes so we can populate context tokens without holding a ref to
  // the list doc.
  const listNames = new Map<string, string>();

  function addPosting(token: string, id: string): void {
    let set = postings.get(token);
    if (!set) {
      set = new Set();
      postings.set(token, set);
    }
    set.add(id);
  }

  function removePosting(token: string, id: string): void {
    const set = postings.get(token);
    if (!set) return;
    set.delete(id);
    if (set.size === 0) postings.delete(token);
  }

  function indexDoc(doc: SearchDoc): void {
    docsById.set(doc.id, doc);
    for (const t of doc.tokens) addPosting(t, doc.id);
    if (doc.kind === "item" && doc.listId) {
      let s = itemsByList.get(doc.listId);
      if (!s) {
        s = new Set();
        itemsByList.set(doc.listId, s);
      }
      s.add(doc.id);
    }
  }

  function unindexDoc(id: string): SearchDoc | undefined {
    const doc = docsById.get(id);
    if (!doc) return undefined;
    for (const t of doc.tokens) removePosting(t, id);
    docsById.delete(id);
    if (doc.kind === "item" && doc.listId) {
      const s = itemsByList.get(doc.listId);
      if (s) {
        s.delete(id);
        if (s.size === 0) itemsByList.delete(doc.listId);
      }
    }
    return doc;
  }

  function makeItemDoc(args: {
    id: string;
    text: string;
    notes: string;
    listId: string;
    lifecycle: SearchLifecycle;
    updatedAt: number;
  }): SearchDoc {
    const titleTokens = new Set(tokenize(args.text));
    const bodyTokens = new Set(tokenize(args.notes));
    const ctxName = args.listId ? listNames.get(args.listId) ?? "" : "";
    const contextTokens = new Set(tokenize(ctxName));
    const tokens = new Set<string>();
    for (const t of titleTokens) tokens.add(t);
    for (const t of bodyTokens) tokens.add(t);
    for (const t of contextTokens) tokens.add(t);
    return {
      id: args.id,
      kind: "item",
      title: args.text,
      body: args.notes,
      listId: args.listId || undefined,
      lifecycle: args.lifecycle,
      updatedAt: args.updatedAt,
      titleTokens,
      bodyTokens,
      contextTokens,
      tokens,
    };
  }

  function makeListDoc(args: {
    id: string;
    name: string;
    updatedAt: number;
  }): SearchDoc {
    const titleTokens = new Set(tokenize(args.name));
    return {
      id: args.id,
      kind: "list",
      title: args.name,
      body: "",
      updatedAt: args.updatedAt,
      titleTokens,
      bodyTokens: new Set(),
      contextTokens: new Set(),
      tokens: new Set(titleTokens),
    };
  }

  function reindexItem(args: {
    id: string;
    text: string;
    notes: string;
    listId: string;
    lifecycle: SearchLifecycle;
    updatedAt: number;
  }): void {
    unindexDoc(args.id);
    indexDoc(makeItemDoc(args));
  }

  function rebuildItemsInList(listId: string): void {
    // Snapshot the id set: reindex mutates itemsByList while iterating.
    const items = itemsByList.get(listId);
    if (!items) return;
    for (const itemId of Array.from(items)) {
      const itemDoc = docsById.get(itemId);
      if (!itemDoc || itemDoc.kind !== "item") continue;
      reindexItem({
        id: itemDoc.id,
        text: itemDoc.title,
        notes: itemDoc.body,
        listId: itemDoc.listId ?? "",
        lifecycle: itemDoc.lifecycle ?? "backlog",
        updatedAt: itemDoc.updatedAt,
      });
    }
  }

  function rebuild(state: WorkspaceState): void {
    docsById.clear();
    postings.clear();
    itemsByList.clear();
    listNames.clear();

    // Lists first so item docs can read list names for context tokens.
    for (const id of state.listsOrder) {
      const list: ListView | undefined = state.listsById[id];
      if (!list) continue;
      listNames.set(list.id, list.name);
      indexDoc(
        makeListDoc({ id: list.id, name: list.name, updatedAt: list.createdAt }),
      );
    }

    // Iteration order is irrelevant to the index — enumerate the id map
    // directly (there is no global order array to walk; see store.ts).
    for (const item of Object.values(state.itemsById)) {
      // Recency signal: latest state-bearing timestamp — the bin mask,
      // else the register's transition time (which falls back to
      // createdAt for never-touched items).
      const updatedAt = item.binnedAt ?? item.lifecycleAt ?? item.createdAt;
      indexDoc(
        makeItemDoc({
          id: item.id,
          text: item.text,
          notes: item.notes,
          listId: item.listId,
          lifecycle: lifecycleOf(item),
          updatedAt,
        }),
      );
    }
  }

  function apply(event: AppEventJs): void {
    switch (event.kind) {
      case "itemAdded": {
        const lifecycle = lifecycleFromEvent(event);
        const updatedAt = bigToNum(event.createdAt) ?? Date.now();
        reindexItem({
          id: event.id,
          text: event.text ?? "",
          notes: event.notes ?? "",
          listId: event.listId ?? "",
          lifecycle,
          updatedAt,
        });
        break;
      }
      case "itemRemoved": {
        unindexDoc(event.id);
        break;
      }
      case "itemTextChanged": {
        const prev = docsById.get(event.id);
        if (!prev || prev.kind !== "item") break;
        reindexItem({
          id: prev.id,
          text: event.text ?? "",
          notes: prev.body,
          listId: prev.listId ?? "",
          lifecycle: prev.lifecycle ?? "backlog",
          updatedAt: Date.now(),
        });
        break;
      }
      case "itemNotesChanged": {
        const prev = docsById.get(event.id);
        if (!prev || prev.kind !== "item") break;
        reindexItem({
          id: prev.id,
          text: prev.title,
          notes: event.notes ?? "",
          listId: prev.listId ?? "",
          lifecycle: prev.lifecycle ?? "backlog",
          updatedAt: Date.now(),
        });
        break;
      }
      case "itemLifecycleChanged": {
        const prev = docsById.get(event.id);
        if (!prev || prev.kind !== "item") break;
        prev.lifecycle = lifecycleFromEvent(event);
        prev.updatedAt = Date.now();
        break;
      }
      case "itemListChanged": {
        const prev = docsById.get(event.id);
        if (!prev || prev.kind !== "item") break;
        reindexItem({
          id: prev.id,
          text: prev.title,
          notes: prev.body,
          listId: event.listId ?? "",
          lifecycle: prev.lifecycle ?? "backlog",
          updatedAt: Date.now(),
        });
        break;
      }
      case "listAdded": {
        listNames.set(event.id, event.name ?? "");
        unindexDoc(event.id);
        const updatedAt = bigToNum(event.createdAt) ?? Date.now();
        indexDoc(
          makeListDoc({
            id: event.id,
            name: event.name ?? "",
            updatedAt,
          }),
        );
        break;
      }
      case "listRemoved": {
        const items = Array.from(itemsByList.get(event.id) ?? []);
        unindexDoc(event.id);
        listNames.delete(event.id);
        // Any items still in this listId lose their context tokens; they
        // keep their listId pointer so a UI that resolves lists by id
        // still has it, but no list name means no context tokens.
        for (const itemId of items) {
          const itemDoc = docsById.get(itemId);
          if (!itemDoc || itemDoc.kind !== "item") continue;
          reindexItem({
            id: itemDoc.id,
            text: itemDoc.title,
            notes: itemDoc.body,
            listId: itemDoc.listId ?? "",
            lifecycle: itemDoc.lifecycle ?? "backlog",
            updatedAt: itemDoc.updatedAt,
          });
        }
        break;
      }
      case "listRenamed": {
        listNames.set(event.id, event.name ?? "");
        unindexDoc(event.id);
        indexDoc(
          makeListDoc({
            id: event.id,
            name: event.name ?? "",
            updatedAt: Date.now(),
          }),
        );
        rebuildItemsInList(event.id);
        break;
      }
      // listMoved / itemMoved are pure ordering — irrelevant to the index.
    }
  }

  function anyStartsWith(set: Set<string>, prefix: string): boolean {
    for (const t of set) if (t.startsWith(prefix)) return true;
    return false;
  }

  function prefixCandidates(prefix: string): Set<string> {
    const out = new Set<string>();
    // Linear scan over unique tokens — corpus is small enough today; a
    // trie can drop in later if measurement says so.
    for (const [token, ids] of postings) {
      if (token === prefix || token.startsWith(prefix)) {
        for (const id of ids) out.add(id);
      }
    }
    return out;
  }

  function intersect(
    a: Set<string> | null,
    b: Set<string>,
  ): Set<string> {
    if (a === null) return new Set(b);
    const out = new Set<string>();
    const [smaller, larger] = a.size <= b.size ? [a, b] : [b, a];
    for (const id of smaller) if (larger.has(id)) out.add(id);
    return out;
  }

  interface Scored {
    doc: SearchDoc;
    titleExact: number;
    titlePrefix: number;
    bodyHits: number;
    contextHits: number;
    score: number;
  }

  function scoreDoc(
    doc: SearchDoc,
    exactTokens: readonly string[],
    finalToken: string,
  ): Scored | null {
    let titleExact = 0;
    let titlePrefix = 0;
    let bodyHits = 0;
    let contextHits = 0;

    for (const t of exactTokens) {
      if (doc.titleTokens.has(t)) titleExact++;
      else if (doc.bodyTokens.has(t)) bodyHits++;
      else if (doc.contextTokens.has(t)) contextHits++;
      else return null;
    }

    // Final token: exact wins over prefix in title; otherwise fall back
    // through body then context. The doc must match it somewhere or it
    // wouldn't have ended up in the candidate set, but recheck so the
    // bucket assignment is correct.
    if (doc.titleTokens.has(finalToken)) titleExact++;
    else if (anyStartsWith(doc.titleTokens, finalToken)) titlePrefix++;
    else if (
      doc.bodyTokens.has(finalToken) ||
      anyStartsWith(doc.bodyTokens, finalToken)
    )
      bodyHits++;
    else if (
      doc.contextTokens.has(finalToken) ||
      anyStartsWith(doc.contextTokens, finalToken)
    )
      contextHits++;
    else return null;

    const lifecycleRank = lifecycleRankOf(doc.lifecycle);
    // Flat numeric score for the public SearchResult.score field. The
    // sort comparator below uses the bucket counts directly so this
    // collapse never affects ordering — it's purely a debugging /
    // display convenience.
    const score =
      titleExact * 10000 +
      titlePrefix * 1000 +
      bodyHits * 100 +
      contextHits * 10 +
      lifecycleRank;

    return { doc, titleExact, titlePrefix, bodyHits, contextHits, score };
  }

  function query(input: string, limit = 50): SearchResult[] {
    const tokens = tokenize(input);
    if (tokens.length === 0) return [];
    const finalToken = tokens[tokens.length - 1];
    const exactTokens = tokens.slice(0, -1);

    let candidates: Set<string> | null = null;
    for (const t of exactTokens) {
      const s = postings.get(t);
      if (!s) return [];
      candidates = intersect(candidates, s);
      if (candidates.size === 0) return [];
    }
    const finalSet = prefixCandidates(finalToken);
    if (finalSet.size === 0) return [];
    candidates = intersect(candidates, finalSet);
    if (!candidates || candidates.size === 0) return [];

    const scored: Scored[] = [];
    for (const id of candidates) {
      const doc = docsById.get(id);
      if (!doc) continue;
      const s = scoreDoc(doc, exactTokens, finalToken);
      if (s) scored.push(s);
    }
    scored.sort((a, b) => {
      if (a.titleExact !== b.titleExact) return b.titleExact - a.titleExact;
      if (a.titlePrefix !== b.titlePrefix) return b.titlePrefix - a.titlePrefix;
      if (a.bodyHits !== b.bodyHits) return b.bodyHits - a.bodyHits;
      if (a.contextHits !== b.contextHits)
        return b.contextHits - a.contextHits;
      const sa = lifecycleRankOf(a.doc.lifecycle);
      const sb = lifecycleRankOf(b.doc.lifecycle);
      if (sa !== sb) return sb - sa;
      if (a.doc.updatedAt !== b.doc.updatedAt)
        return b.doc.updatedAt - a.doc.updatedAt;
      return a.doc.id < b.doc.id ? -1 : a.doc.id > b.doc.id ? 1 : 0;
    });
    return scored.slice(0, limit).map((s) => ({
      id: s.doc.id,
      kind: s.doc.kind,
      title: s.doc.title,
      body: s.doc.body || undefined,
      listId: s.doc.listId,
      lifecycle: s.doc.lifecycle,
      score: s.score,
    }));
  }

  return { rebuild, apply, query };
}
