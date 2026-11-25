# Inbox

Built: the queue 2026-11-16, "One Inbox" 2026-11-25, the multi-select
filter 2026-11-26 (core, api / FFI, wasm, web, iOS, macOS; the CLI reads
one workspace's Inbox). This file is the contract as built. The original
plan and the steps between are in git history.

Inbox is an **arrival-ordered queue** and the **catch-all** for any item
with no live list. Every workspace keeps its own Inbox bucket, and the
user sees **one Inbox** over all of them. It can only be emptied by
dealing with what is in it (move, finish, bin, date), never by arranging
it.

## Why

- **An inbox is a queue, not a workspace.** With no manual order there is
  nothing to tidy. The only ways to make Inbox smaller are the ones that
  process it.
- **An inbox is a place to empty; two places to check is one too many.**
  Hence one Inbox across workspaces, with a filter to narrow it.
- **No order container.** Closed items keep their entries in a list's
  order container, and Inbox sees the most captures, so `order/inbox`
  grew the fastest. A queue needs none.
- **List delete is cheap**, and its closed items land somewhere sensible.
- **It closes a convergence gap.** If device A moves an item into list X
  while device B deletes X, the item's `location` names a list that no
  longer exists. Under the catch-all rule it shows up in Inbox.

## Model (per doc)

### Membership

`inbox` is the reserved id of a doc's no-list bucket, in the core, URLs
and every API. It has no ListMeta row. An item is **in Inbox** when:

1. it has no `location` key (the normal form: Inbox is the absence of a
   list);
2. its `location` is unparseable, or a leftover `inbox:<placement>` from
   an older doc;
3. its `location` names a list id with no ListMeta row (an *orphan*).

All three read the same, so there is no migration. Archived lists still
have their row, so their items are not orphans. No placement id is minted
for Inbox.

**Shown.** Inbox's Open projection leaves out items that have a `when`: Inbox is
what has **no list and no date**. A dated item with no list, task or
event, shows on the calendar instead (an Open task goes to Overdue once
its day passes, so nothing is lost by dating it). It is still located in
Inbox and is listed there again, at its place in the queue, the moment
its date is cleared. User lists are unaffected: they show every open
item, dated or not.

**The "Calendar" label.** The no-list bucket has two names, chosen by whether the item has a date.
Every surface that names an item's list (Done, Bin, search, Starred, the
item's list field) says **"Calendar"** for a dated item with no list and
"Inbox" otherwise, closed and binned items included. Move targets follow:
the item dialog's list picker and the move palette call each workspace's
no-list option "Calendar" when what is being moved has a date (the
palette only when every selected row is dated; a mixed selection says
"Inbox"). It is a label only: the id stays `inbox`, and "Calendar" is
never a list or a move target of its own.

### The `inbox_at` register

| Field | Type | Notes |
|---|---|---|
| `inbox_at` | i64? | unix millis when the item last **deliberately arrived** in Inbox. Absent ≡ `created_at`. Read only while the item is in Inbox. Plain LWW register. |

It sits on the item, not on the list, so it survives compaction. (A list
tombstone was rejected: deleting the row and trimming history would erase
the only record of when its items arrived.)

**Writes `inbox_at = now`:** a move into Inbox from a list (any client
path); a cross-doc move whose target is the other workspace's Inbox (the
adopt side); `delete_list`, for every item it relocates (one shared
`now`).

**Leaves it alone:** capture into Inbox (absence already reads as
`created_at`); a move out (the stale value is ignored and the next move
in overwrites it); every lifecycle change, restore from the bin included,
so an item binned from Inbox and restored goes back to its old place;
content edits.

An orphan from the concurrent case keeps whatever `inbox_at` it last had,
or none, so it sorts near the top of an oldest-first queue, the most
visible spot.

### Order

The resolved order of a doc's Inbox, over all lifecycles, is ascending by

```
(inbox_at ?? created_at, created_at, id)
```

The key has one definition, `monoplan_core::inbox::key`. Items a deleted
list moved in share one `inbox_at`, so the tiebreak orders them by
creation time; the deleted list's manual order is lost. The resolved
order is part of logical state and `doc_fingerprint` hashes it; it is a
pure function of item registers and the `lists` id set, so replicas
agree.

`order/inbox` is dead: never loaded, never written, skipped by
`reconcile()`, and remote ops on it are dropped. Entries in older docs
stay as unreachable history.

