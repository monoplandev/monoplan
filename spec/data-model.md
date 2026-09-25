# Data Model

## Loro doc layout

One Loro doc per account. **Schema version 4** — see "Schema versioning &
compatibility" below; v4 docs must never sync with v1/v2/v3 clients.

- `doc.get_map("items")` — `LoroMap<ItemId, LoroMap>`: item identity and
  content. Keyed by the item's stable UUID; each value is a child `LoroMap`
  (one Item, below). Items live here for their whole lifetime regardless of
  which list they're in or whether they're done/binned.
- `doc.get_movable_list("lists")` — `LoroMovableList<LoroMap>` where each map
  is one ListMeta. Unchanged from v1.
- `doc.get_map("settings")` — `LoroMap` for account-wide synced workspace
  settings. Unchanged from v1.
- `doc.get_movable_list("order/<list-id>")` — one **order container** per
  logical list (`order/inbox` for the built-in list). Entries are **encoded
  scalar strings only** (OrderEntry, below) — never child containers. The
  order container carries ordering; the `items` map carries everything else.
- `doc.get_movable_list("focus")` — the reserved **focus container**: a curated
  list-by-reference of **encoded scalar FocusRef strings only** (never child
  containers). Its own element order *is* the Focus order. Additive within v2 —
  a focus-unaware client simply never projects it. See `spec/focus.md`.

There is **no document-wide item MovableList**. Reordering one list mutates
only that list's order container.

Historical `columns/<list-id>` containers may exist in old accounts (custom
board columns, removed). They are unreachable history: v2 code never opens
them and never projects them.

## Item

One child `LoroMap` under `items`, keyed by `ItemId`.

| Field | Type | Notes |
|---|---|---|
| `id` | string | same as the map key; kept inside the map so a container handle resolves back to its id during diff translation |
| `text` | string | the user's content. Stays a string register on purpose: a title is rewritten whole, so concurrent rewrites resolve by LWW to one clean title rather than a character interleaving (`notes-plan.md` "Why `text` stays a register") |
| `notes` | `LoroText` | optional free-form plain text as a **mergeable child text container** (`LoroMap::ensure_mergeable_text`, `notes-plan.md`). Absent until the first write; created lazily by whichever device writes first, and concurrent first writes on two devices merge into one container. Edits are applied as character diffs (`LoroText::update`), so concurrent edits from two devices merge character-wise. Clearing deletes the content and **keeps the key**: the key is never deleted, because a later `ensure` would resurface the hidden child's old content. Reads take the container's plain string; a stray string value at the key (a v3 leftover) is not read. |
| `location` | string | **atomic placement register** — encoded `"<list_id>:<placement_id>"`, see below |
| `lifecycle` | value | **atomic workflow register** — a plain `LoroValue` list `[state, at]`: the current workflow state (integer `0..=5`, see "Lifecycle") and the unix millis it was entered. Absent ≡ `[Backlog, created_at]`; new items omit it. |
| `binned_at` | i64? | **bin mask** — unix millis when the item was binned. Present ≡ binned (masking the workflow state); absent ≡ not binned. Restore deletes the key, revealing the preserved workflow state. Orthogonal to `lifecycle`. |
| `deadline` | string? | optional **date-only** deadline, a floating local calendar date in `YYYY-MM-DD` format (no time, no timezone, not unix millis). Absent ≡ no deadline; clearing deletes the key. Values that are not a well-formed `YYYY-MM-DD` calendar date are rejected by the mutation. Means "owed by": past it the item is overdue. |
| `when` | string? | optional **planned date**, shape-discriminated: `YYYY-MM-DD` (all-day, 10 chars) or `YYYY-MM-DDTHH:MM` (timed, 16 chars, `HH` 00–23, `MM` 00–59). Floating wall-clock, no seconds, no zone; an RFC 9557 `[Zone]` suffix is reserved and rejected for now. Absent ≡ unset; clearing deletes the key. One register so date and time cannot tear under concurrent edit; sorts by plain string compare (all-day leads its day). Means "happens on" or "act on". Fixed: never rewritten or rolled over by the clock, and never red past its day; whether a past `when` "slipped" is undecided until events and tasks are distinguished (`calendar-plan.md`). Independent of `deadline`. See `calendar-plan.md`. |
| `duration` | i64? | optional **length in whole minutes**, `1..=10080` (one week). A length rather than an end so that moving `when` on one device and setting the length on another can never produce an item that ends before it starts; the end is derived (`when` + `duration`). Meaningful only beside a timed `when`: views ignore it beside an all-day or absent one, and the register is never cross-checked against `when` (no invalid state under concurrent edit). Absent ≡ unset; clearing deletes the key. Written by `set_item_when` too: a timed `when` on an item with no duration defaults it to 60 minutes in the same commit, and clearing `when` deletes it; timed → all-day keeps it. Out-of-range values read as unset. See `calendar-plan.md`. |
| `created_at` | i64 | unix millis (client clock) |
| `started_at` | i64? | **reflection stamp**: set (write-once) the first time the item enters In Progress; never cleared. Feeds analytics (created → started); no view reads it. |
| `done_at` | i64? | **reflection stamp**: set each time the item enters Done; never cleared, so it survives later binning and un-doing. Feeds analytics (started → done); view sorts use the register's `at`, not this. |

