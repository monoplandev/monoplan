import CoreGraphics
import Foundation

public struct DndConfig {
    /// Every row's height. Rows are statically sized.
    public var itemHeight: CGFloat = 32
    /// Rows rendered beyond the viewport, each side.
    public var overscan = 2
    public var multi = true
    /// False: rows still drag (for drops elsewhere), but the list never
    /// reorders itself: no nudge, no placeholder, no ⌘↑ / ⌘↓.
    public var reorder = true
    /// False: rows never drag at all (a list with nothing to reorder
    /// and nowhere to drop).
    public var drag = true
    /// Remaining rows open a one-row gap at the hover slot during a drag.
    public var nudge = true
    /// False: plain arrows are not answered (a list with no visible
    /// selection, like the sidebar).
    public var arrowNavigate = true
    /// A click in the list that hits no row clears the selection.
    public var clearOnBlankClick = false
    public var roundedSelect = true
    /// The list is at least as tall as its viewport.
    public var fillHeight = true
    public var autoscrollBuffer: CGFloat = 32
    public var confineAutoscroll = false
    public var dragStackCount = 3
    /// Pointer travel before a press becomes a drag.
    public var dragSlop: CGFloat = 3
    /// Shaved off the bottom of the placeholder, for gapped rows.
    public var placeholderGap: CGFloat = 0

    public init() {}
}

/// The list's one mutation intent: `keys` (in list order) become a
/// contiguous block directly above `before`, or at the end when nil.
/// Shaped as `Engine.reorder_items(ids, before)`.
public struct DndMove: Equatable {
    public var keys: [String]
    public var before: String?

    public init(keys: [String], before: String?) {
        self.keys = keys
        self.before = before
    }

    public func applied(to order: [String]) -> [String] {
        let moving = Set(keys)
        var rest = order.filter { !moving.contains($0) }
        let present = order.filter { moving.contains($0) }
        let at = before.flatMap { rest.firstIndex(of: $0) } ?? rest.count
        rest.insert(contentsOf: present, at: at)
        return rest
    }
}

public struct DndRenderItem: Equatable {
    public var key: String
    /// Index in the current order: collapsed during a drag, full otherwise.
    public var index: Int
    /// Top in list coordinates, nudge applied.
    public var top: CGFloat
    public var height: CGFloat
    public var selected: Bool
    /// First / last row of a contiguous selected run (rounded corners).
    public var selFirst: Bool
    public var selLast: Bool
}

public struct DndRenderState: Equatable {
    public var isDragging: Bool
    /// Height of the scrolled content (the fill-height floor applied).
    public var listHeight: CGFloat
    /// Height of the rows alone, the nudge slot included while dragging.
    public var contentHeight: CGFloat
    public var items: [DndRenderItem]
    /// Placeholder top in viewport coordinates, nil when hidden.
    public var placeholderY: CGFloat?
}

public enum DndDragPhase: Equatable {
    case began
    case moved
}

/// The list's behaviour with no view attached: the port of the web's
/// `DndController` (`core/dnd-controller.ts`) as a state machine. The
/// view feeds it pointer and key input in viewport coordinates and the
/// viewport's geometry, then draws `renderState()`.
///
/// Not ported: expansion, the native drag type, touch, the trailer slot,
/// foreign-drag preview (a drag from another list hovering this one).
public final class DndListModel {
    public var config: DndConfig
    public let selection: DndSelection
    public private(set) var order: [String]
    /// Kept current by the view before any call that reads geometry.
    public var viewport = DndViewport()

    public struct Drag: Equatable {
        /// Dragged keys in list order.
        public var keys: [String]
        /// The row the pointer went down on.
        public var grabKey: String
        /// The order with the dragged keys removed. Hover, nudge and
        /// windowing all work in this collapsed space.
        public var collapsed: [String]
        /// Slot in `collapsed` (0...count), nil off the viewport.
        public var hoverIndex: Int?
        public var pointer: CGPoint
    }

