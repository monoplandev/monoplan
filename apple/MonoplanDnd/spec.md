# MonoplanDnd

The web's drag-and-drop list (`js/web/src/dnd/`, contract in its
`spec.md` and `spec-diff.md`) on AppKit, with the same primitives.
Built 2026-11-20. The Mac app's list and Starred views are on it
(`apple/MonoplanMac/Sources/MonoplanMac/ItemList.swift`); `DndHarness`
is where to try a change first. Where this file is silent, the web spec
is the contract.

Why not `NSTableView`: `spec/mac-dnd-plan.md` "Phase 2 attempt". Every
failure there was in drag feedback the table owns (`.gap`, the drag
image, `moveRow` timing). Here nothing is AppKit's: no table, no
`NSDraggingSession`, no pasteboard.

## Layout

| Web | AppKit |
|---|---|
| `<primavera-dnd>` host, positioning context | `DndListView` (flipped `NSView`) |
| `.dnd-parent`, `overflow-y: scroll` | `NSScrollView`, no background |
| `role="listbox"`, height of the whole list | `DndListbox`, the flipped document view |
| `role="option"`, `position:absolute; top` | `DndRowView`, one per row in view, set by `frame`, pooled |
| `trailer` in `.dnd-trailer`, after the last row | `setTrailer(view, height:)`: the host's view in a `DndTrailerSlot` in the listbox |
| `on-trailer-navigate` | `DndListDelegate.dndListNavigatePastEnd -> Bool` |
| renderer `mount(key, item, container)` | `DndListDataSource`: make a content view, configure it for a key |
| placeholder: a layer of shades, sibling of the parent, `translateY`; crossfades on a slot change (old shade fades out in place, new fades in) | layer-backed shade `NSView`s behind the scroll view, in viewport coordinates, same crossfade (`placeholderSlot` in the render state) |
| `transition: top 0.15s ease` | additive keyframe animation on the row layer's `position.y`: 0.15 s, the CSS `ease` curve, in whole-pixel steps (below) |
| overlay `position:fixed; inset:0` on `<body>` | borderless, transparent, click-through child window over every screen; rows are `CALayer`s holding snapshots |
| `requestAnimationFrame` loops | one `CADisplayLink` (`NSView.displayLink`, macOS 14), live only while something moves |
| document `mousemove` / `mouseup` | `mouseDragged` / `mouseUp`, which AppKit delivers to the view that took `mouseDown` wherever the pointer goes |
| document capture `keydown` for Escape | `NSEvent.addLocalMonitorForEvents` for the length of the drag |
| `ResizeObserver` on the parent | `layout()` |
| `:focus-within` dormant selection | `DndRowState.emphasized`: first responder in the key window |
| `primavera-dnd-dragstart/move/end`, `preventDefault` to consume | `DndListDelegate` `dragDidBegin` / `dragDidMoveTo` / `dragWillEndAt -> Bool` / `dragDidEnd`, window coordinates |
| `DndSource` ops + 2-phase commit | `DndMove { keys, before }`: applied to the list's own order at once, handed to the delegate, confirmed by `setOrder` |

`DndMove` is `Engine.reorder_items(ids, before)`'s argument shape.

## Split

`MonoplanDndCore` has no AppKit: `DndSelection` (blocks, merge),
`DndVirtualization`, `DndAutoscroll`, the key map, and `DndListModel`,
which is the web's `DndController` as a state machine. The model takes
pointer positions in viewport coordinates (origin top left, y down, not
scrolled), key actions and the viewport's geometry, and returns
`renderState()`: the rows to mount with their `top`s, the list's height
and the placeholder's y. `DndListView` turns events into those calls and
lays out what comes back. Every rule below the pixels is tested without
a view (`swift test`).

The drag rules are the web's, unchanged:

- A plain press on an unselected row selects it alone at once; on a
  selected row it waits. Shift / ⌘ presses act on release and never
  drag. 3 px of travel starts the drag.
- At drag start the dragged rows leave the listbox (they are snapshotted
  first) and everything works in **collapsed space**: the order with
  the dragged keys removed. `hoverIndex = clamp(floor((y + scrollTop) /
  H), 0, collapsed.count)`; rows at or past it sit one row lower; the
  list is one row taller while a hover is live. One slot, however many
  rows are dragged.