Item type is implicit (currently always text). Add an `item_type` field when
other kinds appear.

### Lifecycle

Lifecycle is a **workflow register plus a bin mask** — two persisted fields:

```
enum WorkflowState {          // the `lifecycle` register, 0..=5
    Backlog    = 0,
    Todo       = 1,
    InProgress = 2,
    Review     = 3,
    Done       = 4,           // closed: completed
    Cancelled  = 5,           // closed: deliberately dropped, not completed
}

enum ItemLifecycle {          // API-level resolved lifecycle
    Backlog, Todo, InProgress, Review, Done, Cancelled,
    Binned,                   // = binned_at present, masking the state
}
```

The first four states are **Open**; Done and Cancelled are the two
**closed** (terminal) states. Cancelled exists because "I decided not to do
this" is a real outcome worth keeping next to what was finished — unlike
the bin, which is for mis-captures and duplicates that may be hard-deleted.
It is not a completion: it never writes `done_at`, and it renders as a cross
where Done renders a tick. Cancelled is **not** a board lane; cancelled items
share the Done lane and the Done view (`spec/board.md`).

The `lifecycle` field is a single **atomic register** holding a plain
`LoroValue` list `[state, at]` — the current workflow state and the
unix-millis time it was entered. A plain value (unlike a child container) is
written in one op and merged whole by last-writer-wins, so the state and its
timestamp can never be torn apart by concurrent edits — the same atomicity
rationale as `location`, without the string encoding (Loro value lists don't
need one).

**Bin is not a workflow state.** `binned_at` is an independent optional
register: present ≡ binned, and it *masks* whatever the workflow register
holds. Restore deletes the key and the preserved workflow state (which may be
Done) is revealed — for free, since the register was never touched. This is
the one piece of masking kept from the old design; the workflow ladder itself
has none.

Reading:

- Register **absent** ⇒ `[Backlog, created_at]`. New items omit both fields.
- Register **unparseable** — wrong shape, non-integer members, or an
  unrecognized `state` (e.g. written by a newer client) ⇒ the same fallback. A
  future state degrades to Backlog: visible and open, never silently hidden.
- Resolved lifecycle = `Binned` when `binned_at` is present, else the
  register's state.

Projections:

- **Open** = `binned_at == null && state <= Review` (Backlog | Todo |
  In Progress | Review). The four open states share the list's single manual
  order — the state partitions Open into board lanes without reordering
  anything (`spec/board.md`).
- **Done view** = `binned_at == null && state is Done | Cancelled` (closed,
  not binned), sorted by the register's `at` desc (id asc tiebreak).
  Cancelled rows render muted with a cross; clients may offer a display-only
  "hide cancelled" filter.
- **Bin view** = `binned_at != null`, sorted by `binned_at` desc.

`ItemView::lifecycle()` returns the resolved `ItemLifecycle`; `ItemView` also
exposes the register's `at` (as `lifecycle_at`) and `binned_at` for the
timestamp sorts and display.

**Concurrency.** Two independent LWW registers, merged per-field, resolved by
one precedence rule (binned mask wins while present) — deterministic on every
replica. Within the workflow ladder there is no masking at all: concurrent
transitions converge to the later whole-value write, and **un-done lands in
Backlog** (the default state) rather than revealing anything. A concurrent
bin-vs-transition pair converges to both: the item is binned, with the other
device's state preserved underneath for restore. Restore *position* is also
exact: lifecycle writes never touch order containers (see below), so a
restored item reappears at its old place in the list order.

