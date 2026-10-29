# @primavera-ui/dnd

Optimistic, virtual drag & drop list with multiple select. Statically sized items only.

## Props for dnd-list<T: Any>
Name | Type | Default
|---|---|
| drag-type | ‘native’ \| ‘overlay’ | ‘overlay’
| source | DndSource | -
| overscan | number | 2
| rounded-select | boolean | true
| autofocus | boolean | true
| item-height | number | 32
| confine-autoscroll | boolean | false
| autoscroll-buffer | number | 32
| autoscroll-height | number | 32
| drag-stack-count | number | 3
| expandable | boolean | false
| multi | boolean | true
| clear-on-blank-click | boolean | false
| clear-on-click-outside | boolean | false
| reorder | boolean | true
| trailer | element | -
| trailer-height | number | item-height
| on-trailer-navigate | () => void | -

## Web Component
```
<dnd
	drag-type="native"
	source: DndSource
	context: DragContext
	rounded-select: true
/>
```

## DndSource
```typescript
type Key = string | number;

type DndOp =
  | { type: "move"; keys: Key[]; beforeKey: Key | null }
  | { type: "insert"; items: T[]; beforeKey: Key | null }
  | { type: "remove"; keys: Key[] }
  | { type: "update"; key: Key; patch: any }
  | { type: "reset"; keys: Key[] };

interface DndSourceArgs {
	getKey,
	getOrder,
	getItem,
}

class DndSource<T> {
  getKey(item: T): Key;
  getOrder(): readonly Key[];
  getItem(key: Key): T
	constructor(props: DndSourceArgs) {
		this.getKey = props.getKey;
		this.getOrder = props.getOrder;
		this.getItem = props.getItem;
	}

	 // internal 2-phase commit (implementation example)
  _commitUI(txn) // optimistic ui update
  _commitState(txnId) // Writes changes on move/delete

	// Host can call this so list can apply external updates
	apply(ops: DndOp[]): TxnId
	// Host can call this to see updates produced by list
	onChange(cb: (op: DndOp, txnId: TxnId) => void): () => void //
}

// E.g.
type Key = string;
type Item = {
	key: Key,
};
const data = [];
const keyIndex = new Map<Key, Item>();

const source = new DndSource({
	getKey: (item: T) => item.key,
	getOrder: () => data.map((item) => item.key),
	getItem(key: Key) => keyIndex.get(key);
});

interface DndRenderer<T> {
  mount(key: Key, item: T, container: HTMLElement): () => void; // renders *inside* container, does not replace it
  getNativeDropData?(keys: Key[], items: T[]): Array<{ type: string; data: string }>;
}

interface DragContext {
	register(renderer, source) {}
}
```

## DOM Layout
```
<div class="consumer">
	<div class="parent">
		<div role="listbox" aria-multiselectable="{multi}">
			...<div role="option" />
		</div>
		<div class="placeholder" />
	</div>
</div>
```

## Data model
1. `DndSource` is a class that has read only access to array of all list items. It takes three key access functions provided by the consumer. `getKey()`, `getItem()` and `getOrder()`. `getOrder()` is called once by `DndSource()` to create an internally cached order which is then kept in sync.
2. `DndSource` produces mutation intent & receives external updates. A two-phase commit allow optimistic ui changes. Transactions are created on local mutation actions & `DndSource._commitUI` caches these to preview state (pulled via getOrder()). Given a transaction, commitUI must occur successfully before commitState. `DndSource._commitState()` commits changes back to the host to process. The host receives external updates via apply() called by the host. The host receives updates from the producer via registering a callback to `onChange()`.
3. `DndSource.getOrder` produces an ordered index of item `key`s. This is what the `listitems`s will be rendered against, using `DndSource.getItem(key)`. This should be a direct diff against the currently displayed items, so it is necessary to maintain a `Map<key, listitem>` map within the component state.
5. DragContext is provided to `<dnd>` (or created per dnd automatically if not provided). DragContext allows the consumer to share drag contexts between DndRenderers. Items can thus be dragged from one list to another.
6. TODO: Undo/redo interface