    public private(set) var drag: Drag?

    private struct Press {
        var key: String?
        var point: CGPoint
        /// A plain press on a row: may become a drag.
        var armed: Bool
    }

    private var press: Press?

    public init(config: DndConfig = DndConfig(), order: [String] = []) {
        self.config = config
        self.order = order
        self.selection = DndSelection(order: order)
    }

    public var isDragging: Bool { drag != nil }

    private var virtualization: DndVirtualization {
        DndVirtualization(itemHeight: config.itemHeight, overscan: config.overscan)
    }

    // MARK: Order

    /// Take the host's order. During a drag the collapsed order is
    /// rebuilt; the drag is cancelled when none of its rows remain.
    public func setOrder(_ next: [String]) {
        guard next != order else { return }
        order = next
        selection.updateOrder(next)
        guard var d = drag else { return }
        let present = Set(next)
        d.keys = d.keys.filter { present.contains($0) }
        guard !d.keys.isEmpty else {
            cancelDrag()
            return
        }
        let dragged = Set(d.keys)
        d.collapsed = next.filter { !dragged.contains($0) }
        drag = d
        updateHover()
    }

    // MARK: Render

    public func renderState() -> DndRenderState {
        let h = config.itemHeight
        let virt = virtualization

        if let d = drag {
            let count = d.collapsed.count
            let nudging = config.reorder && config.nudge
            let hover = config.reorder ? d.hoverIndex : nil
            let contentHeight = virt.totalHeight(count: count + (nudging && hover != nil ? 1 : 0))
            let range = virt.range(scrollTop: viewport.scrollTop, viewportHeight: viewport.height, count: count)
            let items = range.map { i -> DndRenderItem in
                var top = virt.top(of: i)
                if nudging, let hover, i >= hover { top += h }
                return DndRenderItem(
                    key: d.collapsed[i], index: i, top: top, height: h,
                    selected: false, selFirst: false, selLast: false
                )
            }
            return DndRenderState(
                isDragging: true,
                listHeight: listHeight(contentHeight),
                contentHeight: contentHeight,
                items: items,
                placeholderY: hover.map { CGFloat($0) * h - viewport.scrollTop }
            )
        }

        let selected = selection.selectedKeySet
        let contentHeight = virt.totalHeight(count: order.count)
        let range = virt.range(scrollTop: viewport.scrollTop, viewportHeight: viewport.height, count: order.count)
        let items = range.map { i -> DndRenderItem in
            let key = order[i]
            let isSelected = selected.contains(key)
            var selFirst = false
            var selLast = false
            if isSelected, config.roundedSelect {
                selFirst = i == 0 || !selected.contains(order[i - 1])
                selLast = i == order.count - 1 || !selected.contains(order[i + 1])
            }
            return DndRenderItem(
                key: key, index: i, top: virt.top(of: i), height: h,
                selected: isSelected, selFirst: selFirst, selLast: selLast
            )
        }
        return DndRenderState(
            isDragging: false,
            listHeight: listHeight(contentHeight),
            contentHeight: contentHeight,
            items: items,
            placeholderY: nil
        )
    }

    private func listHeight(_ contentHeight: CGFloat) -> CGFloat {
        config.fillHeight ? max(contentHeight, viewport.height - viewport.topInset) : contentHeight
    }

    /// The furthest the list scrolls in its current layout.
    public var maxScrollTop: CGFloat {
        let count = drag.map { $0.collapsed.count + (config.reorder && config.nudge && $0.hoverIndex != nil ? 1 : 0) }
            ?? order.count
        return max(minScrollTop, virtualization.totalHeight(count: count) - viewport.height)
    }

    /// The rest position: the first row just below the top inset.
    public var minScrollTop: CGFloat { -viewport.topInset }