Binned and Done items keep their `location` (and therefore their logical
list membership) so "restore to original list" works. Hard delete (e.g.
emptying the bin) removes the key from the `items` map — Loro handles the
tombstone — and best-effort removes the item's order entries.

**Reflection stamps.** Two plain optional registers ride along with transitions
(written in the same commit) purely for the analytics/table direction:
`started_at` (first entry into In Progress, write-once) and `done_at` (last
entry into Done, never cleared). They are not part of state resolution and no
view sorts on them; a rolling `[state, at]` register alone would forget every
transition but the last, which is exactly the history the reflection lens
needs. Delete this pair (and the two write lines) to drop the feature.

## Location and placement IDs

```
Location   = "<list_id>:<placement_id>"     (value of item.location)
OrderEntry = "<item_id>:<placement_id>"     (element of order/<list_id>)
FocusRef   = "<item_id>"                     (element of focus; local doc)
           = "<doc_id>:<item_id>"            (future: cross-doc — see spec/focus.md)
```

All three are single **encoded scalar strings**. Rationale (vs a structured
`LoroValue::Map`): a scalar register is written in one op, so `list_id` and
`placement_id` can never be torn apart by concurrent edits — there are no
independently-mergeable sub-fields to conflict. It is also smaller on the wire
and trivially comparable. The separator `:` is reserved: ids are uuid-v7 hex
(`[0-9a-f]{32}`) or the literal `inbox`, so it can never appear inside a
component. Parsing splits on the **first** `:`. (A bare `FocusRef` has no `:` and
is a local-doc item id; the emitter writes only this form today — see
`spec/focus.md`.)

A `placement_id` is a fresh uuid-v7 generated whenever an item is *placed*
into a list (add, cross-list move, delete-list reassignment). It is **not**
regenerated by within-list reorders, lifecycle changes, or content edits.

### Why placement IDs are required

A cross-list move is a delete-plus-insert across two order containers plus a
`location` register write. Two devices concurrently moving the same item to
different lists each insert an entry into a different container; the CRDT
keeps both inserts. The item's `location` register resolves the conflict
(last-writer-wins on the whole atomic value); the losing insert becomes a
**stale entry**. An order entry is *visible* only when it matches the item's
winning location:

```
visible(entry in order/L) :=
       items[entry.item_id] exists
    && items[entry.item_id].location.list_id == L
    && items[entry.item_id].location.placement_id == entry.placement_id
    && no earlier visible entry in order/L has the same item_id
```

Stale and duplicate entries are therefore harmless garbage: they can never
make an item visible (wrong placement), never duplicate a visible item (first
match wins), and can be cleaned opportunistically (see Reconciliation).

## Projection invariants

- **Item location is authoritative.** The `items` map + `location` register
  fully determine which list every item belongs to. Order containers only
  order; they never own membership.
- **Stale order entries never make an item visible.** (Visibility rule
  above.)
- **Duplicate entries produce one visible item.** First visible match in
  container order wins; later duplicates are ignored.
- **Missing canonical entries never hide data.** An item whose `location`
  names list `L`/placement `p` but has no matching entry in `order/L` (lost
  to concurrent deletion, partial history, or bugs) is still projected: it is
  appended after all entry-backed items of `L`, in `(created_at, id)`
  ascending order — deterministic across replicas. This is the **fallback
  tail**.
- **Reads never mutate.** Projection (including the fallback tail) is pure;
  it never writes order entries. Materializing fallback placements into real
  entries happens only through the explicit reconciliation mutation.
- The Done view (closed items: Done and Cancelled) sorts by the workflow
  register's `at` desc (id asc tiebreak), the Bin view by `binned_at` desc;
  both are timestamp sorts, not order-container projections.

### Resolved order