## Mutation contracts

Each is one Loro commit.

- **Add to Inbox:** create the item map with no `location`, no order
  entry and no `inbox_at`. A requested index is ignored.
- **Move into Inbox:** delete `location`, write `inbox_at = now`,
  best-effort delete of the source list's order entries. A requested
  index is ignored.
- **Move out of Inbox:** as any cross-list move; writes `location`.
- **Reorder within Inbox:** refused with `Invalid` (`move_item` to the
  same list at another index, `reorder_items`, `plan_reorder` callers).
- **Delete list** (`delete_list(list_id, mode)`): refuses for `inbox`.
  Every item located in the list (open, closed, binned) loses its
  `location` and gets the shared `inbox_at = now`; with `ToBin`, open
  items also get `binned_at = now`. Deleting the ListMeta row finishes
  the commit, so one undo restores the row and every register. Workflow
  registers, stamps, notes, dates, place and Starred refs are untouched.

  **Every client passes `ToBin`** and confirms once ("Its open items go
  to the Bin", plus a note naming a shared workspace). Done and cancelled
  items stay closed and unbinned with no list, so the Done view shows
  them under Inbox, and one reopened joins the queue's end. `ToInbox`
  stays in core, `api::lists::DeleteMode`, the FFI and the wasm
  `deleteList(id, toBin)` with no caller. Delete is offered wherever the
  list menu is, archived lists included, and is not owner-only (it is
  content, and one undo reverses it).

  Relocating items explicitly, instead of leaving them pointing at the
  dead id, keeps `location` clean for everything that reads it; orphan
  membership remains only as the safety net for the concurrent case.

## Core projection

`ProjectionIndex` (`core/src/doc.rs`):

- `ItemMeta` carries `located_in` (the raw `location` list), `list_id`
  (the effective one: `inbox` for an orphan) and `dated`; the index holds
  `live_lists`.
- Inbox is a list whose every item is in the fallback tail, sorted by the
  queue key; an Inbox change re-sorts with `refresh_open` (O(n log n)
  over Inbox's items).
- `listed()` (open, and not a dated item of Inbox) is what
  `open_members` / `open_by_list` are over; `ItemView::is_listed` is the
  same rule for clients. Only Inbox differs from plain open-ness, and it
  has no order entries, so no ordered list ever has a hidden open row.
- A remote or undo frame that adds or removes a list row takes the
  rebuild fallback (`FullResync`) when an item it didn't touch is located
  in that list, or when a returning row's order container still has
  entries (undoing a delete). Order ops on a row-less list are dropped.
- Events: `inbox_at` is on `ItemView`, in exports and in the fingerprint,
  but on no event; clients take Inbox order from `open_index` like every
  list. `ItemWhenChanged.open_index` is `None` when a date arrives
  (consumers drop the item from Inbox's Open array) and the queue
  position when a cleared date brings it back; `ItemListChanged.
  open_index` reports the same for a move (`None` moving a dated item
  into Inbox).

The core still accepts `set_default_view("inbox", …)`
(`settings.inbox_view`), but no client offers an Inbox board or reads
the setting: Inbox is a flat list everywhere.

## One Inbox

Each doc is its own E2EE container, so no single container can span
them. The one Inbox is a **read** over every open doc's Inbox, merged
into one queue. No sidebar row, view, URL or title names one workspace's
Inbox.

- **Rows.** What each doc's Inbox shows (Open, not binned, no date),
  merged oldest first on the queue key, then doc id and item id. A row of a
  shared workspace names it at its far (right) edge, whatever the
  filter; a Personal row names nothing.
- **Shared workspaces contribute everything** in their Inbox, other
  members' captures and a deleted shared list's closed items included:
  triage is everyone's job in a small team. Narrowing to one's own
  captures or assignments was considered and left until it proves noisy.
- **Filter** (multi-select). "All workspaces" first, checked by default;
  then a check per workspace, any number at once. Checking "All", or
  Clear, drops the picks; checking every workspace is "All". It is a
  device preference (the picked doc ids): the Inbox opens the way it was
  left. A pick naming a doc no longer open is ignored and kept; when no
  pick is open the Inbox shows all. Shown only while more than one
  workspace is open.
- **Counts** (nav, header, Home row) are of the rows the filter shows.
  The title is "Inbox" whatever the filter.
- **Reveal.** Showing an Inbox item of a workspace the filter hides
  (search, "Show in List", a capture, a link, a workspace just created or
  joined) resets the filter to all.
- **Capture** lands in Home's Inbox while Personal is shown (all, or
  picked), else in the first shown workspace's Inbox.
- **Drop onto the Inbox entry** sends each row to its own workspace's
  Inbox: nothing crosses a workspace, so nothing needs confirming. One
  undo step per workspace.
- **Moves out** are the usual list moves; into another workspace's list
  they are the cross-doc move (`sharing-plan.md`).
- **Order preference.** Newest first is a device preference on iOS and
  macOS (`inboxNewestFirst`); it reverses the display only. The web is
  always oldest first.

### Where it lives

| Layer | |
|---|---|
| core | `monoplan_core::inbox` (`core/src/inbox.rs`): `scope(open, filter)`, `doc_rows(doc, keep)`, `key` / `queue_key`, `merge` |
| api / FFI | `api::inbox::inbox(cx, filter)` (`cli/src/api/inbox.rs`), `InboxEntry { item, doc_id }`; FFI `Engine.inbox(filter:)` (doc ids, empty ≡ all) |
| wasm | `InboxMux` (`new(filter)`, `add(docId, replica, keep)` per open doc, `ids()` the merged queue; `keep` is the linger beat's just-closed rows) and `inboxScope(open, filter)` |
| web | view `{ kind: "list", id: "inbox" }`, never with a `docId` (`docs.listView(_, "inbox")` returns it), `#inbox` its only URL (`urls.md`); `combinedInbox` in `Workspace.tsx` over `InboxMux`; one top-group "Inbox" nav row (`data-drop-inbox-own`); the header filter is the Done menu with an "All workspaces" option over its workspace facet; the filter in `inboxScope.ts` (localStorage JSON array, a signal); no drag-reorder, no display options; a capture's draft row opens at the queue's end |
| iOS | `InboxView.swift` over `AppModel.inbox`, one Home row first in the top group (Personal's section shows only with user lists), a toolbar filter menu of toggles that stays open (`@AppStorage("inboxFilter")`, doc ids comma-separated, empty ≡ all), Order in the "..." menu; Move in selection mode only within one workspace; a capture scrolls to the new row; type-checked, not run |
| macOS | `Lens.inbox` first in the sidebar's top group (Order in its context menu), `InboxFilter.swift` (checkbox popover, "All Workspaces" first, Clear; `UserDefaults` `MonoplanInboxFilter` string array), `select(workspace:list: "inbox")` maps to the lens; sidebar drop groups by doc into `moveItems(…, "inbox")`; rows never reorder |
| CLI | no combined read: `item ls` shows one workspace's Inbox in queue order (`--newest` reverses), `item add --top` into Inbox is an error, `mv` into Inbox goes to the end, `list delete <list> [--yes]` (`cli.md`) |

The scope rule (`scope`) and the merge are Rust's on every client. The
filter's bookkeeping around it (which picks are open, the toggle's
"every workspace is All", the capture workspace) is written once per
client: `Workspace.tsx`, `AppModel.swift`, `MainWindow.swift`.

## Tests

- Capture into Inbox writes no `order/inbox` entry and sorts by
  `created_at`.
- A move into Inbox stamps `inbox_at` and goes to the end of the queue,
  even for an old item.
- Restore from the bin keeps the old position; un-done keeps it too.
- Reorder in Inbox is refused.
- `delete_list`: both modes; closed items are in the Done view located in
  Inbox, binned items stay binned, one undo restores everything.
- Concurrent move-into-X and delete-X on two replicas: the item is in
  Inbox on both, the fingerprints match, and its order uses its prior
  `inbox_at`. Undoing the delete returns the orphan to the restored list.
- An old doc with a populated `order/inbox` projects the queue order and
  ignores the entries.
- A date takes an item out of Inbox's Open array and clearing it brings
  it back in place, locally, remotely and through undo.
- `inbox::scope` (filter, or everything), `doc_rows` (lingering closed
  rows only when asked), `api::inbox` (merge across docs in arrival
  order; the filter narrows).

## Open

- The filter's bookkeeping could move behind `Engine.inbox` / the wasm
  mux (shown workspaces and capture target returned with the rows), so
  the three clients stop repeating it.
- `settings.inbox_view` and `DeleteListMode::ToInbox` have no client.