    /// The row under a viewport point in the idle layout, nil for blank
    /// space. The view uses its own mounted rows; this is for callers
    /// with no view.
    public func key(at point: CGPoint) -> String? {
        guard drag == nil, viewport.contains(point) else { return nil }
        let i = Int(((point.y + viewport.scrollTop) / config.itemHeight).rounded(.down))
        return order.indices.contains(i) ? order[i] : nil
    }

    // MARK: Pointer

    /// Button down. A plain press on an unselected row selects it alone
    /// straight away and arms a drag; a press on a selected row leaves
    /// the selection for the drag to carry. Shift / ⌘ presses wait for
    /// the release (`pointerUp`), so they never start a drag.
    public func pointerDown(key: String?, at point: CGPoint, modifiers: DndModifiers = []) {
        let modified = !modifiers.isDisjoint(with: [.shift, .command])
        let armed = key != nil && !modified && drag == nil
        if armed, let key, !selection.isSelected(key) {
            selection.selectOnly(key)
        }
        press = Press(key: key, point: point, armed: armed)
    }

    /// Pointer moved with the button down. Returns `.began` on the move
    /// that crosses the slop and starts the drag, `.moved` for every
    /// later one, nil while still a click.
    public func pointerDragged(to point: CGPoint) -> DndDragPhase? {
        if drag != nil {
            drag?.pointer = point
            updateHover()
            return .moved
        }
        guard config.drag, let press, press.armed, let grabKey = press.key else { return nil }
        let dx = point.x - press.point.x
        let dy = point.y - press.point.y
        guard (dx * dx + dy * dy).squareRoot() >= config.dragSlop else { return nil }

        let keys = selection.selectedKeys
        guard !keys.isEmpty else { return nil }
        let dragged = Set(keys)
        drag = Drag(
            keys: keys,
            grabKey: grabKey,
            collapsed: order.filter { !dragged.contains($0) },
            hoverIndex: nil,
            pointer: point
        )
        updateHover()
        return .began
    }

    /// Button up with no drag in flight: the click. Blank space clears
    /// (when configured), shift extends, ⌘ toggles, plain selects only.
    public func click(modifiers: DndModifiers = []) {
        guard let press, drag == nil else { return }
        self.press = nil
        guard let key = press.key else {
            if config.clearOnBlankClick { selection.clear() }
            return
        }
        if config.multi, modifiers.contains(.shift) {
            selection.extendActive(key)
        } else if config.multi, modifiers.contains(.command) {
            selection.toggle(key)
        } else {
            selection.selectOnly(key)
        }
    }

    /// Button up on a drag. Unless a foreign target `consumed` it, the
    /// rows land at the hover slot: the order is updated here
    /// (optimistically, the host confirms with `setOrder`) and the move
    /// returned. Nil when nothing moved.
    public func endDrag(consumed: Bool = false) -> DndMove? {
        guard let d = drag else { return nil }
        drag = nil
        press = nil
        guard !consumed, config.reorder, let hover = d.hoverIndex else { return nil }
        let move = DndMove(keys: d.keys, before: hover < d.collapsed.count ? d.collapsed[hover] : nil)
        return commit(move)
    }

    /// Abort the drag with no reorder. False when none was in flight.
    @discardableResult
    public func cancelDrag() -> Bool {
        guard drag != nil else { return false }
        drag = nil
        press = nil
        return true
    }

    /// The viewport scrolled under a still pointer (autoscroll, wheel):
    /// the hover slot depends on `scrollTop` as much as on the pointer.
    public func scrolled() {
        updateHover()
    }

    private func updateHover() {
        guard var d = drag else { return }
        if viewport.contains(d.pointer) {
            let y = d.pointer.y + viewport.scrollTop
            let slot = Int((y / config.itemHeight).rounded(.down))
            d.hoverIndex = min(max(slot, 0), d.collapsed.count)
        } else {
            d.hoverIndex = nil
        }
        drag = d
    }

    private func commit(_ move: DndMove) -> DndMove? {
        let next = move.applied(to: order)
        guard next != order else { return nil }
        order = next
        selection.updateOrder(next)
        // The moved rows are one run now; re-merge the blocks over it.
        selection.setSelectedKeys(move.keys)
        return move
    }