The **resolved order** of list `L` = visible entries of `order/L` in
container order, then the fallback tail. It covers items of *all* lifecycle
states that locate to `L`. The **Open** projection of `L` is the resolved
order filtered to open items (`state <= Review && binned_at == null`). The
four open states share this single order — the workflow state partitions Open
into board lanes without reordering. The
resolved order — not just the Open projection — is part of logical state (it
fixes restore positions), so `doc_fingerprint` hashes it. The **Focus order** is
logical state too (it fixes the curated Focus sequence), so `doc_fingerprint`
hashes the focus container's order as well (`spec/focus.md`).

## Done / binned items stay in the order container

**Decision:** lifecycle writes (the workflow register and the `binned_at`
mask) do **not** touch order containers. A hidden item's entry stays where it
is; restore simply makes the item visible again in exactly its former
position.

Tradeoffs considered:

- *Keep entries (chosen)*: restore is deterministic and exact for free; done/
  bin toggles are single map writes (cheap, clean undo steps, no concurrent
  order churn). Cost: an order container's length grows with the list's
  *lifetime* item count, so projecting a list is O(lifetime items in that
  list). At the 13k-lifetime-items yardstick this matches today's cost for
  `inbox` while making every *other* list's projection proportional to its own
  history — and hard delete (bin emptying) is the natural pruning mechanism:
  deleting a binned item removes its entry.
- *Remove entries + restoration anchors (rejected)*: keeps order containers
  live-only (faster projection) but restore needs anchor bookkeeping that is
  itself concurrency-prone (anchor deleted/moved/hidden), turns every done/
  bin toggle into a two-container mutation, and makes undo of a lifecycle flip a
  structural edit. Complexity concentrated on the most common mutation in the
  product; rejected.

## Mutation contracts

Every mutation below forms **one Loro commit** (one undo step, one op group).

- **Add item** — create the item map in `items` (id, text, created_at),
  generate a placement id, set `location = target:placement` atomically,
  insert `"item:placement"` into `order/target` at the requested position.
- **Reorder within one list** — `MovableList::mov` on the list's order
  container. The placement id is preserved; `location` is untouched.
- **Move across lists** — generate a fresh placement id; set the new
  `location` atomically; insert the matching entry into the target order
  container; best-effort delete the old entry (and any other entries for
  this item) from the source order container. Concurrent moves converge via
  the location register; the loser's entry goes stale.
- **Set lifecycle** — every lifecycle transition is **one Loro commit** on the
  item map only (order containers are untouched — see decision above); the
  bulk variant shares one `now` across all items, and re-applying the current
  resolved state is a no-op (no commit, no event). The targets:

  | Target | `lifecycle` register | `binned_at` |
  |---|---|---|
  | Backlog / Todo / In Progress / Review / Done / Cancelled | write `[state, now]` | clear |
  | Binned | *untouched* (preserved for restore) | set now |

  Reflection stamps ride in the same commit: entering In Progress sets
  `started_at` iff absent; entering Done sets `done_at`. Entering Cancelled
  stamps nothing — it is not a completion, and throughput analytics must
  not count it as one; the register's own `at` is the cancelled time.

  The convenience transitions:

  - **Un-done** — set lifecycle Backlog (a plain register write; the workflow
    ladder has no masking, so there is no prior state to reveal). Applies to
    both closed states: reopening a Cancelled item is the same write.
  - **Restore from Bin** — clear `binned_at` only, revealing the preserved
    workflow state (which may itself be Done or Cancelled, if the item was
    closed before it was binned).

  An item's order entry never moves on a lifecycle change; restore/un-done
  reveal it in its former position (or the fallback tail if its entry was
  lost). Board drops that additionally reorder within the shared Open order
  fold the `move_item` reorder into the *same* commit.

  **Focus exception (the one second-container write).** The **Done** and
  **Cancelled** transitions additionally remove the item's focus ref(s) from
  the `focus` container in the same commit, so closing an item removes it from
  Focus and it does not return on un-done. This is the sole lifecycle
  transition that touches a container other than the item map; it is
  justified because a closed focus ref renders nothing (the Focus view is
  Open-only) and Focus must stay finite without relying on the unwired
  `reconcile()`. Transitions between the four open states — including
  into Review — never touch Focus. **Binned does not** touch the focus
  container — binned refs are filtered from the view and swept on the next
  focus interaction. See `spec/focus.md` "Lifecycle interplay".
- **Hard delete** — delete the item's key from `items`; best-effort remove
  its entries from its located order container. Entries elsewhere are
  invisible anyway (item lookup fails) and left to reconciliation.
