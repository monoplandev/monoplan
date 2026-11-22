# macOS drag and drop

Plan (decided 2026-11-15). Bring the web's drag and drop to the native
macOS window (`apple/MonoplanMac`, `spec/swift-bindings-plan.md` "The
macOS window"): reorder inside a list, Starred and the sidebar's lists;
drag items onto a list, Starred or the Bin in the sidebar. No board
(the Mac window has none yet), no lifecycle lane drops.

Status: phase 1 built 2026-11-15. Phase 2 attempted 2026-11-15 and
**abandoned**: the Swift code was reverted, the window is back on
SwiftUI lists. What was learnt is in "Phase 2 attempt" below; a v2
spec for the macOS side is to be written before another attempt.
Phase 3's list reorder built 2026-11-22 (below); its item drops and
phase 4 not built. The AppKit mapping below is the original plan,
not a design proven in the app.

**v2 baseline (spike, 2026-11-20):** `apple/MonoplanDnd/`, a port of
the web component itself rather than a mapping onto `NSTableView`:
frame-positioned rows in a scroll view, our own nudge, placeholder,
drag stack and autoscroll, no `NSDraggingSession`. A Foundation-only
core (selection, windowing, the drag state machine) under an AppKit
view, with a harness window and scripted drags that dump frames
(the "way to see it" asked for below). Contract and web → AppKit
mapping in `apple/MonoplanDnd/spec.md`.

**Phase 2 built on it (2026-11-20):** a list's rows and Starred's are
`ItemList.swift` over `DndListView`: multi-select, drag to reorder and
⌘↑ / ⌘↓ through `reorder_items` / `reorder_starred` (the model reorders
its own rows first, so the list never shows the old order while the
write is out), right-click on several rows to bin them in one step
(`set_binned_many`). One row selected opens its panel; none or several
shut it. An Inbox neither drags nor reorders. Rows are AppKit with the
checkbox and star hosted from SwiftUI. Not built: item drops on the
sidebar (phase 3, the list's delegate already reports the drag for it), across
workspaces (4); the other lenses are still SwiftUI lists.

**Sidebar list reorder (2026-11-22):** the half of phase 3 that moves
lists. Each workspace's active lists are one `DndListView` block
(`SidebarLists.swift`) set into the SwiftUI sidebar `List` under its
Inbox, as tall as its rows, with no row insets and 6 pt wider than the
cell either side, which puts its rows and selection where the system's
are (measured off-screen against a sidebar `List`: same icon column,
title, count and selection rect to the pixel). Drag one list, or
⌘↑ / ⌘↓, through `move_lists`; the model reorders `sections` first.
↑ / ↓ move between a workspace's lists; right-click is Delete. One list
at a time (`multi` off). The block's selection is its own (the list
shown when it is one of these; the `List` then has no selected row),
in the web nav's grey (`--bg-active`, `PaneColor.rowSelectedInactive`)
whether or not it has the keyboard, text unchanged on it (2026-11-22;
first built in the system's accent blue). The block does not scroll (`DndListView.scrolls`
off): the wheel goes to the sidebar. Inbox, the lenses and the
workspace headings stay SwiftUI rows. Not built: item drops onto
sidebar rows (the other half of phase 3), dragging a list into another
workspace, autoscroll of the sidebar during a drag, arrows across the
block's edges into the SwiftUI rows.

**New-item row (2026-11-20):** a list ends in a row to type the next
item into, the web's trailing capture slot (`ItemAddRow` in the list's
trailer, `DndListView.setTrailer`). Return adds at the end of the list
(`add_item`, one undo step) and keeps the keyboard; Return on nothing
and Escape hand it back to the list; ↓ on the last row enters the row
and ↑ in it, empty, leaves for the last row; text left in it when the
keyboard or the list goes elsewhere is added to the list it was typed
under. It stands in for "Nothing here" on an empty list. Not on
Starred (the web's capture-and-star needs one typed call for one undo
step), not on an Inbox shown newest first (its end is the top). Not
built: the web's draft row mid-list (N under a selection, needs an
`add_item_at` call) and the date phrase suggestion on a new title.