- For a one-row drag this is the layout the user was already looking
  at, so nothing jumps at drag start (tested).
- The hover slot is recomputed on scroll as well as on pointer moves.
- Off the viewport: no hover, no nudge, no placeholder; released there
  with no target, the rows snap back.
- Drop: `before = collapsed[hoverIndex]`, nil past the last row.
  A drop that changes nothing reports no move.
- Autoscroll: within `autoscroll-buffer` of the top or bottom edge, or
  past it; 20% of 720 px/s rising to full over 0.5 s; stops when the
  pointer leaves sideways.
- The stack: the first `drag-stack-count` rows, 2 px lower and 10%
  fainter per depth, the lead closing 80% of the distance to the pointer
  per 60 Hz frame and each deeper one chasing the row above it; a count
  badge 12 px above the pointer for two or more rows. The stack keeps the
  pointer where it took hold of the grabbed row.

## Differences from the web

- Frame-rate independent: autoscroll is px per second and the lerps are
  `1 - (1 - f)^(dt·60)`, where the web moves a fixed amount per frame.
- No separate `click` event: button up without a drag is the click, so
  the web's `suppressNextClick` has no counterpart.
- `DndSelection.updateOrder` drops selected keys that left the order
  (the web resolves a missing key to index 0).
- `DndSelection` merge keeps the whole range when the active anchor ends
  up inside it. The web drops the rows below the anchor: select two
  runs with one row between, ⌘-click that row, and the lower run is
  gone (`js/web/src/dnd/core/selection.ts` `merge`, reproduced
  2026-11-20, not fixed there).
- After a move the moved rows are re-merged into one block.
- A press inside a row goes to the row unless it lands on an
  interactive view: `NSButton`, an editable or selectable
  `NSTextField`, `NSTextView`, or anything adopting
  `DndInteractiveView`.
- An order change mid-drag rebuilds the collapsed order and cancels the
  drag when none of its rows remain (the web's open TODO).
- Double click and the context menu go to the delegate; a right-click
  outside the selection selects that row first.
- Reduce Motion turns the row slide off.
- The row slide moves in whole device pixels (`dndSlideSteps`, 120
  steps a second). Each row is its own layer; at a fractional offset
  the compositor blends a layer's edge with what is behind it, so two
  rows sliding edge to edge showed a 1 px light gap between them, most
  visible across a run of selected rows (seen on video, 2026-11-20).
  A browser has no such seam: its rows are painted into one surface.
- `viewport.topInset`: the list scrolls under a toolbar, as the app's
  panes do. The rest position is `scrollTop = -topInset`; reveal,
  autoscroll's top zone and the fill-height floor allow for it.
- `drag = false` turns dragging off altogether (the Inbox).
- The trailer's presses are its view's own (the listbox does not take
  them, as it does a row's). One the view does not handle climbs the
  responder chain to the listbox and is a blank click, which is the
  web's rule with `stopPropagation` turned round. Its slide rule is the
  web's: it slides only on a pass where a row already on screen moved.
- `scrollToTrailer()`: the host calls it after putting the keyboard in
  the trailer, and as the list grows above it. Revealing the last row
  scrolls to the end, so the trailer comes with it.

## Not ported

Expansion, the native (`draggable`) drag type, touch,
foreign-drag preview (`setForeignHover`: a drag from another list
previewing an insertion here), `clear-on-click-outside`, accessibility
roles. Rows hosted in `NSHostingView` are untried: SwiftUI content takes
its own mouse events, so the press rule above needs a decision first.

## Harness

`swift run --package-path apple/MonoplanDnd DndHarness` opens 2,000 fake
rows beside a "Bin" drop target (a stand-in for a sidebar row), with a
field in the trailer that adds a row on Return.

`DndHarness --script <dir>` drives an off-screen window through one-row
and multi-row drags, autoscroll, Escape, a drop on the foreign target
the keyboard and the trailer, with time advanced by hand (`step(dt:)`). Each step
writes a PNG and a line of `frames.jsonl` (row frames, placeholder,
stack, scroll position) and the outcomes are checked. It sees layout,
not motion: the PNGs show where rows end up, not the slide between, and
the stack is painted on from its frames. How the animation, the overlay
window and real mouse input feel has only been checked by reading the
code: run the window.
