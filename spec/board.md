# Board (lifecycle view)

Board view is a **second lens on an existing list**, not a new container kind.
Every list (including the reserved `inbox`) can be viewed as a board. Archiving
a list (`spec/data-model.md` "Archived lists") changes nothing here: an
archived list's board — lanes, order, saved view — remains intact and renders
normally when the list is opened from the archived section. The board
has **five fixed lanes** driven by item lifecycle — there are no user-created,
renamed, reordered, or deleted lanes:

```
Backlog | Todo | In Progress | Review | Done
```

Bin is **not** a lane. Binned items are the existing global discarded-items
view (`spec/data-model.md`), reachable from the nav, not the board.

The lane *set* is fixed; lane *visibility* is not. A client may hide lanes
(see "Lane visibility" below) — a small board can render as few as two. Hiding
is display-only and never changes any item's state.

The flat list view and the board share one order. The list view shows the
list's **Open** projection (the four open states) in their single manual
order — the list view is the binary done | other lens: everything Open, with
Done and Binned elsewhere. The board splits that same Open order into the four
open lanes by lifecycle state and adds a Done lane. Moving an item between
lanes changes its lifecycle (`spec/data-model.md` "Lifecycle"); it does not
reorder anything by itself.

## Lanes

- **Backlog / Todo / In Progress / Review** — the list's Open items with the
  matching lifecycle state, in list order.