**The row on the item panel (2026-11-20):** the web's draft mirror
(`TaskDialog.tsx` `NewItemTarget.draft`). The panel is always shown
(decided 2026-11-20: it no longer opens on a selection and folds away
without one; empty with nothing selected, Escape empties it). The row
taking the keyboard puts the item it will be on the panel
(`ItemCapture` in `MainWindowModel`): the title follows the row as it
is typed, and the list, date, times, event flag and place picked there
are held and go on the item as it is made, in one call and so one undo
step (`api::items::capture`, `Engine.capture_item`). The capture is an item
as soon as its title is left with text in it:

- Return in the row makes it and starts the next capture, fields clean.
- A press in the panel that takes the keyboard from a named row makes
  it and puts the panel on it, its views kept as they were (the field
  the press landed in carries on, now writing to the item).
- The same press from an empty row leaves the capture on the panel,
  where its title is typed; Return or leaving that title makes the
  item, and the panel is its page.
- Escape, or the keyboard leaving an empty row for anywhere else,
  drops the capture, and the panel goes back to the item it showed,
  if it showed one.

A capture shows no subtasks, notes or history and has no status, star
or "…" in the bar; the list picker re-targets it, across workspaces
too, with no audience confirm (nothing exists to move). The row holds
its text until the item is in the rows wherever it went (the model's
`captureSettled`). Not built: a capture opened from the bar's
New Item button (still a stub), done / state / star on a capture.

## What the web does

The web carries its own component (`js/web/src/dnd/`, contract in
`dnd/spec.md`): virtualised fixed-height rows, block selection,
placeholder + nudge, an overlay stack of the dragged rows with a count,
autoscroll, touch. It emits `move { keys, beforeKey }` for a reorder and
document-level `primavera-dnd-dragmove` / `dragend` events for drops
outside the list.

- **Reorder in a list** (`Workspace.tsx` `onReorder`):
  `planReorderMoves(openIds, keys, beforeKey)` (`js/web/src/reorder.ts`)
  plans at most one `moveItem` per dragged row, all inside one
  `withActionBatch` (one undo step). Indices address the open projection.
- **Reorder in Starred**: the same planner over Home's full Focus entry
  list (hidden foreign / unresolved entries included, `spec/focus.md`),
  each step a `moveInFocus`.
- **Reorder sidebar lists** (`nav.tsx` `onReorder`): `moveList` per
  displaced list over the active (unarchived) lists. Inbox sits outside
  the draggable set and is never moved.
- **Drop onto a sidebar row** (`Workspace.tsx` `findDropTarget` /
  `onDndDragEnd`): rows carry `data-drop-list-id` / `data-drop-bin` /
  `data-drop-focus`; the dragged ids are taken in the source view's
  visible order.
  - *List*: the rows land as the target's **first** items, in visible
    order (`relocateTo`). Binned rows are restored; Done rows stay Done.
    Rows of another workspace cross with `moveToDoc` (ids kept), behind
    a confirm when the audience changes (`confirmingCrossDoc`). Selection
    clears unless the target is the list on screen.
  - *Bin*: `setBinnedMany` over the not-yet-binned rows.
  - *Starred*: `addToFocusMany`: each Open, not-yet-starred row
    prepended so the batch sits on top in visible order; items keep
    their list.

## AppKit mapping

No custom component. `NSTableView` / `NSOutlineView` give the whole
web component with public API (CLAUDE.md "No private APIs"):