    // MARK: Keyboard

    public struct KeyResult: Equatable {
        /// The row to scroll into view: the leading edge of the change.
        public var scrollTo: String?
        /// A reorder the host must persist (⌘↑ / ⌘↓).
        public var move: DndMove?
    }

    /// True when a plain ↓ has nowhere to go: the selection's bottom is
    /// the last row, or the list is empty. The host may take the key on
    /// to whatever follows the rows.
    public var navigateDownIsPastEnd: Bool {
        order.isEmpty || selection.bottom == order.last
    }

    /// Apply a key action. Nil when the list does not answer it (so the
    /// view passes the event on).
    public func handleKey(_ action: DndKeyAction) -> KeyResult? {
        var result = KeyResult()
        let sel = selection
        func edge(_ dir: DndDirection) -> String? { dir == .down ? order.first : order.last }
        func only(_ key: String?) {
            guard let key else { return }
            sel.selectOnly(key)
            result.scrollTo = key
        }
        func extend(_ key: String?) {
            guard let key else { return }
            if config.multi { sel.extendActive(key) } else { sel.selectOnly(key) }
            result.scrollTo = key
        }

        switch action {
        case .selectOnlyFirst:
            only(order.first)
        case .selectOnlyLast:
            only(order.last)
        case .navigate(let dir):
            guard config.arrowNavigate else { return nil }
            if !sel.hasSelection {
                only(edge(dir))
            } else if dir == .down, let bottom = sel.bottom {
                only(sel.next(bottom))
            } else if let top = sel.top {
                only(sel.prev(top))
            }
        case .extend(let dir):
            if let active = sel.activeBlock {
                extend(dir == .down ? sel.next(active.to) : sel.prev(active.to))
            } else {
                only(edge(dir))
            }
        case .extendToFirst:
            extend(order.first)
        case .extendToLast:
            extend(order.last)
        case .selectAll:
            if config.multi { sel.selectAll() }
        case .clear:
            sel.clear()
        case .moveSelection(let dir):
            guard config.reorder else { return result }
            result.move = moveSelection(dir)
            if result.move != nil {
                result.scrollTo = dir == .up ? sel.top : sel.bottom
            }
        }
        return result
    }

    /// ⌘↑ / ⌘↓: the selection moves one row past its neighbour, as one
    /// block. Nil at the list's edge.
    private func moveSelection(_ dir: DndDirection) -> DndMove? {
        let keys = selection.selectedKeys
        guard !keys.isEmpty, drag == nil else { return nil }
        let moving = Set(keys)
        let rest = order.filter { !moving.contains($0) }
        let before: String?
        switch dir {
        case .up:
            guard let top = selection.top, let idx = order.firstIndex(of: top), idx > 0 else { return nil }
            let above = order[idx - 1]
            guard !moving.contains(above) else { return nil }
            before = above
        case .down:
            guard let bottom = selection.bottom, let idx = order.firstIndex(of: bottom),
                  idx < order.count - 1 else { return nil }
            let below = order[idx + 1]
            guard !moving.contains(below), let at = rest.firstIndex(of: below) else { return nil }
            before = at + 1 < rest.count ? rest[at + 1] : nil
        }
        return commit(DndMove(keys: keys, before: before))
    }

    /// Where to scroll so `key` is in view, nil when it already is. The
    /// last row scrolls to the end of the content.
    public func scrollOffset(toReveal key: String) -> CGFloat? {
        guard let idx = order.firstIndex(of: key) else { return nil }
        let top = virtualization.top(of: idx)
        let bottom = top + config.itemHeight
        if top < viewport.scrollTop + viewport.topInset {
            return top - viewport.topInset
        }
        guard bottom > viewport.scrollTop + viewport.height else { return nil }
        return idx == order.count - 1 ? maxScrollTop : bottom - viewport.height
    }
}