## Markup/Sizing
1. Regular list structure comprised of `parent` container `<div class="parent">` , the `<div role="listbox">` list and `<div role="option">` listitems.
2. A drop placeholder div sits directly behind parent div, anchored to same position (position: absolute, left:0, right:0, z-index: 1). The scroll container must have a higher z-index (e.g. 2) so its scrollbar renders above the placeholder. The placeholder is moved with `transform: translateY(...)` and faded with an `opacity` CSS transition so position updates and show/hide stay GPU-composited and reflow-free.
3. The `parent` has `overflow-y: scroll` and is sized by the component consumer. Scroll position can be modified natively.
4. `listitem` are a fixed size and absolutely positioned, determined by their `top` px value. There is a `transition` on `top` of 0.15s to support smooth movement as surrounding list items are added, moved and removed.
5. The height of `list` is defined by `max(qty(visibleItems) * height(listitem), height(parent))`. The `max` with parent height ensures the list always fills the scroll container, even when most items are hidden (e.g. during drag with many selected items).
6. Only items within view + overscan either side will be rendered. The top-most item index `startIndex = max(floor(scrollTop / H) - overscan, 0)` where `overscan` is a fixed value (usually 2). The end index is defined by `endIndex = min(startIndex + ceil(viewportHeight / H) + overscan*2, count)`. A range is held in state that updates as `scrollYOffset(parent)` changes, e.g. due to scrolling. As the range changes, new items in the range render, and items outside the range drop off. There is the option here for a pool of hot rendered items. A `ResizeObserver` on `parent` recomputes the range when its height changes — required for parents whose height is dynamic or only resolves after layout (without it the initial range is stuck at whatever `clientHeight` was at mount, often 0).
7. One or more items can be selected. A set of item keys will be used to track selected items. And a Key[] to maintain order. `aria-selected: true` will be applied on these elements if rendered. Select is probably best managed with its own controller per list.
8. Selected items get `z-index: 1` so they paint above unselected siblings during top transitions and where selection chrome (background, rounded corners) overlaps neighbours.

## Selection

### Selection interactions
1. List focus: The entire list a tab-stoppable. Tab/Shift+Tab will focus out of the list. Only one list can be focused at a time. Mouse click on list automatically focuses on list and applies relevant selection action.
2. Offset: When an out-of-view item is selected, the scroll position should quickly, but smoothly shift to that item position.

### Selection model

1. A block is a group of contiguous items; this could be a single item or a discrete range.
2. Selections comprise one or more non-contiguous blocks. Therefore, blocks must merge implicitly when 2 blocks are adjacent or overlapping.
3. The stable end of a block is the `anchor`, while the fluid end is `to`.
4. `Anchor` may be always explicitly defined, or sometimes inferred from context. Implementation is flexible.
5. `Adjacent` refers to previously unselected block next to another block.

#### Relative identifiers
- first: the first item in the list (top of the full list)
- last: the last item in the list (bottom of the full list)
- next(item): item below, or item itself if at end of list
- prev(item): item above, or item itself if at top of list
- top(block): the upper boundary item of a block
- bottom(block): the lower boundary item of a block
- topmost: the block with the highest position in the list, i.e. blocks[0]
- bottommost: the block with the lowest position in the list, i.e. blocks[last]
- top(selection): shorthand for top(topmost)
- bottom(selection): shorthand for bottom(bottommost)
- to(block): the fluid end of a block (the end that moves on extend)


#### Implicit states

For explanatory purposes, there are 4 commonly ocurring implicit states:

- Idle: No items selected
- Single: One block consisting of one item selected.
- Range: One block selected.
- Multiple: Two or more blocks selected. Active is the block currently targeted by shift/arrow operations.

#### Types
```
type Block = {
  anchor: Key,
  to: Key,       // anchor == to for single item; defines fluid end of block
}

type Selection = {
  blocks: Block[],  // ordered by position in list; never overlapping
  active: Block | null,
}
```

#### Behaviour

```
selectOnly(item)
  blocks = [{anchor: item, to: item}]
  active = blocks[0]

addBlock(item)
  b = {anchor: item, to: item}
  blocks += b
  active = b
  → merge

extendActive(item)
  if anchor != to and item is on the opposite side of anchor from to:
    active.anchor = active.to    // pivot so the block grows rather than flips
  active.to = item
  → merge
  Invariant: shift+select outside the current range is always additive —
  items already in the active block are never dropped. The block contracts
  only when item is between anchor and the current to (inside the block).

toggleItem(item)
  if item ∈ active:
    remove item from active (shrink, or split if interior)
    active = nearest remaining block
  else:
    addBlock(item)
  → merge

moveSelection(dir)
  shift all blocks up/down by 1 position, clamped to list bounds
  preserve active

merge:
  sort blocks by top(block)
  collapse any adjacent or overlapping blocks into one
  if active was consumed by merge, active = merged block
```