| Web | AppKit |
|---|---|
| Drag the selection after a 3px slop | `tableView(_:pasteboardWriterForRow:)`, one writer per selected row; AppKit drags every selected row |
| Overlay stack + count | `NSDraggingSession.draggingFormation = .stack`; the count badge is the system's |
| Placeholder + nudge | `draggingDestinationFeedbackStyle = .gap` (the attempt's sticking point, below) |
| Between rows vs onto a row; end slot | `validateDrop` / `acceptDrop` with `.above` vs `.on`, retargeted via `setDropRow(_:dropOperation:)` |
| Autoscroll | built in |
| Escape cancels | built in |
| Drop onto Inbox / list / Starred / Bin | sidebar as a source-list `NSOutlineView`; `.on` drops for items, `.above` drops for list reorder |
| `⌘↑` / `⌘↓` move selection | `keyDown` on the table |

One private pasteboard type, `app.monoplan.item-ids` (and
`app.monoplan.list-ids` for the sidebar's own drag), carries the ids
with their workspace; a target accepts by type, which replaces the
web's `isItemDrag` sniffing. Rows stay SwiftUI views hosted in
`NSHostingView` cells, so the existing row (tick, star, context menu)
carries over.

SwiftUI `List` + `.onMove` / `.dropDestination` is rejected: the
package targets macOS 14 (multi-item drag APIs are macOS 26), `.draggable`
drags one item not the selection, `.onMove` and on-row drops fight in a
`List`, and the item list's row identity is positional
(`ForEach(... id: \.offset)`, kept deliberately for list switching),
which breaks move animation.

## Engine: the FFI surface (phase 1)

**One typed call is already one undo step.** `HostCmd::Call` brackets
the closure with `History::before` / `record` across every open doc
(`cli/src/daemon.rs` `on_call`, `cli/src/undo.rs`), so a call that
writes several times, in one doc or several, undoes as one action. No
batch / transaction API is needed: each multi-row gesture is one
multi-id call. Each is a plain function in `cli/src/api/moves.rs`
(testable over bare docs) and one `Engine` method in `core/ffi`.

The rules live in Rust, not Swift, so iOS gets them when it adds
reordering (memory: put cross-doc rules in Rust above `Doc`).

| `Engine` method | api fn | Does |
|---|---|---|
| `reorder_items(workspace, ids, before)` | `reorder_items` | Reorder within the ids' own list: the ids become one block, in list order, directly above `before` (nil: at the end). Indices over the open projection, planned by `plan_reorder` (the port of `planReorderMoves`). |
| `move_items(workspace, ids, list, index)` | `move_items` | Land `ids` (caller's visible order) at the 0-based open `index` of `list`, contiguous; binned ones restored first. Done stays Done. The sidebar drop passes `index = 0`. Same workspace only. |
| `reorder_starred(ids, before)` | `reorder_starred` | Starred reorder over Home's full entry list, `before` a starred id or nil (end). |
| `move_lists(workspace, ids, before)` | `move_lists` | Sidebar list reorder over active lists; `before` nil appends. Reserved lists refused by the core. |
| `set_binned_many(items, binned)` | `set_binned_many` | Bin / restore `ItemRef`s from any workspace; ones already in the wanted state skipped. |
| `star_items(items)` | `star_items` | Star `ItemRef`s from any workspace on top of Starred in the given order; already starred or not Open skipped (`addToFocusMany`). |

`ItemRef { workspace: Option<String>, id }` is the cross-workspace row
handle: the lenses (Starred, Calendar, Bin) mix workspaces, and a drop
from them onto the Bin or Starred must still be one step. `workspace`
is the call's selector grammar (doc id, name, nil = Home), resolved
against `Ctx::workspaces`.

`before` rather than an index everywhere a reorder happens: it is what
a drop between two rows names, and it stays right when the client's
rows are a filtered view of the order (Starred's hidden entries). A
`before` that is itself being dragged resolves to the first undragged
row after it. The existing single-row `reorder_item` / `move_item` /
`move_starred` stay for the panel and menus.

Cross-workspace relocation (`move_items_across_docs`) is phase 4.

## Phases

1. **FFI** — `cli/src/api/moves.rs` + the six `Engine` methods +
   `ItemRef`. Done when: Rust tests cover the planner (port of
   `js/web/test/reorder.test.ts`, exhaustive convergence included) and
   each call over bare docs (block order, restore-on-move, skip rules,
   one undo step per call through `History`); `cargo build -p
   monoplan-ffi` passes. Swift bindings regenerate in phase 2 with the
   first consumer.
2. **Item list as `NSTableView`** — replace the SwiftUI item list and
   Starred list in `MainWindow.swift` with an AppKit table (hosted
   SwiftUI cells, same selection → panel contract), multi-select,
   `.gap` reorder via `reorder_items` / `reorder_starred`, `⌘↑/⌘↓`.
   Attempted and abandoned 2026-11-15 (below).
   Done when: reorder of one and several (discontiguous) rows works in
   a list and in Starred, one Undo reverts it, list switching is no
   slower than today.
3. **Sidebar as `NSOutlineView`** — workspaces as groups, Inbox and
   lists as rows, lenses on top. Item drops `.on` Inbox / a list
   (`move_items` index 0) / Starred (`star_items`) / Bin
   (`set_binned_many`), with the hover highlight; list reorder
   `.above` via `move_lists`. Selection clears unless dropped on the
   shown list. Done when: every web sidebar drop works from a list and
   from Starred, refusals (another workspace's list until phase 4) show
   no highlight.
4. **Across workspaces** — `move_items` accepts `ItemRef`s from other
   workspaces via `move_items_across_docs` (ids kept, Focus retargeted,
   one entry in `History` spanning both docs) behind a confirm sheet
   when the audience changes. Done when: a drop onto another
   workspace's list moves, undoes as one step, and asks first.

## Phase 2 attempt (2026-11-15, abandoned)

`ItemTable.swift` replaced the list and Starred with an `NSTableView`
(hosted SwiftUI cells, multi-select, `.gap` drag, ⌘↑ / ⌘↓ through
`reorder_items` / `reorder_starred`). It ran, with too many
interaction defects to converge by iterating without seeing it run;
the Swift was reverted. Kept from it:

**Hosted cells are fast enough, updated in place.** Switching between
two lists, off-screen window at the app's size, to laid out and drawn,
median per switch (macOS 26, release build):

| | 50 rows | 300 | 1,000 |
|---|---|---|---|
| SwiftUI `List` (today's, positional ids) | 6 ms | 8 ms | 9 ms |
| table, `reloadData`, hosted cells | 42 ms | 44 ms | 47 ms |
| table, `reloadData`, plain `NSTableCellView` | 53 ms | 58 ms | 62 ms |
| table, in place, hosted cells | 4.3 ms | 4.2 ms | 4.5 ms |
| table, in place, plain cells | 3.0 ms | 3.5 ms | 3.9 ms |

The row count barely matters (the table makes only the ~18 visible
cells). `reloadData` builds every visible cell again rather than
reusing them, and on macOS 26 even an AppKit checkbox measures itself
through SwiftUI, so plain cells are no cheaper to build. In place
means by position, as the SwiftUI list's `id: \.offset`: rewrite the
cells the table holds, insert or remove rows only at the end, keep the
selection by id. This part worked.

**What went wrong, as reported from the app:**

- Rows moved by `moveRow` animate at AppKit's default pace, far too
  slow; wrapping in `NSAnimationContext` with a short duration was
  tried, effect unconfirmed.
- Transparent, layer-backed rows flickered and blended as they slid;
  their dividers flickered. Opaque rows, redraw-on-demand
  (`layerContentsRedrawPolicy`), no divider at a selection and raising
  the moved rows (`zPosition`) were tried; a flicker remained.
- The drag image sat a row above the pointer from the drag's start,
  and the row right above the dragged one did not nudge when dragging
  up. View and layer geometry were probed equal (no SwiftUI offset).
  The likely cause, not confirmed: AppKit validates a drop at the
  drag's start, over the source row, and `.gap` opens a gap there,
  sliding the source row down under a still image. Refusing slots
  where nothing would move was the last change, untested in the app.
- Hiding the dragged rows for the drag (`hideRows`) left a gap under
  the dropped rows and fixed nothing: dropped.
- A hosted cell's drag image needs its own `draggingImageComponents`
  (the default builds it from `textField` / `imageView`), drawn on an
  opaque background; `.stack` formation jumps the images to the
  pointer.

**For v2:** decide the drag feedback first (AppKit's `.gap`, its
`.regular` insertion line, or our own nudge and slot over a table with
feedback off): the attempt's failures were all there. And build a way
to see it before the feature: a harness window with fake rows whose
frames can be captured during a scripted drag.

## Open questions

- Drag out to other apps (plain text of the titles, like `itemClip.ts`)
  — cheap to add as a second pasteboard type; not required.
