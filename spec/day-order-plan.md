# Day order: plan

Status: plan, not built (decided 2026-11-22). The user orders the untimed
tasks of a day on the calendar by hand. Nothing else on the calendar
changes order.

Read with `calendar-plan.md` ("Within a day", where the bands are) and
`focus.md` (Starred, the pattern this copies).

## Decisions in one screen

| Question | Decision |
|---|---|
| What is ordered? | **Band 3 only**: rows placed by a 10-character `when` on a task (not an event). All-day events and deadline rows (band 1) lead the day in their fixed order; timed rows (band 2) are clock order; Overdue and "Completed today" keep their date / close order. |
| Whose order? | **Personal.** Each account orders its own days, as Starred is personal. Calendar and Starred are the two answers to "what am I working on now" (the product thesis); both are lenses, neither is a home. A member of a shared workspace reordering their day does not move anyone else's. |
| Where is it stored? | **The Home doc**, never a shared one: a root map `day_order`, one mergeable `LoroMovableList` per day under it, keyed by the day key (`YYYY-MM-DD`). |
| What is an entry? | A **FocusRef string** (`focus.md` grammar): `<item_id>` for a Home item, `<doc_id>:<item_id>` for an item in another workspace. Scalar strings only, never child containers. |
| What about rows nobody ordered? | They follow the ordered rows in today's default order (`created_at`, then id). A new task lands at the end of its day's untimed band without a write to Home. |
| One write per gesture? | **Yes, to Home only.** A reorder is one Home commit and one undo step, whichever workspaces the rows come from. |
| Schema version? | No bump. Additive, as `focus` was: a client that does not know `day_order` never projects it. |

## Storage

```
home.get_map("day_order")                      // root map, Home doc only
    .ensure_mergeable_movable_list("2026-11-22")   // one list per ordered day
        = ["<item_id>", "<doc_id>:<item_id>", ...]
```

- **Why a mergeable child under a map, not a root per day.** A named
  root (`day/2026-11-22`) would merge concurrent creation just as well,
  as `order/<list-id>` does; Loro builds a mergeable child as a root
  with a derived name anyway (`notes-plan.md` "Mergeable containers").
  The parent map is what makes the set of ordered days enumerable, which
  the fingerprint and the sweep need. Roots are not cleanly enumerable.
- **Two devices ordering the same day offline** both call
  `ensure_mergeable_movable_list(day)` and get the same container; their
  moves merge as moves in one list (Loro's MovableList: concurrent moves
  of one element resolve to one position, no duplicates from a move).
- **Never delete a day's key.** As with `notes`, a later `ensure` on the
  same key writes the marker back and resurfaces the child's old state.
  Pruning empties the list and leaves the key.
- **Day key** is the floating local date, the same string as the first
  ten characters of `when`. No zone, no `Date` construction.

## Visibility

```
visible(ref in day_order[D]) :=
     ref resolves to an item the host holds
  && the item has a band-3 row on D            // open task, 10-char when, D within its span
  && no earlier entry in day_order[D] names the same item id   // first wins, either form
```

Everything else is a **stale entry**, harmless and never shown: the item
was rescheduled, given a time, made an event, closed, binned, deleted,
or its workspace was left or hidden (`calendar_hidden_docs`). Band 1 and
band 2 rows never read the order, so a stale ref can never reorder them.

Consequences, all intended:

- **Un-done returns a task to its slot.** Ticking moves the row to
  "Completed today" (its ref goes stale, it stays in the list);
  unticking makes the same ref visible again where it was.
- **Rescheduling away and back returns it to its old slot**, since the
  old day's ref is not removed on reschedule (that would be a second
  doc's write whenever the item is not in Home). The new day shows it at
  the end of its untimed band until it is ordered there.
- **Time added then removed**: same, it returns to its old slot.
- **A slip into Overdue** (an open task whose `when` is over) leaves the
  day's order; Overdue has no bands and stays by date.
- **Multi-day all-day spans**: each covered day is a separate list, so a
  task can sit at different positions on different days of its span.

As with the focus container, Home's core cannot resolve a foreign ref
(it does not hold the other doc). The core keeps every well-formed ref
and returns a day's entries in order; the **host** (web personal layer,
`api` layer over `Ctx::workspaces`) resolves them and applies the rule
above. The core never judges a foreign ref dead.

## Projection

Band 3 of day D, given the day's band-3 rows R (from every visible
workspace) and `day_order[D]`:

1. The visible entries of `day_order[D]`, in list order.
2. Then the rows of R with no visible entry, by `created_at`, then id
   (the band's current order: their raw `when` is the same string).

Nothing else on the day moves. Both sorts must apply this identically:
`groupByDay` (web) and `build_agenda_from` (CLI, tray, widgets, iOS,
macOS). The engine reads Home's `day_order` alongside the docs it
already merges; `build_agenda_from` gains the per-day entries as an
input, and so does `groupByDay`. Reads never mutate.

## Mutations (Home doc core)

- `day_order_entries(day) -> Vec<FocusRef>`: the list as written, stale
  entries included (wasm `dayOrderEntriesJson`, as `focusEntriesJson`).
- `set_day_order(day, refs: &[FocusRef])`: the host passes the band's
  **whole visible order after the gesture**; the core edits the list to
  match with the fewest moves (`mov` refs already present, `insert` the
  missing ones in place), leaving entries not named (stale, hidden
  workspaces) where they are. One commit, one undo step. A no-op when the
  visible order already matches (no commit). Refs are validated as
  FocusRefs; existence is not checked (the core cannot for foreign ones).
  - Why a whole order rather than an index move: indices into a list
    that holds invisible entries are brittle (`focus.md` makes indices
    address the full entry list, which every caller has to get right).
    A whole order also covers the first reorder of a day, which writes
    every row's ref (the unordered tail is materialised then and only
    then), multi-select drags and ⌘↑ / ⌘↓ with one primitive.
- `retarget_focus(item_ids, doc_id)` (cross-doc move, `sharing-plan.md`)
  rewrites matching refs in **every** day list too, in the same commit,
  so a moved task keeps its slot.
- **Sweep**: `reconcile_with(Some(cutoff))` empties the lists of days
  before the cutoff day (key kept). Optional: entries are a few dozen
  bytes and a past day is never shown ordered (a past day holds only
  events and done items).

Not written by: capture, `set_item_when`, lifecycle changes, binning,
duplicate (the copy has a new id and lands at the tail; placing it
under the original, as `duplicate --star` does on Starred, is a later
nicety). None of those touch Home for this.

## Gestures

- **Reorder within a day** (web first): drag a band-3 row within its
  own day's band; ⌘↑ / ⌘↓ move the selection one row within the band
  (they do nothing on band 1 / 2 rows, Overdue, or "Completed today").
  The `calendar-plan.md` line "⌘↑ / ⌘↓ (reorder) ... do nothing here"
  changes when this lands.
- **Drop on another day at a position** (later phase): the `when` write
  goes to the item's doc and the order write to Home, two docs for one
  gesture. Needs the linked undo cross-doc move already uses. Until
  then a drop on another day reschedules only and the row lands at the
  end of that day's untimed band (today's `dropOnDay`).
- A drop onto band 1 / band 2 territory of the same day is clamped to
  the band's ends.

## Export, fingerprint, sharing

- **Fingerprint**: `day_order` is logical state (it fixes what a day
  shows), so `doc_fingerprint` hashes every non-empty day's list,
  as it hashes `focus`.
- **Export / import**: not exported, as foreign focus refs are not. It
  is a personal lens over items, not item data.
- **Leaving a workspace**: its refs go stale (unresolvable) and are
  never shown; no pruning needed.

## Pitfalls

1. **Two sorts, one rule.** The web and the engine sort separately
   today; a mismatch shows as rows that order differently on the phone.
   Shared test vectors (the same items and entries asserted in
   `dayGroups.test.ts` and `agenda.rs` tests) are the guard.
2. **Concurrent whole-order writes.** Two devices reordering the same
   day offline converge (MovableList), but to an interleaving neither
   wrote. Acceptable for a personal lens; no extra machinery.
3. **Home must be loaded** for the order to show. Every host holds Home
   while it shows the calendar, so this is a constraint on new hosts
   only.
4. **A shared workspace's reschedule moves your row.** If a teammate
   moves a task you ordered, it leaves your order (stale ref) and shows
   at the end of its new day. Correct, but visible.
5. **No ordering in Overdue.** A day's careful order is lost while a
   task is overdue and comes back only if it is rescheduled to that day.

## Phases

1. Core: `day_order` map, `day_order_entries`, `set_day_order`,
   `retarget_focus` extension, fingerprint, sweep. Wasm bindings. Merge
   tests (two peers order the same day offline; stale refs; first-wins
   dedup across both ref forms; key never deleted).
2. Engine: `build_agenda_from` reads the order; `api` / FFI
   `set_day_order` for the native apps. CLI `agenda` output follows the
   order (no CLI reorder verb).
3. Web: `groupByDay` projection, drag within a day, ⌘↑ / ⌘↓, through
   `HomeApp` only (shared `DocApp`s have no day order, as they have no
   Focus).
4. Drop on another day at a position (two-doc, linked undo).
5. iOS and macOS gestures.
