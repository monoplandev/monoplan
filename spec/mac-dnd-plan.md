# macOS drag and drop

Plan (decided 2026-11-15). Bring the web's drag and drop to the native
macOS window (`apple/MonoplanMac`, `spec/swift-bindings-plan.md` "The
macOS window"): reorder inside a list, Starred and the sidebar's lists;
drag items onto a list, Starred or the Bin in the sidebar. No board
(the Mac window has none yet), no lifecycle lane drops.

Status: phase 1 built 2026-11-15. Phases 2–4 not built.

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
| Placeholder + nudge | `draggingDestinationFeedbackStyle = .gap` |
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

## Open questions

- Hosted SwiftUI cells: does `NSHostingView` per cell keep list
  switching fast at a few hundred rows? Measure in phase 2 before
  committing; fallback is plain `NSTableCellView` rows.
- Drag out to other apps (plain text of the titles, like `itemClip.ts`)
  — cheap to add as a second pasteboard type in phase 2; not required.