#### Performance optimisations
It may be worth keeping an index to describe blocks e.g. itemToBlock: Map<Key, Block> to ensure O(1) operations esp re. merge.

### Selection interaction bindings

TODO: macOS vs WINDOWS/Linux, not macOS vs Linux

| macOS | Linux | Condition | Operation |
|---|---|---|---|
| `click` | `click` | any | `selectOnly(item)` |
| `⌥+↑` | `⌥+↑` | any | `selectOnly(first)` |
| `⌥+↓` | `⌥+↓` | any | `selectOnly(last)` |
| `↓` | `↓` | nothing selected | `selectOnly(top)` |
| `↑` | `↑` | nothing selected | `selectOnly(bottom)` |
| `↓` | `↓` | selected | `selectOnly(next(selBottom))` |
| `↑` | `↑` | selected | `selectOnly(prev(selTop))` |
| `shift+click` | `shift+click` | any | `extendActive(item)` |
| `shift+↓` | `shift+↓` | any | `extendActive(next(to(active)))` |
| `shift+↑` | `shift+↑` | any | `extendActive(prev(to(active)))` |
| `shift+⌘+↓` | `shift+ctrl+↓` | any | `extendActive(last)` |
| `shift+⌘+↑` | `shift+ctrl+↑` | any | `extendActive(first)` |
| `⌘+click` | `ctrl+click` | item ∉ selection | `addBlock(item)` |
| `⌘+click` | `ctrl+click` | item ∈ selection | `toggleItem(item)` |
| `⌘+↓` | `ctrl+↓` | any | `moveSelection(down)` |
| `⌘+↑` | `ctrl+↑` | any | `moveSelection(up)` |
| `⌘+a` | `ctrl+a` | any | `selectAll()` — single block from first to last |
| `escape` | `escape` | any | `blocks = []; active = null` |

When `multi` is `false`, the listbox is single-select: every multi-select gesture in the table above collapses to a single-selection equivalent. Specifically, `shift+click`, `shift+↓/↑`, `shift+⌘+↓/↑`, and `⌘/ctrl+click` all behave as `selectOnly(target)`; `⌘/ctrl+a` is a no-op. `aria-multiselectable` reflects this (`"true"` by default, `"false"` when `multi=false`).

Every keyboard action that changes the selection or moves items must scroll the leading edge into view (e.g. top of selection when moving up, bottom when moving down).

Custom bindings by consumer should be accounted for in kb interface.

## Drag mode
1. On click: if the item is unselected, the selection set will include only that item. If the item is selected, nothing will happen. If the user then starts dragging, `drag` mode starts. A slight buffer of 3px, easily calculated with Pythagoras should be required before drag begins.
2. In `drag` mode, a preview order is generated with selected items filtered out. Selected elements can be held in a cache, to avoid a re-render when used in drag overlay mode or when put back in the list.
3. In `drag` mode, if `nudge` is enabled, remaining (non-dragged) items visually shift to make room at the hover position. See "Nudge" section below. If the pointer leaves the parent container, items render without any nudge offset.
4. If `drag-type=‘native’`, the `listitem` component will have draggable attribute set to true. On HTML native `drag` event, it will call `event.dataTransfer.setData` with the properties of getNativeDropData().
5. If `drag-type=‘overlay'`, the `<li>` component will have draggable attribute set to false. An overlay div with `position absolute; top: 0, left: 0, width: 100vw, height 100vh` will take up the entire page, it may have to be added to the `<body>` and be tracked so it can be taken down when `drag` mode ends. A copy or reference of the first `drag-stack-count` item elements are taken and presented as a stack of items, each [2px, 2px] further from and a lower depth than the previous, with a number signifying the total count of grabbed items. Stack positions are updated via a `requestAnimationFrame` loop using linear interpolation (lerp) toward the cursor — the lead item has a high lerp factor (~0.8, nearly instant), trailing items slightly lower (~0.6, decreasing ~0.03 per depth). This produces a subtle trailing effect without explicit delay timers.
6. Drag mode is ended when user releases the mouse or when the user hits escape. If the user was hovering over an item and thus an index is tracked, that item will provide the “beforeKey” in the move op.
7. Edge case: to allow dropping at the very end of the list, during drag the listbox is extended by exactly one `height(listitem)` when a hover is active (the nudge slot). When the pointer is south of `qty(listitem)` but still within `list`’s bounds, `hoverIndex = qty(listitem)`.
8. The autoscroll controller tracks the user’s drag position. If the drag position is above `parent top offset + autoscroll-buffer`, the list will automatically scroll up, starting at 20% of max speed and linearly accelerating to max over 500ms. If the drag position is below `parent bottom offset - autoscroll-buffer`, the list will scroll down in the same way. If `confine-autoscroll` is enabled, autoscroll will only occur when drag is within the parent’s y bounds and will stop when moved above/below the parent. Autoscroll stops when user leaves the x-bounds of the parent, the autoscroll region, or stops dragging.
9. The placeholder div is moved to the hover space via `transform: translateY(...)`. The calculated position is `hoverIndex * height(listitem) - scrollTop`. When the drag is outside the parent the placeholder is faded out (opacity:0). When the user is at the bottom of the list, the placeholder y position must be at `items.length * height(listitem)`.
10. `rounded-select: true` will generate css border-radius within a style block inside the component, around contiguous selection groups, using sibling/next/previous selectors to do so, such that each contiguous selection appears as block with rounded edges. Top & bottom of virtualised window are always at the border even if there's an adjacent selected item, so may be render those edges incorrectly - this is ok because they're out of view.

