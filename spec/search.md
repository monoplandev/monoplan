# Search

Current search is a **local, client-side, plaintext index** over the already-decrypted account doc. Its first consumer is the web command palette (`cmd/ctrl+f`). The server does not participate.

**Status (2026-10-08): the engine lives in core** (`core/src/search.rs`, `Doc::search`). Every client gets the same tokenizer and ranking; the web client is a thin query bridge over wasm (`searchJson`, `tokenize`, `matchesName`). The JS implementation this spec was first written against has been removed.

## Goals

- Instant search over items and lists in the active account. **Archived lists
  are included**: a list stays indexed (and keeps supplying its name as item
  context) whether it is active or archived (`spec/data-model.md` "Archived
  lists").
- Works offline.
- Updates incrementally from the same domain event stream that drives UI state.
- Small and simple enough to ship without introducing a storage engine or external dependency.

## Non-goals

- Server-side search.
- Cross-account search.
- Full-text ranking sophistication.
- Typo-tolerant / fuzzy search.
- Substring search over arbitrary middles of words.
- Shared persisted index format across web / CLI.

## Placement

The search index lives on the **client**, inside the core `Doc`, beside the projection index.

- The server cannot read op contents or run a doc, so it cannot own search. See `spec/architecture.md`.
- The index must not live inside the palette component itself; the palette is a query/view surface, not the source of truth.
- The index is maintained by core, not mirrored by each client. Core already sees every mutation (local commits and translated remote imports) through one event funnel, so it is the one place that cannot drift from the doc.

Clients call `Doc::search(query, limit)` (wasm: `searchJson`). Nothing else is needed: no rebuild call on attach, no per-event apply. A client that wants to fold names the same way (pickers, non-doc indexes) calls `tokenize` / `matches_name` from core rather than keeping its own copy.

## Source data

Index these doc entities:

- `Item`
- `ListMeta`

For `Item`, index:

- `text` with highest weight
- `notes` with lower weight
- current list name with lower weight, so `work` can match an item living in the Work list

For `ListMeta`, index:

- `name`

Do **not** index:

- encrypted blobs
- op payloads
- timestamps as searchable text
- ids as searchable text

## Indexed document shape

Implementations may choose concrete language-native containers, but the logical shape is:

```ts
type SearchIndex = {
  docsById: Map<string, SearchDoc>;
  postings: Map<string, Set<string>>;
};

type SearchDoc = {
  id: string;
  kind: "item" | "list";
  title: string;
  body: string;
  listId?: string;
  lifecycle?: "backlog" | "todo" | "in_progress" | "review" | "done" | "cancelled" | "binned";
  updatedAt?: number;
  tokens: string[];
};
```

Notes:

- `docsById` is the canonical indexed representation per entity.
- `postings` is the inverted index: token -> matching doc ids.
- `tokens` must be stored on each `SearchDoc` so updates can remove stale postings before re-inserting the new token set.
- `updatedAt` is for ranking only; it is not part of tokenization.
- `lifecycle` is for ranking / filtering only; it is not part of tokenization.

## Normalization

Normalization rules:

- Unicode normalize (`NFKD` if available in the host) and strip combining
  marks, folding accents so an accent-free query matches accented text and
  vice versa (`articulo` ↔ `artículo`). Applied to both indexed text and
  queries via the shared tokenizer.
- lowercase
- trim surrounding whitespace
- split on whitespace and punctuation
- drop empty tokens
- de-duplicate tokens within a single doc

Do not stem, lemmatize, or remove stop words.

Examples:

- `"Buy groceries"` -> `["buy", "groceries"]`
- `"PR #142"` -> `["pr", "142"]`
- `"Q3 roadmap"` -> `["q3", "roadmap"]`

## Query semantics

Given user input:

1. Normalize and tokenize the query.
2. Every complete query token except possibly the final token is an **exact token match**.
3. The final token is:
   - exact-match if the query ends at a token boundary, or
   - prefix-match if the user is still typing and the token is partial.

In practice, the command palette should treat the last token as prefix-match.

Example:

- Query `"buy gro"` matches `"Buy groceries"`
- Query `"pho"` matches `"Read Phoenix spec"`
- Query `"off"` does not need to match `"team offsite"` by arbitrary substring; it matches because `offsite` has the prefix `off`

Document inclusion rule:

- A result must satisfy every query token.
- Multi-token queries use AND semantics, not OR semantics.

## Ranking

Ranking is heuristic and local. Keep it deterministic and simple.

Suggested precedence:

1. Exact title/name token hits
2. Prefix title/name hits
3. Notes hits
4. List-name context hits on items
5. Lifecycle order when text match is otherwise equal: `in_progress`, then
   `review`, then `todo`, then `backlog`, then `done` / `cancelled` (tied),
   then `binned` (active
   work first, then queued, then closed)
6. More recently updated items before older items
7. Stable tie-breaker by id

This is intentionally not BM25/Tf-Idf territory. The corpus is small and the UX target is command-palette relevance, not document retrieval science.

## Update model

The index is maintained incrementally from domain events via a **dirty set**, reconciled against the doc on the next query.

- Every `AppEvent` core enqueues is inspected as it is pushed. Item events that affect indexed fields mark the item id dirty; `ListAdded` / `ListRemoved` / `ListRenamed` mark the list id dirty; `FullResync` and every bulk path that rebuilds the projection index (boot replay, import, translation fallback) mark the whole index dirty.
- `search()` first drains the dirty set: a whole-dirty index rebuilds from `all_lists` + every item; otherwise each dirty list is re-read (indexed or dropped) and every item currently indexed under it is re-read for context tokens, then each dirty item is re-read (indexed or dropped).
- Reconciling reads the doc, never the event payload, so the index only depends on an event naming the right id. Events that are pure ordering or touch unindexed registers (`when`, `deadline`, `place`, icons, settings) are ignored.
- Recency (`updatedAt`) is the item's `binned_at`, else its workflow transition time (`lifecycle_at`, which falls back to `created_at`); for lists it is `created_at`. It is read from the doc, so it is deterministic across clients and does not bump on a text edit.

The required behaviors, stated per event kind, are unchanged:

- `ItemAdded`: build a new indexed doc and insert postings
- `ItemRemoved`: remove doc and its postings
- `ItemTextChanged`: rebuild that item's token set
- `ItemNotesChanged`: rebuild that item's token set
- `ItemLifecycleChanged`: update rank/filter metadata only
- `ItemListChanged`: rebuild that item's token set because list-name context may have changed
- `ListAdded`: add the list doc
- `ListRemoved`: remove the list doc; rebuild any item docs that referenced that list name as context
- `ListRenamed`: rebuild the list doc; rebuild any item docs whose `listId` references that list
- `ItemMoved` / `ListMoved`: no token change; ordering changes are irrelevant to the index
- `FullResync`: rebuild once from the newly materialized workspace state

Implementations may do targeted updates or opportunistic small rebuilds, but they must preserve correctness.

## Build timing

A fresh `Doc` starts whole-dirty, so the first `search()` call builds the index from the doc. This is deliberate: core is shared with the CLI, whose commands never search, and an eager build at boot would be pure cost on the load path `spec/tui-plan.md` measures. The build is one pass over items and lists in Rust and is not measurable at current data limits (`spec/data-model.md`). If a client ever observes a first-open hitch, it may issue a throwaway `search("", 0)`-style warm call after attach; none does today.

Do not rebuild the entire index after every mutation. The dirty set gives the minimal invalidation surface, and at most one reconcile happens per query, not per mutation.

## Persistence

Default: **do not persist a separate search index**.

Reasons:

- the local doc is already persisted
- corpus size is small
- index rebuild on startup is acceptable at the current scale
- avoiding a second local artifact keeps migration / invalidation logic out of scope

If startup cost later becomes measurable, a client may add a persisted sidecar index. If so:

- it is strictly a cache derived from the doc
- it must be safe to delete at any time
- stale or failed loads must fall back to full local rebuild

## Security / privacy

The index contains plaintext derived from decrypted local state.

- It must never be sent to the server.
- It must never be included in protocol messages.
- Browser implementations should treat it as in-memory only unless a later spec explicitly allows persisted search caches.

This is not a new trust boundary; any client capable of rendering the decrypted doc already holds the same plaintext in memory.

## Performance envelope

Current data limits from `spec/data-model.md` are small enough for an in-memory inverted index.

Acceptable characteristics:

- initial build proportional to number of local items + lists
- per-event update proportional to tokens touched by that event
- query latency roughly proportional to query tokens plus candidate set size

Do not introduce heavyweight search libraries unless measurement proves the simple index insufficient.

Trees are optional future optimizations:

- trie / radix tree for faster prefix token lookup
- fuzzy index structures if typo tolerance becomes a product requirement

Start with `Map`/`Set`-style inverted index structures first.

## Client contract

Core exposes:

```rust
pub fn tokenize(input: &str) -> Vec<String>;
pub fn matches_name(name: &str, query: &str) -> bool;
impl Doc { pub fn search(&self, input: &str, limit: usize) -> Vec<SearchResult>; }
```

and wasm mirrors them as `tokenize`, `matchesName` and `Doc.searchJson` / `SyncEngine.searchJson` (a JSON array of the result shape below, `lifecycle` as its wire name, `body` / `listId` / `lifecycle` omitted when unset).

The palette/query surface should consume a narrow interface, e.g.:

```ts
type SearchResult = {
  id: string;
  kind: "item" | "list";
  title: string;
  body?: string;
  listId?: string;
  lifecycle?: "backlog" | "todo" | "in_progress" | "review" | "done" | "cancelled" | "binned";
  score: number;
};

interface SearchEngine {
  query(input: string, limit?: number): SearchResult[];
}
```

The exact API may differ by client, but the separation is load-bearing:

- the update path is core's alone; clients never feed the index
- palette UI depends on `query(...)`, not on the index internals

## Palette-level entries (built-in views)

The built-in views — Focus (`spec/focus.md`), Inbox (the reserved `main`
capture list), and Done — are not `ListMeta` rows, carry localized labels,
and are therefore **not indexed by the engine**. The palette synthesizes
them above the engine's results:

- **Empty query (default menu):** the palette shows the built-in views
  (Focus, Inbox, Done — nav order) followed by every *active* user list in
  CRDT order. The palette doubles as a jump-to-view switcher before the
  user types anything. Archived lists are omitted from the default menu
  but remain reachable by query (they stay indexed).
- **Archived marker:** a result that is an archived list, or an item
  whose owning list is archived, renders the localized "Archived" badge (plain `.badge` style)
  between its title and the owning-list column, so a hit in an archived
  list is never mistaken for live work. The engine stays unaware of
  archive state; the surface resolves it from the list projection by id.
- **Non-empty query:** each built-in view whose localized label matches
  the query (same tokenizer / prefix semantics as list-name matching) is
  prepended above the engine's results. Selecting one navigates to that
  view.

Because the labels are localization-owned UI strings, this matching lives
in the palette, not the engine — the engine stays a pure index over doc
entities.

## Testing

Minimum test coverage:

1. tokenization / normalization examples
2. add -> query returns result
3. text edit removes stale tokens and adds new ones
4. notes edit affects matches at lower rank than title matches
5. list rename updates both the list result and item context matches
6. delete removes result from queries
7. multi-token AND queries
8. last-token prefix queries
9. ranking preference: in_progress over review over todo over backlog over done / cancelled over binned when textual match is otherwise equal

Where feasible, drive the index through the ordinary `Doc` mutation API rather than bespoke test-only paths. Core's coverage is `core/tests/search.rs`; the web keeps one boundary test (`js/web/test/search.test.ts`) that the wasm exports round-trip.

## Open questions

- Whether item `notes` should appear in the palette result preview, or only participate in matching.
- Whether CLI should expose `monoplan find <query>` or defer search to the web UI first. The engine is now in core, so the verb is a thin wrapper when wanted.

Resolved: built-in view labels are matched at the palette layer by their
rendered (localized) names rather than indexed by the engine — see
"Palette-level entries (built-in views)".
