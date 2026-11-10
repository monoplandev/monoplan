# Monoplan web-UI list-perf survey — handoff

> Working design/handoff doc (not a finalized contract). Companion to the
> "Performance note" in [`data-model.md`](data-model.md). Move/rename freely.

## Status (2026-07-02)

**Phase 1 landed.** Core emits `open_index` (position in the owning
list's open projection) on `ItemAdded` / `ItemMoved` /
`ItemLifecycleChanged` / `ItemListChanged`, alongside the global `index`;
wasm surfaces it as `openIndex`. The web store dropped `itemsOrder`
entirely: `WorkspaceState.listOpen` per-list arrays + maintained
`binCount`, list-local dispatch, lazy Done/Bin timestamp sorts. The
done-linger affordance survives via a store-side `recentDone` capture
(id, listId, vacated open index, doneAt) that `Workspace.items()`
overlays for the linger window.

**Also fixed (found post-Phase-1 on a 13k-item store):** the batch
lifecycle mutations in core (`set_items_done` / `set_items_binned` /
`delete_binned_items`) did a full-doc materialize → `rebuild_item_index`
→ diff on *every* call — and the web Delete-key path always routes
through the `*Many` variants, so binning ONE item cost three O(N)
whole-doc passes in wasm. They are now surgical below 64 ids
(`BULK_LIFECYCLE_EVENT_THRESHOLD`, matching the store's coarse threshold)
and fall back to rebuild+diff above it.

**Snapshot-per-ack fixed:** the dominant remaining lag was persistence
policy, not mutation paths — `snapshotIfFullySynced` ran on the
per-ack pulse, i.e. a whole-doc Loro export + AES seal + IDB write
(~40ms native / ~100ms wasm at 13k) one RTT after *every* mutation.
Now thresholded: hot pulse compacts only at ≥250 op rows
(`COMPACT_MIN_OPS`), with an idle timer (20s quiet) and the
visibility-hidden hook folding the remainder. CLI still folds every
run (`snapshot_if_fully_synced(1)`).

**Remote frames fixed (`events_translator`, promised by the old
`events.rs` header, now real):** `apply_remote` no longer does
pre-collect → `rebuild_item_index` → whole-doc diff (~35ms native per
frame — felt directly with two open tabs). A root Loro subscription
captures Import-triggered container diffs; `Doc::translate_remote_diffs`
turns them into surgical per-id `AppEvent`s and incremental index
updates, using a new global-order shadow (`ItemIndex.order`) to
resolve positional deletes. Frames touching ≥64 items, or any diff
shape we can't translate, fall back to one whole-doc resync pass
and one `FullResync` control event; consumers materialize current state
once rather than receiving N synthetic item events. Measured: **1 remote op at 13k items = ~0.7ms native** (was
~35ms), emitting exactly one event.

**Phase 0 item 1 done (dnd order-version guard):**
`selection.updateOrder` no-ops on reference equality
(`selection.ts`) — sound because `DndSource` only ever *replaces* its
order array. Kills the per-scroll key→index Map rebuild and dedupes
the 3–4 rebuilds per mutation down to 1.

**Large-selection drag fixed:** dragging a whole-view selection (e.g.
10k Done rows) was O(selection) *per pointermove*, twice: (1)
`dispatchDrag` materialized the full `keys` + `items` arrays (10k
`getItem` calls) into every `primavera-dnd-dragmove` event, whose only
per-move consumer reads one element to classify the drag — the detail's
`keys`/`items` are now lazy cached getters plus a `firstItem` field,
and `Workspace.isItemDrag` classifies via `firstItem`; (2)
`getRenderState` rebuilt a `Set` of all selected keys per render frame
— now memoized as `DndSelection.getSelectedKeySet()`, invalidated by
`notify()` (every block mutation ends there) and by order reindexes;
(3) — the big one — `keepAlive` mounted a **hidden DOM row for every
dragged key** (10k hidden Solid components for a whole-view drag) and
rebuilt that array on every drag frame. Now snapshotted once at drag
start and bounded to rows that were actually *mounted* then (the
virtualized window — unmounted rows have no component state to
preserve, which was keepAlive's whole purpose).

**Undo/redo follow-up landed:** Loro container diffs are now captured during
undo/redo and use the same incremental translator as remote frames. A normal
move undo emits one `ItemMoved`; unsupported/bulk shapes retain the whole-doc
fallback. Reorder planning now emits at most one core move per selected row,
instead of one move per displaced row, while the web action stack keeps the
whole selection drag as one visible undo.

**Boot/refresh follow-up landed:** local snapshot + tail hydration uses deferred
oplog replay and builds `ItemIndex` once at completion, with no historical open-projection
events. Server snapshot bootstrap persists the received encrypted blob with a
`ServerFrontier(up_to_seq)` cutoff (pruning the confirmed rows it contains, so
refresh no longer replays the full history), then emits one `FullResync`. Initial
attach and resync use one compact `workspaceSnapshotJson` crossing rather than
thousands of wasm `AppEventJs` wrappers. Snapshot compaction is gated until the
replica reaches steady-state `Idle`.

**Known remaining O(N) per-op paths:**
- Loro's own movable-list `UndoManager::undo` work still scales with document
  size; at 13k lifetime items a reloaded-doc move undo measured ~30ms native
  while redo measured <1ms. Twenty moves undo in ~267ms native. The former
  Monoplan full rebuild/diff and distance amplification are gone.
- `delete_list` / `empty_bin` — O(N) once, rare and explicitly bulk.

Measured terms (native M1 release, 13k lifetime items; wasm ≈2–4×):
`add_item` 0.11ms · `set_item_done` 0.08ms · `set_items_binned(1)`
0.14ms · `apply_remote(1 op)` 0.7ms · `snapshot_blob` 40ms ·
`iter_items().collect()` 14ms · `rebuild_item_index` 7.6ms. Re-run via
`cargo test -p monoplan-core --release bench_mutation_terms_at_13k --
--ignored --nocapture`.

### v2 per-list order schema (2026-07-04)

The document-wide `items` MovableList is gone (`spec/data-model.md`
schema v2): item identity lives in a root `items` map, ordering in
per-list `order/<list-id>` containers of scalar entries, membership in
an atomic `location` register. Structural consequences for perf:

- Reordering one list never touches another list's container — small
  lists no longer pay for `main`'s 13k-entry history on any path.
- Cross-list moves are register writes + scalar entry ops, not `mov`s
  on a giant container; the multi-peer move/undo wasm traps
  (`loro-bug/`) are no longer reachable through this path.
- Hot mutations are O(open) in-memory splices, gated by per-list
  visible-entry counts; restores and remote frames recompute one list's
  projection (O(that list's lifetime entries), pure memory).

Same bench, same shape (13k lifetime, 200 open in main): `add_item`
0.17ms · `set_item_done` 0.011ms · `set_items_binned(1)` 0.009ms ·
`apply_remote(1 op)` 5.2ms · `snapshot_blob` 59ms · `iter_items
().collect()` 19ms · `rebuild_index` 15ms · reloaded-doc move undo
21ms (was ~30ms) / redo 2.4ms / 20-move undo 67ms (was ~267ms).
`apply_remote`, `snapshot_blob` and the whole-doc walks regressed
2–7×; all are per-frame or rare paths (not per-keystroke) and the
13k-in-one-list shape is their worst case. Follow-up candidates:
single-walk translation refresh, interning ids in the projection
index.

## Problem
On M1, lists feel sluggish to respond at ~5k+ items. Confirmed cause: **not DOM
re-render, not the CRDT op** — it's JS reactivity recomputing derived state O(N
over *all* items in the doc) on every mutation. Matches the note in
`data-model.md` ("global changes per move"; core move ~0.13ms but ~10 FPS → tens
of ms in JS).

## Confirmed seams (file:line)
Per single mutation (e.g. one move / lifecycle toggle), with N = total items in
doc, M = current-view length:

1. **Four global O(N) memos in `js/web/src/Workspace.tsx`**, all scanning the
   global `state.itemsOrder` (all lists + Done + Bin):
   - `items()` `Workspace.tsx:170-196` — maps N→new array; filter reads
     `listId/doneAt/binnedAt`, so any lifecycle change re-runs it.
   - `lingerChain()` `:130-151`
   - `openCountsByList()` `:221-229`
   - `binCount()` `:207-214`
   - → Done/Bin items are dead weight in every list-view mutation (explains
     Done-heavy slowness).
2. **DnD cascade O(M)**, several times/mutation: `setDndItems` effect
   `Workspace.tsx:253` → `keyIndex` rebuild `dnd/solid/Dnd.tsx:96` →
   `source.syncOrder` `Dnd.tsx:234` → `selection.updateOrder`
   (`dnd/core/selection.ts:43`, full Map rebuild) → `getRenderState` calls
   `selection.updateOrder` **again** `dnd/core/dnd-controller.ts:294`.
3. **Scroll cost**: `onScroll`→`onChange`→`getRenderState`→`selection.updateOrder`
   rebuilds the full key→index Map **on every scroll event**, independent of
   mutations.

DOM is virtualized (`dnd-controller.ts:384-416`), so node count is bounded —
ruled out as the cause.

## Root cause
Web store keeps one global `itemsOrder` (`js/web/src/sync/store.ts:42`) and
re-derives every view + count from it each mutation. The core event contract
(`core/src/events.rs:18-20`) emits a **global** index and instructs UIs to
"track a single global order and filter per view."

## Key enabler (already exists in core)
`da4a631c` added `ItemIndex.open_by_list` in `core/src/doc.rs:218` — **per-list
open-order arrays** (Done/binned excluded). The projection exists in Rust but is
flattened back to global at the event boundary. Finishing the job = surfacing
per-list local index through events.

## Design direction
Goal: **no global-order evaluation on any mutation path**; only per-list arrays,
built once at boot.
- Global order is not fundamentally needed post-startup: list views =
  `open_by_list[L]` (CRDT order); Done/Bin = timestamp sort (`doneAt`/`binnedAt`
  desc), not CRDT order.
- Mandatory boot O(N) is `itemsById` + **search index** (`store.ts:407`) — a
  *content* pass, not an order pass. Build per-list arrays as a byproduct; drop
  the global `itemsOrder` array entirely.
- This removes the Done-weight problem **without** the risky second-doc split
  floated in the `data-model.md` note.

### Target store shape
```ts
interface WorkspaceState {
  itemsById: Record<string, ItemView>;
  listOpen: Record<string, string[]>;       // per-list open order (the projection)
  openCountsByList: Record<string, number>;  // maintained, not scanned
  binCount: number;                          // maintained, not scanned
  listsOrder: string[];
  listsById: Record<string, ListView>;
  settings: SettingsView;
  // no global itemsOrder
}
```
Dispatch becomes list-local (splice into one `listOpen[listId]`, adjust
counters). Done/Bin computed lazily only while that view is active, memoized.

### Core change needed
Emit **per-list local open index** on `ItemAdded`/`ItemMoved`/`ItemListChanged`
(position within that list's open array), instead of/alongside global index.
Optionally expose `open_by_list` snapshot for boot.

## Recommended sequencing
**Phase 0 — cheap, no core change, measure after:**
1. Order-version guard on `selection.updateOrder` (`selection.ts:43`) → no-op
   when order unchanged; kills per-scroll rebuild + double-rebuild-per-mutation.
2. Incremental counters for `binCount`/`openCountsByList` → drop the two global
   scans.

**Phase 1 — real fix:** per-list `listOpen` arrays + per-list index in events;
drop `itemsOrder`; lazy Done/Bin.

## Residual ceiling (defer)
Intra-list add/move is still O(M_list) store splice (cheap pointer memmove; the
list you're viewing). A pathological single 10k open list keeps one O(M) splice.
Truly sub-linear would need fractional indexing / order-statistic tree —
over-engineering for now.