## Drag overlay

When `drag-type='overlay'`, a full-page overlay captures pointer events and displays a visual stack of the dragged items.

### Markup

```
<body>
  ...
  <!-- appended to body, removed on drag end -->
  <div style="position:fixed; inset:0; z-index:9999; pointer-events:none">

    <!-- one per drag-stack-count, each wrapping a cloned role="option" -->
    <div class="dnd-stack-item" style="position:absolute; width:{listWidth}px; height:{itemHeight}px; overflow:hidden; background:{resolvedBg}">
      <div role="option" ...><!-- cloned content --></div>
    </div>

    <!-- count badge, only when dragging 2+ items -->
    <div class="dnd-badge" style="position:absolute">3</div>

  </div>
</body>
```

### Style notes

- Each stack item wrapper provides explicit `width` (from list) and `height` (from `itemHeight`) since cloned elements lose their layout context.
- `overflow:hidden` clips the clone to the wrapper bounds.
- Background is resolved from the source element's computed style at drag start (CSS vars don't cascade into the body-level overlay).
- Stack items offset by `[2px, 2px]` per depth with decreasing opacity.
- `box-shadow` via `var(--dnd-drag-shadow)`.
- Count badge is a direct child of the overlay (not a stack item) to avoid being clipped. It tracks the first stack item's position in the animation loop, anchored at top-right.
- Grab offset: the stack anchors relative to the element that initiated the drag (not the first selected item). Compute `grabOffset = elementRect.topLeft - cursorPosition` at drag start and apply to all position updates.

### Foreign drop zones (overlay mode)

Bubbling `CustomEvent`s `primavera-dnd-dragstart` / `-dragmove` / `-dragend` fire on `<primavera-dnd>` with `detail: { keys, items, x, y }`; foreign drop zones hit-test their own bounds and call `preventDefault()` on `dragend` to consume the drag and suppress the internal reorder.

## Locked order (`reorder=false`)

When `reorder` is `false`, this list refuses to reorder itself. Items are still draggable — selection, drag-start, the overlay stack, and the `primavera-dnd-drag*` CustomEvents all fire as normal so foreign drop zones can still consume the drag — but within this list there is no nudge, no placeholder, and a same-list drop is a no-op (items snap back). Keyboard reorder (`⌘/ctrl+↑/↓`) is also disabled. Use this when a list is a drag *source* whose internal order is fixed (e.g. a palette).

## Expansion
When `expandable` is set, double-clicking an item toggles a single-item expanded state. Only one item is expanded at a time. The renderer is invoked once per item with an `expanded` prop — the same component instance just re-renders when its expansion flag flips, so the consumer can branch on `expanded()` to render an extra body. Expand/collapse is mutually exclusive with selection from a UX standpoint; the two states should not be conflated.

