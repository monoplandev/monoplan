# Inbox as a queue

Built 2026-11-16 (decided the same day): core, api / CLI / FFI, web, iOS,
macOS. "As built" at the end records where the build settled the open
questions or differs from the plan.

Inbox stops being a hand-ordered list. It becomes an **arrival-ordered queue**
and the **catch-all** for any item that has no live list. Two consequences
follow: deleting a list becomes a cheap, user-facing action, and Inbox can only
be emptied by dealing with what is in it (move, finish, bin), never by
arranging it.

## Why

- **An inbox is a queue, not a workspace.** With no manual order there is
  nothing to tidy. The only ways to make Inbox smaller are the ones that
  process it. This constraint is the point, including when a deleted list
  lands 40 items there.
- **`order/inbox` is the worst order container.** Closed items keep their
  entries (`data-model.md` "Done / binned items stay in the order
  container"), and Inbox sees the most captures, so its container grows the
  fastest. A queue needs no container.
- **List delete is clumsy today.** `delete_list` relocates every item, mints
  placements, appends to `order/inbox` and bins everything, and no client
  exposes it (`data-model.md` "Archived lists"). Under this plan it rewrites
  two registers per item, and items land somewhere sensible.
- **It closes a convergence gap.** If device A moves an item into list X
  while device B deletes X, the item's `location` names a list that no longer
  exists. Today nothing projects it, so it is effectively lost. Under the
  catch-all rule it shows up in Inbox.

## Model

### Membership

An item is **in Inbox** when its `location` list id is any of:

1. `inbox`;
2. missing or unparseable (already true today: `item_meta` in `core/src/doc.rs`);
3. **new:** a list id with no ListMeta row in `lists` (an *orphan*).

**Shown in Inbox** is narrower (decided and built 2026-11-16, later the
same day): Inbox's Open projection leaves out items that have a `when`.
Inbox is what has no list and no date. A dated item with no list, task or
event, shows on the calendar instead; an Open task among them goes to the
calendar's Overdue once its day passes, so nothing is lost by dating it.
The item is still located in Inbox: it is listed there again the moment
its date is cleared, at its place in the queue, and every other surface
(Done, Bin, search, Starred, the item's list field) labels it
**"Calendar"**, not "Inbox": the no-list bucket has two names, chosen by
whether the item has a date. That is a label only. The id stays `inbox`
in the core, URLs and every API, "Calendar" is never a list or a move
target of its own. Move targets follow the same naming, in every
workspace offered: the item dialog's list picker and the move palette
call each workspace's no-list option "Calendar" when what is being moved
has a date. The palette does so only when every selected row is dated; a
mixed selection says "Inbox". User lists are unaffected: they show every open item, dated or
not. See "As built".

Archived lists still have their row, so their items are not orphans. (When
this was written `events` was a second reserved list; it is gone since
2026-11-16 and an item still located there is an orphan like any other,
`events-plan.md`.) Nothing else about
`location` changes: an Inbox item still carries `inbox:<placement>`, and the
placement id is still minted on every move in (that keeps the location codec
and the cross-list move path uniform), but no order entry ever matches it.

### The `inbox_at` register

A new optional item register:

| Field | Type | Notes |
|---|---|---|
| `inbox_at` | i64? | unix millis when the item last **deliberately arrived** in Inbox. Absent ≡ `created_at`. Read only while the item is in Inbox; ignored elsewhere. Plain LWW register. |

It sits on the item, not on the list, so it survives compaction. A list
tombstone (`deleted_at` on a ListMeta row) was considered and rejected:
deleting the row and trimming history would erase the only record of when its
items arrived.

**Writes `inbox_at = now`:**

- a move into Inbox from another list (`move_item`, `move_items`, a nav drop,
  the list picker, the CLI);
- a cross-doc move whose target is the other workspace's Inbox (adopt side);
- `delete_list`, for every item it relocates (one shared `now`).

**Leaves it alone:**

- capture (`add_item` into Inbox), because absence already reads as
  `created_at`;
- a move out of Inbox (the stale value is ignored, and the next move in
  overwrites it);
- every lifecycle change, including restore from the bin, so an item binned
  from Inbox and restored goes back to its old place in the queue, the same
  way restore works everywhere else;
- content edits.

An orphan created by the concurrent case keeps whatever `inbox_at` it last
had, or none. It sorts by that older time, which puts it near the top of an
oldest-first queue, the most visible spot. That is acceptable for a rare edge.

### Order

The **resolved order** of Inbox, over all lifecycles, is ascending by

```
(inbox_at ?? created_at, created_at, id)
```

The **Open** projection filters that order exactly as other lists do. Items a
deleted list moved in share one `inbox_at`, so the tiebreak orders them by
creation time; the deleted list's manual order is lost. That is fine for a
queue.

The resolved order is still part of logical state, and `doc_fingerprint` keeps
hashing it. It is now a pure function of item registers and the `lists` id set,
so replicas agree.

**Reverse order** (newest first) is a **device preference**, not doc state,
handled like list counts: a web cookie, macOS `UserDefaults`, iOS
`UserDefaults`. It reverses the display only and never writes anything. Oldest
first is the default.

### Board view

Inbox keeps its saved view (`WorkspaceSettings.inbox_view`) and its board.
Every lane sorts by the same key. Dragging between lanes is a lifecycle change
only. No drop names a position inside Inbox, and the board does not fold a
reorder into the commit (`board.md`).

### `order/inbox`

After this change, `order/inbox` is dead. Nothing reads it, nothing writes it,
and `reconcile()` skips it. Existing entries stay as unreachable history, like
an abandoned `order/<list-id>` after a delete. No migration and no schema bump
(pre-release: `no-protocol-version-bumps-prerelease`). An old client that still
reorders Inbox writes entries nothing projects.

## Mutation contracts

Each change below is still **one Loro commit**.

- **Add item to Inbox:** create the item map and set
  `location = inbox:<fresh placement>`. Write no order entry and no
  `inbox_at`. A requested index is ignored.
- **Move into Inbox:** fresh placement, atomic `location` write, write
  `inbox_at = now`, and best-effort delete of the source list's entries, as
  today. Insert no entry into `order/inbox`. A requested index is ignored.
- **Move out of Inbox:** as any cross-list move. There is normally no source
  entry to delete; the best-effort delete still runs for old docs.
- **Reorder within Inbox:** refused with `Invalid`. This covers `move_item`
  with the same list and a different index, `reorder_items`, and
  `plan_reorder` callers.
- **Delete list** `delete_list(list_id, mode)`, where `mode` is `ToInbox`
  (default) or `ToBin`. It refuses for `inbox`. For every item
  located in the list (open, closed, binned):
  - `location = inbox:<fresh placement>`
  - `inbox_at = now` (shared)
  - `ToBin` only: `binned_at = now` on open items (since 2026-11-16; before,
    every item not already binned). Done and cancelled items land closed and
    unbinned in Inbox.

  Workflow registers, stamps, notes, dates, place and Starred refs are
  untouched. Closed items therefore show in the Done view with Inbox as their
  list, and items already binned stay binned. Deleting the ListMeta row
  finishes the commit, so one undo restores the row and every register.

  Relocating items explicitly, instead of leaving them pointing at the dead
  id, keeps `location` clean for everything that reads it: search badges,
  the "in list" label, export, cross-doc move. Orphan membership remains
  only as the safety net for the concurrent case.

## Core projection

In `ProjectionIndex` (`core/src/doc.rs`):

- Hold the set of live list ids, maintained on `ListAdded`, `ListRemoved` and
  rebuild.
- Inbox's membership is `members["inbox"]` plus the members of every id not in
  that set. Fold orphans in when their list disappears, and back out if the
  row comes back (undo, or a concurrent re-add of the same row).
- Inbox's projection is a sort, not a container walk: keep `open_by_list
  ["inbox"]` sorted by the key and binary-insert on change. An `inbox_at` or
  `created_at` change on an Inbox item re-sorts that one item.
- Add `inbox_at` to `ItemMeta`, `ItemView`, `ItemAdded`, a new
  `ItemInboxAtChanged` event (or fold it into `ItemListChanged`, which always
  accompanies it except for a remote concurrent write), export dumps (when
  set), and import.

The cost is O(log n) per change and O(n log n) on rebuild for Inbox, where n is
its item count. That is small by design and cheaper than walking a container
that only grows.

## Clients

| Surface | Change |
|---|---|
| core / api | contracts above; `api::moves::reorder_items` refuses Inbox; `api::lists::delete` exposes both modes; FFI `Engine.delete_list(workspace, list, mode)` |
| CLI | `list delete <list> [--bin]`, with a confirm prompt showing the open count, skipped by `--yes`; `mv` into Inbox ignores a position and says so; Inbox output follows the queue order, `--newest` reverses it |
| web | no drag-reorder inside the Inbox list or Inbox board lanes (lane-to-lane still works); nav drops onto Inbox still move; "Delete list…" in the list menu next to Archive, opening a dialog: "Move N open items to Inbox" (default) / "Bin them", plus a note when the workspace is shared ("Everyone in *Name* will see them"); reverse-order toggle in the Inbox header |
| iOS | `HomeView` drops `.onMove` for Inbox; delete list in the list page menu with the same two choices; reverse toggle |
| macOS | sidebar list menu gains Delete; Inbox table never offers row reorder (input to the `mac-dnd-plan.md` v2 spec) |

**After capture:** with oldest first, a new item lands at the bottom. Clients
scroll to it or flash it so capture still gives feedback. With newest first,
it lands at the top, as today.

**Shared workspaces:** Inbox belongs to the workspace, so deleting a shared
list puts its open items in every member's Inbox. That is intended. Delete is
not owner-only (it is content, and one undo reverses it), but the dialog
states who will see the result.

## Tests

- Capture into Inbox writes no `order/inbox` entry and sorts by `created_at`.
- A move into Inbox stamps `inbox_at` and goes to the end of the queue, even for
  an old item.
- Restore from the bin keeps the old position; un-done keeps it too.
- Reorder in Inbox is refused, and `plan_reorder` callers get `Invalid`.
- `delete_list(ToInbox)`: open items are open in Inbox in `(now, created_at)`
  order, closed items are in the Done view located in Inbox, binned items stay
  binned, one undo restores everything.
- `delete_list(ToBin)` matches today's behaviour apart from `inbox_at`.
- Concurrent move-into-X and delete-X on two replicas: the item is in Inbox on
  both, the fingerprints match, and its order uses its prior `inbox_at`.
- Undoing a delete after a concurrent move: the orphan returns to the
  restored list.
- An old doc with a populated `order/inbox` projects the queue order and
  ignores the entries.
- `reconcile()` leaves `order/inbox` alone.

## Open questions

- **Tiebreak after a list delete:** `created_at` loses the deleted list's
  order. Writing `now + i` would keep it at the cost of fake timestamps. The
  plan takes `created_at`.
- **Event naming:** a dedicated `ItemInboxAtChanged`, or carrying `inbox_at`
  on `ItemListChanged`.
- **Archive vs delete** once both are cheap: archive stays "shelve intact,
  reversible from the archived section", delete is "drop the container and
  send the work to Inbox". Is a separate "delete archived list" action
  needed, or does Delete work from the archived section too?

## As built (2026-11-16)

- **Index.** `ItemMeta` carries `located_in` (the raw `location` list) and
  `list_id` (the effective one: `inbox` for an orphan); `ProjectionIndex`
  holds `live_lists`. Inbox is treated as a list whose every item is in the
  fallback tail, sorted by the queue key, so every Inbox change re-sorts with
  a `refresh_open` (O(n log n) for n Inbox items) rather than the binary
  insert the plan sketched. `order/inbox` is never loaded, and remote ops on
  it are dropped.
- **Orphans over sync.** A remote / undo frame that adds or removes a list row
  takes the rebuild fallback (`FullResync`) when an item it didn't touch is
  located in that list, or when a returning row's order container still has
  entries (undoing a delete). Order ops on a row-less list are dropped, as the
  rebuild never loads them.
- **Event naming:** neither. `inbox_at` is on `ItemView` and in exports (and
  the fingerprint), but not on events: clients take Inbox order from
  `open_index` like every list, so no client reads the register.
- **Tiebreak after a list delete:** `created_at`, as planned.
- **Inbox is the absence of a list** (2026-11-16, after the first build):
  an Inbox item has **no `location` key**, instead of the plan's
  `inbox:<placement>`. Capture into Inbox writes none, and a move in,
  `delete_list` and an adopt into Inbox delete it; a move out writes it.
  No placement is minted for Inbox. A leftover `inbox:<placement>` from an
  older doc, an unparseable value and an orphan all read the same, so no
  migration. `inbox` stays the id every API, URL and client uses to name
  that bucket, and the product still calls it Inbox.
- **Delete always bins** (2026-11-16): the user-facing choice is gone. Web,
  iOS, macOS and `list delete` confirm once ("Its open items go to the Bin",
  plus the shared-workspace note) and call `delete_list(ToBin)`, which bins
  only open items. Done and cancelled ones stay closed and unbinned; they
  have no list row (their `location` names the reserved `inbox` bucket), so
  every surface shows them in Inbox: its board's Done lane, the Done view,
  and the queue's end if reopened. The CLI's `--bin` flag is removed.
  `ToInbox` stays in core, `api::lists::DeleteMode`, the FFI and the wasm
  `deleteList(id, toBin)` with no caller, so no binding changed.
  The surface table above records the original two-choice build.
- **Archive vs delete:** Delete is offered wherever the list menu is (web nav
  and header, iOS list page, Mac sidebar), so an archived list's header menu
  on the web offers it too; there is no separate action.
- **Move out of Inbox** does not delete old `order/inbox` entries (nothing reads
  them); the plan's "best-effort delete for old docs" was dropped.
- `reconcile()` skips Inbox entirely; a missing-location item is already in
  Inbox, so it no longer writes a location for it.
- Clients: the CLI makes `item add --top` into Inbox an error rather than
  ignoring it, since `add` prints only ids. `api::moves::move_items` onto
  Inbox skips rows already there. The Mac table had no row reorder to remove,
  and the Mac window has no capture, so the capture-feedback rule applies to
  the web (the draft row opens at the queue's end) and iOS (scroll to the new
  bottom row). The dialogs offer "Move N open items to Inbox" / "Bin them"
  and name a shared workspace (non-Home, member count not known to be one).
- **Dated items are not shown** (2026-11-16, with the `event` register,
  `events-plan.md`). `ItemMeta` carries `dated`, and `listed()` (open, and
  not a dated item of Inbox) is what `open_members` / `open_by_list` are
  over; only Inbox differs from plain open-ness, and Inbox has no order
  entries, so no ordered list ever has a hidden open row. `ItemView::
  is_listed` is the same rule for clients. `ItemWhenChanged` gained
  `open_index`: `None` when a date arrives (consumers drop an Inbox item
  from the Open array), the queue position when a cleared date brings it
  back. A move reports the same through `ItemListChanged.open_index`
  (`None` moving a dated item into Inbox, the position moving it out to a
  list). Remote and undo frames translate to the same events. The web
  opens such an item over the calendar from Find, links and "Show in";
  the Mac tray does the same.
- **"Calendar" as the label of a dated item with no list** (2026-11-16).
  Clients and the typed API name the bucket by the item's date: the web's
  `listLabel(listId, docId, dated)` and `findResultListLabel`, the item
  dialog's list picker (its `inbox` option reads "Calendar", with the
  calendar glyph, while the item has a date), `api::lists::no_list_label`
  behind `ListNames::of_item` (starred, places, find, `done`), `NoList.
  label` on iOS and `MainWindowModel.noListLabel` on the Mac. `BinEntry`
  carries `when` for it. Closed and binned items are labelled the same
  way.