- **Delete list** — refuses for `inbox`. Deleting a list *discards its
  contents to the bin* rather than dumping them into Home's Open view. Every item
  locating to the list (open, done *and* binned) is moved to `inbox` with a
  fresh placement, appended to `order/inbox` in the deleted list's resolved
  order, and — unless it was already binned — marked binned with a shared
  `binned_at` timestamp (already-binned items keep their original one; workflow
  registers are untouched). The
  relocation to `inbox` gives each item a real home list for when it is later
  restored from the bin; the bin move keeps discarded items out of every
  live view without losing them. The ListMeta row is deleted. The abandoned
  `order/<list-id>` container remains as unreachable history (root containers
  can't be deleted; it simply stops being projected).

### Reconciliation

`reconcile()` is an explicit, idempotent maintenance mutation (never run
implicitly by reads): for every list it removes stale/duplicate entries and
appends real entries for fallback-tail items (using each item's existing
placement id). It also prunes **focus** refs that are missing / closed / binned /
foreign and dedups them (`spec/focus.md`). One commit; a no-op when the doc is
clean. Clients may run it opportunistically (e.g. after bootstrap); nothing
depends on it running — Focus stays bounded via auto-remove-on-close and the
sweep folded into each focus mutation.

## ListMeta

| Field | Type | Notes |
|---|---|---|
| `id` | string | uuid v7 hex; stable, used in `Location.list_id` |
| `name` | string | display name |
| `icon` | string? | user-chosen display icon, stored as the literal emoji grapheme. Absent/empty ≡ no icon; clients render a built-in fallback glyph. |
| `view` | string? | the list's **saved default view** — an encoded scalar (`"list"` / `"board"` / `"board:<visible lanes>"`) written as one register so a concurrent save can't tear the mode apart from the lane set. Absent (or unparseable, e.g. written by a newer client) ≡ no saved default: clients fall back to their own. Clients may override it locally. See `spec/board.md`. |
| `archived_at` | i64? | unix millis when the list was archived. **Absent ≡ active; present ≡ archived.** Written by `set_list_archived`; unarchiving deletes the key. Archiving is pure ListMeta metadata — see "Archived lists" below. |
| `created_at` | i64 | unix millis |

### Archived lists

Archiving is the user-facing way to remove a list from the active workspace
without destroying anything. It is a single-register write on the ListMeta row
(`archived_at`), and **nothing else**:

- Item `location` registers, placements, and order containers are untouched.
- Item lifecycle, timestamps, notes, deadlines, and planned dates are untouched.
- Focus refs are untouched.
- The list keeps its id, name, icon, saved view, `created_at`, and its position
  in the `lists` MovableList.

Unarchiving deletes the key and the list reappears in the active projection
exactly as it was.

**Projections.** `all_lists()` remains the canonical projection: every ListMeta
row in CRDT order, archived or not — archived lists never disappear from it.
Clients derive two views from it: the **active** projection (`archived_at`
absent — the main nav, capture/move destinations, keyboard nav) and the
**archived** projection (`archived_at` present — the archived section).
Archived lists stay searchable, their boards stay intact, and items locating to
them keep rendering the list's name.

**`set_list_archived(list_id, archived)`** — refuses for the reserved `inbox`.
Archiving an active list writes `archived_at = now`; unarchiving deletes the
key. Re-applying the current state is a no-op: no commit, no event. One commit
otherwise; emits `ListArchivedChanged { id, archived_at }`. `ListAdded` also
carries `archived_at` so snapshot/backfill consumers materialize archived lists
correctly.

**Reordering.** Because clients render only active lists in their draggable
nav while the `lists` MovableList still holds archived rows in place, an
active-index drop target does not equal the raw CRDT index. `move_list` on an
**active** list therefore interprets `target_index` as a position within the
active-list projection and resolves it to the raw index itself (moving an
archived list — no client UI does today — keeps raw-index semantics). The
emitted `ListMoved.index` remains the list's position in the full `all_lists`
projection.

`delete_list` remains an **internal destructive primitive** (tests, future
permanent-deletion work); it is not exposed as a user-facing action in any
client UI.

Whether the nav shows an open-item count (all Open states) beside each list is governed by a single doc-level flag — see `WorkspaceSettings.show_list_counts`. There is no per-list override; Inbox's count is always shown regardless.

## Built-in lists

Monoplan has one reserved primary capture list:

- `inbox` — rendered as "Inbox". This id is reserved and addressable by items,
  but it is not stored as a `ListMeta` row in the `lists` MovableList. Its
  order container is `order/inbox`. Its label is client-defined (the localized
  built-in) and it is non-renamable, non-movable, and non-deletable — there is
  no display-name override. Doc-level settings for it live in `settings`.

The bin is *not* a list; it's the `binned_at` mask on items.

**Focus** is a reserved lens, not a list — the `focus` container of FocusRefs
(`spec/focus.md`), projected as a flat ordered view of Open referenced items. It
is non-renamable and non-deletable, and like the bin it is not a `ListMeta` row.

## WorkspaceSettings

Doc-level synced settings that are not owned by any specific `ListMeta`.

| Field | Type | Notes |
|---|---|---|
| `show_list_counts` | bool? | when true, clients render each non-Inbox list's open-item count (all Open states) in the nav (subject to a `count > 0` gate). Inbox's count is always shown regardless. Absent ≡ false; the mutation deletes the key on the off path so an unset flag leaves no on-disk trace. |
| `inbox_view` | string? | the reserved `inbox` list's **saved default view**. Inbox has no `ListMeta` row, so its `view` register lives here — same encoding, same absent ≡ no-default reading. See `spec/board.md`. |

## Mutations (rust core API surface)

All mutations go through Loro APIs internally; the core exposes typed helpers:

- `add_item(list_id, text) -> ItemId`
- `move_item(item_id, target_list_id, target_index)` — in-list reorder when
  `target_list_id` equals the current list (order `mov`, placement kept);
  cross-list move otherwise (fresh placement, atomic location write,
  entry delete+insert). One commit either way.
- `set_item_lifecycle(item_id, lifecycle)` / `set_items_lifecycle(item_ids, lifecycle)` — move one or many items to an `ItemLifecycle` (`Backlog | Todo | InProgress | Review | Done | Cancelled | Binned`) in a single commit, writing the `[state, at]` register / `binned_at` mask (plus reflection stamps) per the transition table above. This is the primitive the board uses; the `done`/`bin`/`restore`/`un-done` helpers below are convenience wrappers over it.
- `edit_item_text(item_id, text)`
- `set_item_deadline(item_id, deadline)` — `Some(date)` validates a `YYYY-MM-DD`
  calendar date and writes the `deadline` register; `None` deletes the key. One
  commit. Rejects malformed dates with `Invalid`.
- `set_item_when(item_id, when)` — `Some(value)` validates `YYYY-MM-DD` or
  `YYYY-MM-DDTHH:MM` and writes the `when` register with the trimmed value;
  `None` deletes the key. One commit. Rejects anything else (seconds, offsets,
  the reserved zone suffix) with `Invalid`. Emits `ItemWhenChanged { id, when }`;
  `ItemAdded` carries `when` too. Export dumps carry `when` only when set.
- `add_list(name) -> ListId`
- `rename_list(list_id, name)`
- `set_list_archived(list_id, archived)` — archives (`true`) or unarchives
  (`false`) a user list; refuses for `inbox`. Metadata-only: performs **no**
  item, order, lifecycle, or Focus mutations. No-op (no commit, no event) when
  the list is already in the requested state. See "Archived lists" above.
- `set_show_list_counts(show)` — toggles the doc-level "show counts on non-Inbox lists" flag. Inbox's count is always visible (subject to count > 0) and is not gated by this.
- `set_default_view(list_id, view)` — saves (`Some`) or clears (`None`) a list's default view as one encoded register; accepts the reserved `inbox`, whose value lands in `settings.inbox_view` and reports via `SettingsChanged` rather than `ListDefaultViewChanged`. One commit; a no-op when unchanged. See `spec/board.md`.
- `delete_list(list_id)` — refuses for `inbox`; see "Delete list" contract above. **Internal-only**: not surfaced in any client UI (archive is the user-facing removal).
- `empty_bin()` — hard-deletes all `Binned` items.
- `delete_binned(item_id)` — hard-deletes one `Binned` item.
- `add_to_focus(item_id, index)` / `remove_from_focus(item_id)` / `move_in_focus(item_id, index)` — curated Focus lens mutations over the `focus` container (`spec/focus.md`). Each is one commit and sweeps dead focus refs. `add_to_focus` no-ops when the item already has a visible ref.
- `focus_view()` / `focus_refs()` — the Focus projection (Open, deduped, resolved order). Pure reads.
- `reconcile()` — explicit stale/duplicate/missing order-entry repair plus focus
  ref pruning/dedup; see Reconciliation.

The wire format for ops is whatever Loro emits — opaque bytes from the server's POV.

## Schema versioning & compatibility

This layout is a **breaking CRDT-schema change** from v1 (document-wide
`items` MovableList), and it is a **clean break** — no in-doc migration, no
legacy bridge, no data-carry-over guarantee while pre-release:

- The **wire protocol version stays at 1** (`spec/sync-protocol.md`): frames
  are unchanged and op blobs are opaque to the protocol, and with a single
  pre-release user there are no old clients to fence off at the handshake.
  The cutover is operational: export JSON on the old build, wipe the
  account/local databases, import on the new build. (Loro root containers
  are typed by (name, type), so v2 code opening stray v1 bytes sees empty
  containers rather than garbage — but don't mix them; wipe.)
- Pre-v2 accounts, local databases, and server op logs are simply discarded
  and re-created.

### Inbox rename (`main` → `inbox`) — a v2-internal cutover

Renaming the reserved list's stored id from `main` to `inbox` is a **stored-data
change, not additive**: the reserved literal appears in every reserved-list
item's `location` register and in the order-container name (`order/main` →
`order/inbox`). The container *shapes* are unchanged (only the reserved literal
and the `order/*` name differ), so **no schema-version renumber** — it stays v2 —
but it rides the same one-time **export → wipe → import** cutover as the v1→v2
break above, run once on the live doc at a clean checkpoint.

The cutover was a one-time export → edit → import; the importer no longer
aliases `main` ⇒ `inbox`. Exports carry `list_id: "inbox"` for reserved-list
items.

### Focus container — additive within v2

The `focus` container (`spec/focus.md`) is **additive within schema v2**: it
reinterprets no existing container, and Loro roots are typed by `(name, type)`,
so a focus-unaware v2 client simply never projects it. No version bump; the
sqlite migration (`001_init.sql`) is unaffected (opaque blobs).

### Workflow register — the v2 → v3 break

Replacing the v2 item fields `live` / `done_at`-as-state with the atomic
`lifecycle` workflow register (widening the open ladder to Backlog | Todo |
In Progress | Review) changes the meaning of every item's stored shape:
**schema version bumps to 3**. (`binned_at` keeps its v2 shape and meaning.)
Same clean-break policy as v1 → v2 — wire protocol stays 1, `001_init.sql` is
untouched (op blobs are opaque), and the cutover is the one-time
**export → wipe → import** at a clean checkpoint.

The JSON export carries `lifecycle: { state, at }` for every item, with
`state` as a name — `"backlog" | "todo" | "in_progress" | "review" | "done" | "cancelled"` —
plus `binned_at` and the reflection stamps when present. `lifecycle` is
required on import; there is no mapping from the v2 `live` / `done_at` shape
(the v2 → v3 cutover was a one-time export → wipe → import and its importer
bridge has been removed). An unrecognized `state` name degrades to Backlog.

### Notes text container — the v3 → v4 break

`notes` changes from a whole-string LWW register to a mergeable `LoroText`
child of the item map (`spec/notes-plan.md`, Phase 1): **schema version
bumps to 4**. `text` stays a string register. Same clean-break policy as
before — wire protocol stays 1, `001_init.sql` is untouched, and the
cutover is the one-time **export → wipe → import** at a clean checkpoint.
A v4 build opening a v3 doc reads every item's notes as empty, and the
first notes write to such an item fails (`ensure_mergeable_text` refuses a
key that holds a plain value); do not mix, wipe.

The JSON export shape is unchanged (`notes` is a string in both), so a v3
export imports into v4 as-is; the importer writes non-empty notes into the
text container.

Loro realises a mergeable child as a root container with a derived name
(`🤝:…>notes`) plus a small binary marker in the parent map slot. Diff
translation therefore routes text diffs by event path
(`[items, item map, text]`), not by target container.