### Behaviour
- Pressing `Escape` or clicking anywhere outside the expanded item collapses. Double-clicking the expanded item itself is a no-op.
- Double-clicking a different item collapses the current and expands the new one atomically.
- Starting a drag clears expansion synchronously (no animation) so drag-mode math stays uniform-height.
- Expanding sets selection to just the expanded item, and selection chrome is suppressed everywhere while anything is expanded — the expanded item is the visual focus and should not also wear select chrome. On collapse the chrome reappears with the previously-expanded item as the sole selected entry; clicking another item to collapse lets that click's selection win naturally.

### Height & layout
- The expanded item's height transitions over `0.15s ease`, matching the `top` transition used for surrounding items as they shift down.
- The expanded item's contents are pinned to the top of the container; as the container grows, content reveals top-down (the rest is clipped by `overflow:hidden` during the animation).
- The measured expanded height is fed into virtualization, pushing items below down by the difference (also via the `top` transition).

### Click-outside + dblclick recovery
A click anywhere outside the expanded item collapses it. When the user double-clicks an item *below* the expanded one, the first click triggers a click-outside collapse — items below shift up, and the second click of the intended dblclick lands on a different element.

To recover the user's intent, every `click` snapshots the prior click's target and timestamp. In `dblclick`, if the snapshot is within the dblclick window (500ms — comfortably above the 0.15s shift animation), it is used as the target instead of the `dblclick` event's own target (which is typically the common ancestor of the two diverged clicks, or the second-clicked sibling).

### Clearing selection by click
`clear-on-blank-click`: a click on the listbox that hits no row clears the selection. Clicks anywhere else leave it alone — the listbox just loses focus, and the host styles the selection as dormant via `:focus-within` (Finder's model: clicking another window doesn't deselect). Escape still clears. Use this for the main item list, where a side panel acts on the selection and must not dissolve it underfoot.

`clear-on-click-outside`: superset — a click anywhere outside the host element clears too. Use for content-height listboxes whose blank space lies outside the listbox (the nav), where a blank-click rule alone would leave no pointer way to deselect.

### Trailer slot
`trailer`: an element rendered in a fixed-height slot (`trailer-height`, default `item-height`) directly after the last row, inside the scrolling listbox so it follows the list rather than pinning to the viewport. The listbox's scroll extent grows to cover it (the fill-height floor still wins when the list is short). It is not an option: it has no key, takes no part in selection, drag or keyboard navigation, and a click on it is a blank click as far as the controller is concerned — a consumer that wants a click to do something stops propagation in its own handler so the blank-click clear and the document-level collapse check don't fire. The host sets `data-trailer` while a trailer is present so styling can keep the last row's divider. With `on-trailer-navigate` set, a plain ArrowDown whose selection bottom is already the last row (or on an empty list) calls it instead of stopping, handing the keyboard on to the trailer; shift/modifier arrows are unaffected. The app uses it for the list's trailing "new item" capture slot.

### Click-outside hit-testing
Both `clear-on-click-outside` and the expanded-item collapse check use cursor coordinates against the relevant element's bounding rect, not DOM containment. A portaled dismissable layer (e.g. a context menu) commonly sets `pointer-events:none` on `<body>` while open, which routes the click event to `<html>` — DOM containment then reports the click as outside even when the cursor was on top of us, spuriously clearing selection or collapsing.

## Touch devices (differences from mouse drag/select model)
1. Multi-select is currently not possible on touch devices.
2. A tap constitutes select one behaviour.
3. A drag with an initial hold of less than 150ms initiates a scroll.
4. A drag with a hold of more than 150ms & 3px drag buffer initiates an item drag.
5. `touchmove` must be tracked to determine hoverIndex & leaving parent.

## TODO (prior to release)
1. Mid-drag update prevention
2. DragContext thorough specification
3. Nudge behaviour should be optional (this needs to be optional to support trees!)
4. allow itemHeight changes! Real fix is upstream — either drop the cached fields and read the live getter, or add observedAttributes + propagate via setItemHeight(). Worth filing against primavera-ui if you control it.

## FUTURE (Out of scope)
1. tree extension specifications
TODO: Basically add children support, expand/collapse & flatten items for preview with depth attr for renderer to understand correctly. At the edge, use x plane to decide whether you are nesting an item or putting it below on the same level - probably needs some visual notifier.
2. Edit mode - allows for multiple selection & bulk actions on touch devices.