- **Done** — the list's closed-but-not-binned items (`state is Done |
  Cancelled && binned_at == null`), sorted by the workflow register's `at`
  **descending** (id asc tiebreak). Scoped to the current list. This is the
  per-list slice of the global Done view. Cancelled is deliberately **not a
  sixth lane**: a cancelled card sits in the Done lane with a cross instead
  of a tick, so the closed history stays in one place and the lane set (and
  its `LaneSet` bitmask / view grammar) is unchanged.

The open lanes **preserve relative order** from the list's Open projection:
an item's position is the same whether you read `order/<list-id>` linearly or
read the open lanes left-to-right, top-to-bottom. A lifecycle change moves an
item between lanes without changing its underlying order entry.

## Lane visibility

Clients may hide any lane except one (at least one lane always renders).
Hiding is a display preference: it is part of the list's **view** (see
"Default view"), so it resolves like the lens itself — a client's local
override, else the saved default — and "Save as default" publishes the visible
lane set along with the lens. It never touches item state:

- A hidden lane's items keep their state and their place in the shared Open
  order; they simply don't render on this client.
- A hidden lane is not a drop target; drags land only on visible lanes.
- Whether a hidden lane shows a collapsed stub (name + count) or vanishes
  entirely is a client presentation choice, not spec'd here.

This is what keeps five states from imposing five columns: a solo user can run
`Backlog | In Progress | Done`, or even just `In Progress | Done`, while a
team list shows all five.

### State on the flat list

The list view flattens the four open states into one order, so an item's
state is otherwise invisible there. Clients may offer a per-list, client-local
**show state** display option that badges each open row with its state label.
Like lane hiding it is display-only, never syncs, and is off by default. It
has no meaning on the board (the lane is the state) or on the Done view.

## Projection

- The board reads the list's Open projection (the core's per-list `open`
  index — see `spec/data-model.md`) and partitions it by
  `ItemView::lifecycle()` into the four open lanes. No new core projection is
  needed; the open lanes are four views of one ordered array.
- Done is a timestamp sort over the list's closed-but-not-binned items
  (done and cancelled), exactly the global Done view filtered to this
  `list_id`.
- `ItemView` carries the lifecycle state and its `at` timestamp; the state is
  the displayed lane.

## Interactions

Every lane move is one `set_item_lifecycle` commit (`spec/data-model.md`) —
the target lane *is* the target state, uniformly:

- **Drop into an open lane** (Backlog / Todo / In Progress / Review) — set
  that lifecycle state. If the drop names a target position in the shared
  Open order, fold a `move_item` reorder into the same commit.
- **Drop into Done** — set lifecycle Done (never Cancelled: cancelling is
  an explicit action — context menu, `⇧X`, the status picker — not a drop
  target). Done is timestamp-ordered, so a drop position within Done is
  ignored.
- **Drop from Done into an open lane** — set that state, for a done or a
  cancelled card alike; the item reappears in the Open order at its preserved
  entry position (drops naming a position fold the reorder in, as above).

Lanes are the state axis; lists are the location axis. A **cross-list move**
(the `m` palette, the task dialog's list picker, or a drag onto a list in the
nav) is a relocation only: it never changes lifecycle. A closed item moved to
another list stays closed and appears in the target's Done lane; reopening it
is a separate lane drop or un-done. The one exception is a *binned* item, which
is restored into the target list on move — the Bin is outside the workspace,
so a move while binned has no meaning (restore reveals the preserved state, so
a done-then-binned item arrives Done).

## Capture

- Adding in an **open lane** creates the item directly in that lane's state
  (`add_item` — the `lifecycle` field omitted for Backlog; an add variant sets
  `[state, now]` in the same commit for the other three).
- The list view's capture creates Backlog items (`add` default).

## Default view

A list carries an optional **saved default view**: the lens a client renders it
in before that client overrides it locally, and — for the board lens — which
lanes it shows. It is synced doc state — one encoded scalar register on the
list's `ListMeta` (`view`), or on the doc-level `settings` map (`inbox_view`)
for the reserved `inbox`, which has no ListMeta row. See `spec/data-model.md`.

```
DefaultView = "list" | "board" | "board:" Lane ("," Lane)*
Lane        = "backlog" | "todo" | "in_progress" | "review" | "done"
```

The lane list names the **visible** lanes in ladder order; bare `"board"` is
the canonical form for all five. Parsing is set-like (order and repeats are
tolerated and normalised on re-encode), but an unknown lane name or an empty
lane list is unparseable. Writers never emit an empty set — at least one lane
always renders.

The whole view is one register, not a mode flag plus a lane set, so a
concurrent save on another device replaces it atomically rather than merging a
mode from one device with lanes from another (same rationale as `Location`).
The list lens carries no lane state — it always encodes as bare `"list"` — so
two specs that render identically also compare identically.

Resolution, per list, on every client:

```
local override (if any)  →  saved default (if any)  →  built-in flat list
```

- Absent ≡ "no saved default", including a value written by a newer client that
  this build doesn't recognize: an unparseable register reads as absent, so a
  future lens degrades to the local fallback rather than rendering something
  wrong.
- The default only decides what an **un-overridden** client renders. Changing
  it never disturbs a client that has its own override.
- `set_default_view(list_id, view)` accepts the reserved `inbox`; that write
  lands in `settings` and surfaces as `SettingsChanged`, not
  `ListDefaultViewChanged`.
- Export/import carries it: `ExportList.view` per user list and
  `ExportSettings.inbox_view` for Inbox, both in encoded form. Import applies
  per-list views to the fresh lists it creates; like every other doc-level
  setting, Inbox's is exported for fidelity but not applied by the additive
  importer.

## Client (web) contract

- Per-list view (list ⇄ board, plus the board's visible lanes) resolves as
  above: a **local override** in `localStorage` (`monoplan:list-view`, a map of
  list id ⇒ encoded view) wins over the synced default. The same account may
  want a board on desktop and a flat list on a phone, or fewer lanes on the
  phone's narrower board.
- The override map holds only genuinely divergent lists. Choosing a view that
  matches what the client would render anyway *drops* the override rather than
  pinning it, so a client that never diverges keeps following the account.
- The view-mode popover's "Save as default" publishes the current view to the
  doc and clears this client's override for that list (it now follows the
  default it just set). The action is disabled when the current view already is
  the saved default.
- Board renders the five fixed lanes (minus locally hidden ones); there are no
  lane CRUD, rename, reorder, or menu affordances. The generic drag-and-drop
  infrastructure (placeholder, nudge, foreign-lane drop targets,
  one-transaction remove+insert) is retained; only custom-column-specific
  behaviour is removed.
- Lane visibility is part of the view spec, so it lives in the same override
  map and is published by the same "Save as default". Hiding never mutates
  item state.
- A drag between open lanes is a same-list lifecycle change: the item is
  **not** spliced out of the list's Open array (`listOpen`), it stays in place
  and its lane is recomputed from the lifecycle state. Only Done / Cancelled
  / Binned transitions remove it from `listOpen`.

## Future

Custom grouping (user-defined lanes / fields) is intentionally left
**unspecified**. The board is deliberately coupled only to the lifecycle model,
not to any lane-definition storage, so a future grouping feature is unconstrained
by this spec.
