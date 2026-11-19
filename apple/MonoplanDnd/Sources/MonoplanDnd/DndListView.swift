#if canImport(AppKit)
import AppKit
import MonoplanDndCore

/// Supplies the rows' content. Content views are pooled: one is made
/// per visible row, then re-configured as rows scroll through.
public protocol DndListDataSource: AnyObject {
    func dndListMakeRowContent(_ list: DndListView) -> NSView
    func dndList(_ list: DndListView, configure content: NSView, key: String, state: DndRowState)
}

public protocol DndListDelegate: AnyObject {
    /// The user reordered: `keys` (list order) now sit directly above
    /// `before` (nil: at the end). The list already shows the new order;
    /// persist it, and call `setOrder` if the truth differs.
    func dndList(_ list: DndListView, didMove keys: [String], before: String?)
    func dndListSelectionDidChange(_ list: DndListView)
    func dndList(_ list: DndListView, didDoubleClick key: String)
    func dndList(_ list: DndListView, menuFor keys: [String]) -> NSMenu?
    /// A plain ↓ with nowhere further to go. True: the host took the
    /// keyboard on (to a capture row, say).
    func dndListNavigatePastEnd(_ list: DndListView) -> Bool

    // A drag, for drop targets outside the list (the sidebar). Points
    // are in the window's coordinates. The web's `primavera-dnd-drag*`.
    func dndList(_ list: DndListView, dragDidBegin keys: [String])
    func dndList(_ list: DndListView, dragDidMoveTo windowPoint: NSPoint, keys: [String])
    /// The button came up. True: a foreign target took the rows, and the
    /// list must not reorder.
    func dndList(_ list: DndListView, dragWillEndAt windowPoint: NSPoint, keys: [String]) -> Bool
    /// The drag is over, however it ended (dropped, consumed, cancelled).
    func dndListDragDidEnd(_ list: DndListView)
}

public extension DndListDelegate {
    func dndList(_ list: DndListView, didMove keys: [String], before: String?) {}
    func dndListSelectionDidChange(_ list: DndListView) {}
    func dndList(_ list: DndListView, didDoubleClick key: String) {}
    func dndList(_ list: DndListView, menuFor keys: [String]) -> NSMenu? { nil }
    func dndListNavigatePastEnd(_ list: DndListView) -> Bool { false }
    func dndList(_ list: DndListView, dragDidBegin keys: [String]) {}
    func dndList(_ list: DndListView, dragDidMoveTo windowPoint: NSPoint, keys: [String]) {}
    func dndList(_ list: DndListView, dragWillEndAt windowPoint: NSPoint, keys: [String]) -> Bool { false }
    func dndListDragDidEnd(_ list: DndListView) {}
}

/// The web's `<Dnd>` on AppKit: a virtualised, multi-select, reorderable
/// list of fixed-height rows. Same structure as the web component:
///
///     DndListView            host: positioning context
///     ├── placeholder        the drop slot, behind the scroll view
///     └── NSScrollView       parent: the scroll viewport
///         └── DndListbox     listbox: as tall as the whole list
///             └── DndRowView option: one per row in view, frame-positioned
///
/// Behaviour lives in `DndListModel`; this view turns events into model
/// input, then lays the rows out from `renderState()`.
public final class DndListView: NSView {
    /// The web's `transition: top 0.15s ease`.
    static let moveDuration: CFTimeInterval = 0.15

    public let model: DndListModel
    public weak var dataSource: DndListDataSource?
    public weak var delegate: DndListDelegate?

    public let scrollView = NSScrollView()
    private let listbox = DndListbox()
    private let placeholder = NSView()

    private var mounted: [String: DndRowView] = [:]
    private var pool: [DndRowView] = []

    private var overlay: DndDragOverlay?
    private var autoscroll: DndAutoscroll
    /// Where the smooth scroll is heading; nil at rest.
    private var scrollTarget: CGFloat?
    private var link: CADisplayLink?
    private var lastTick: CFTimeInterval = 0
    private var escapeMonitor: Any?
    private var isRendering = false
    private var needsRerender = false
    private var emphasized = false
    private var restInset: CGFloat = 0

    /// False: nothing ticks by itself; the caller advances time with
    /// `step(dt:)` (the scripted harness).
    public var drivesFrames = true
    /// The count badge's text for a drag of `n` rows (n > 1).
    public var badgeText: (Int) -> String = { "\($0)" }
    public var selectionRadius: CGFloat = 4
    /// The selection's and the placeholder's margin from the list's
    /// left and right edges (an inset list).
    public var selectionInset: CGFloat = 0
    public var selectionColor: NSColor = .selectedContentBackgroundColor
    public var dormantSelectionColor: NSColor = .unemphasizedSelectedContentBackgroundColor
    public var placeholderColor: NSColor = .quaternaryLabelColor { didSet { needsLayout = true } }
    /// Painted under each row of the drag stack (rows are often clear).
    public var dragBackgroundColor: NSColor = .windowBackgroundColor

    public override var isFlipped: Bool { true }
    public override var acceptsFirstResponder: Bool { true }

    public init(config: DndConfig = DndConfig()) {
        model = DndListModel(config: config)
        autoscroll = DndAutoscroll(buffer: config.autoscrollBuffer, confine: config.confineAutoscroll)
        super.init(frame: .zero)
        wantsLayer = true

        placeholder.wantsLayer = true
        placeholder.alphaValue = 0
        addSubview(placeholder)

        listbox.owner = self
        scrollView.drawsBackground = false
        scrollView.hasVerticalScroller = true
        scrollView.autohidesScrollers = true
        scrollView.documentView = listbox
        scrollView.frame = bounds
        scrollView.autoresizingMask = [.width, .height]
        scrollView.contentView.postsBoundsChangedNotifications = true
        addSubview(scrollView)

        let center = NotificationCenter.default
        center.addObserver(
            self, selector: #selector(clipBoundsChanged),
            name: NSView.boundsDidChangeNotification, object: scrollView.contentView
        )
        center.addObserver(
            self, selector: #selector(userScrolled),
            name: NSScrollView.willStartLiveScrollNotification, object: scrollView
        )
        model.selection.onChange = { [weak self] in
            guard let self else { return }
            self.delegate?.dndListSelectionDidChange(self)
        }
    }

    @available(*, unavailable)
    public required init?(coder: NSCoder) { fatalError("init(coder:) is not used") }

    deinit {
        link?.invalidate()
        if let escapeMonitor { NSEvent.removeMonitor(escapeMonitor) }
    }

    // MARK: Host API

    public var order: [String] { model.order }
    public var selectedKeys: [String] { model.selection.selectedKeys }

    /// Take the host's order (an external change, or the confirmed
    /// result of a move). Rows already on screen slide to their places.
    public func setOrder(_ order: [String], animated: Bool = true) {
        model.setOrder(order)
        if !model.isDragging, overlay != nil { finishDrag() }
        render(animated: animated)
    }

    /// Re-configure rows whose content changed (nil: every mounted row).
    public func reloadRows(_ keys: [String]? = nil) {
        for (key, row) in mounted where keys?.contains(key) ?? true {
            if let state = row.state {
                dataSource?.dndList(self, configure: row.content, key: key, state: state)
            }
        }
    }

    /// To the rest position, at once.
    public func scrollToTop() {
        scrollTarget = nil
        setScrollTop(-scrollView.contentInsets.top)
    }

    public func scrollToKey(_ key: String) {
        syncViewport()
        guard let offset = model.scrollOffset(toReveal: key) else { return }
        scrollTarget = offset
        updateTicker()
    }

    // MARK: Layout

    public override func layout() {
        super.layout()
        settleInset()
        render(animated: false)
    }

    /// The top inset (a toolbar over the list) arrives after the first
    /// layout: a list still at its old rest position follows it.
    private func settleInset() {
        let inset = scrollView.contentInsets.top
        guard inset != restInset else { return }
        let atRest = abs(scrollTop + restInset) < 0.5
        restInset = inset
        if atRest { setScrollTop(-inset) }
    }

    public override func viewDidMoveToWindow() {
        super.viewDidMoveToWindow()
        let center = NotificationCenter.default
        center.removeObserver(self, name: NSWindow.didBecomeKeyNotification, object: nil)
        center.removeObserver(self, name: NSWindow.didResignKeyNotification, object: nil)
        if let window {
            center.addObserver(self, selector: #selector(focusChanged), name: NSWindow.didBecomeKeyNotification, object: window)
            center.addObserver(self, selector: #selector(focusChanged), name: NSWindow.didResignKeyNotification, object: window)
        } else {
            abortDrag()
        }
    }

    public override func viewDidChangeEffectiveAppearance() {
        super.viewDidChangeEffectiveAppearance()
        needsLayout = true
    }

    private var scrollTop: CGFloat { scrollView.contentView.bounds.origin.y }

    private func setScrollTop(_ y: CGFloat) {
        let clip = scrollView.contentView
        let minY = -scrollView.contentInsets.top
        let maxY = max(minY, listbox.frame.height - clip.bounds.height)
        let clamped = min(max(y, minY), maxY)
        guard clamped != clip.bounds.origin.y else { return }
        clip.scroll(to: NSPoint(x: 0, y: clamped))
        scrollView.reflectScrolledClipView(clip)
    }

    private func syncViewport() {
        let clip = scrollView.contentView.bounds
        model.viewport = DndViewport(
            scrollTop: clip.origin.y, width: clip.width, height: clip.height,
            topInset: scrollView.contentInsets.top
        )
    }

    @objc private func clipBoundsChanged() {
        settleInset()
        syncViewport()
        // Autoscroll and the wheel move the content under a still
        // pointer: the hover slot follows.
        model.scrolled()
        render()
    }

    @objc private func userScrolled() {
        scrollTarget = nil
    }

    /// Lay the rows, the placeholder and the listbox out from the model.
    /// Rows that were already mounted slide to a new `top`; rows mounted
    /// by this pass appear in place.
    public func render(animated: Bool = true) {
        if isRendering {
            needsRerender = true
            return
        }
        isRendering = true
        defer {
            isRendering = false
            if needsRerender {
                needsRerender = false
                render(animated: animated)
            }
        }

        syncViewport()
        let state = model.renderState()
        let width = scrollView.contentView.bounds.width
        let size = NSSize(width: width, height: state.listHeight)
        if listbox.frame.size != size {
            // May clamp the scroll position (the list collapses when a
            // drag starts); that re-enters through the bounds notice.
            listbox.setFrameSize(size)
        }

        let animate = animated && window != nil
            && !NSWorkspace.shared.accessibilityDisplayShouldReduceMotion
        // Rows out of the window, and the dragged rows for the length of
        // the drag, leave the listbox first, so the rows this pass mounts
        // reuse them: a jump down the list swaps the whole window, and
        // would otherwise build a second set of row views. They are
        // re-mounted fresh, in place, when they are next wanted.
        let seen = Set(state.items.map(\.key))
        for (key, row) in mounted where !seen.contains(key) {
            row.removeFromSuperview()
            row.reset()
            pool.append(row)
            mounted[key] = nil
        }
        for item in state.items {
            let frame = NSRect(x: 0, y: item.top, width: width, height: item.height)
            let rowState = DndRowState(
                selected: item.selected, selFirst: item.selFirst, selLast: item.selLast,
                emphasized: emphasized
            )
            let row: DndRowView
            if let existing = mounted[item.key] {
                row = existing
                row.move(to: frame, animated: animate)
            } else {
                row = dequeueRow()
                row.key = item.key
                row.frame = frame
                listbox.addSubview(row)
                mounted[item.key] = row
            }
            row.selectionRadius = selectionRadius
            row.selectionInset = selectionInset
            row.selectionColor = selectionColor
            row.dormantSelectionColor = dormantSelectionColor
            if row.apply(rowState) {
                dataSource?.dndList(self, configure: row.content, key: item.key, state: rowState)
            }
        }
        renderPlaceholder(state.placeholderY)
    }

    private func dequeueRow() -> DndRowView {
        if let row = pool.popLast() { return row }
        return DndRowView(content: dataSource?.dndListMakeRowContent(self) ?? NSView())
    }

    /// The placeholder is a sibling of the scroll view, in viewport
    /// coordinates, so it does not ride the scrolled content. It moves
    /// without animation and fades in and out.
    private func renderPlaceholder(_ y: CGFloat?) {
        effectiveAppearance.performAsCurrentDrawingAppearance {
            placeholder.layer?.backgroundColor = placeholderColor.cgColor
        }
        placeholder.layer?.cornerRadius = selectionRadius
        if let y {
            placeholder.frame = NSRect(
                x: selectionInset, y: y,
                width: scrollView.contentView.bounds.width - selectionInset * 2,
                height: model.config.itemHeight - model.config.placeholderGap
            )
        }
        let alpha: CGFloat = y == nil ? 0 : 1
        guard placeholder.alphaValue != alpha else { return }
        if window == nil || !drivesFrames {
            placeholder.alphaValue = alpha
        } else {
            NSAnimationContext.runAnimationGroup { context in
                context.duration = Self.moveDuration
                placeholder.animator().alphaValue = alpha
            }
        }
    }

    // MARK: Focus

    public override func becomeFirstResponder() -> Bool {
        defer { focusChanged() }
        return true
    }

    public override func resignFirstResponder() -> Bool {
        defer { DispatchQueue.main.async { [weak self] in self?.focusChanged() } }
        return true
    }

    /// Selection is live while the list has the keyboard, dormant
    /// otherwise (Finder's model; the web's `:focus-within`).
    @objc private func focusChanged() {
        let next = window?.firstResponder === self && window?.isKeyWindow == true
        guard next != emphasized else { return }
        emphasized = next
        render(animated: false)
    }

    // MARK: Pointer

    private func viewportPoint(_ event: NSEvent) -> CGPoint {
        convert(event.locationInWindow, from: nil)
    }

    private func modifiers(_ event: NSEvent) -> DndModifiers {
        var mods: DndModifiers = []
        if event.modifierFlags.contains(.shift) { mods.insert(.shift) }
        if event.modifierFlags.contains(.command) { mods.insert(.command) }
        if event.modifierFlags.contains(.option) { mods.insert(.option) }
        return mods
    }

    /// The row under a viewport point, by the rows as laid out.
    public func key(at point: CGPoint) -> String? {
        guard scrollView.frame.contains(point) else { return nil }
        let p = listbox.convert(point, from: self)
        return mounted.first { $0.value.frame.contains(p) }?.key
    }

    public override func mouseDown(with event: NSEvent) {
        window?.makeFirstResponder(self)
        pointerDown(at: viewportPoint(event), modifiers: modifiers(event))
    }

    public override func mouseDragged(with event: NSEvent) {
        pointerDragged(to: viewportPoint(event))
    }

    public override func mouseUp(with event: NSEvent) {
        let wasDragging = model.isDragging
        let key = key(at: viewportPoint(event))
        pointerUp(modifiers: modifiers(event))
        if !wasDragging, event.clickCount == 2, let key {
            delegate?.dndList(self, didDoubleClick: key)
        }
    }

    func contextMenu(for event: NSEvent) -> NSMenu? {
        let point = viewportPoint(event)
        guard let key = key(at: point) else { return nil }
        // A right-click outside the selection acts on that row alone.
        if !model.selection.isSelected(key) {
            model.selection.selectOnly(key)
            render()
        }
        return delegate?.dndList(self, menuFor: model.selection.selectedKeys)
    }

    // The pointer entry points take viewport coordinates (this view's
    // own, y down). The mouse overrides above call them; so does a
    // script.

    public func pointerDown(at point: CGPoint, modifiers: DndModifiers = []) {
        syncViewport()
        scrollTarget = nil
        model.pointerDown(key: key(at: point), at: point, modifiers: modifiers)
        render()
    }

    public func pointerDragged(to point: CGPoint) {
        syncViewport()
        guard let phase = model.pointerDragged(to: point), let drag = model.drag else { return }
        let windowPoint = convert(point, to: nil)
        let screenPoint = window?.convertPoint(toScreen: windowPoint) ?? windowPoint
        if phase == .began {
            // Before the render below takes the dragged rows out.
            beginDrag(drag, cursor: screenPoint)
        }
        overlay?.update(cursor: screenPoint)
        autoscroll.buffer = model.config.autoscrollBuffer
        autoscroll.confine = model.config.confineAutoscroll
        autoscroll.update(pointer: point, viewport: model.viewport)
        render()
        delegate?.dndList(self, dragDidMoveTo: windowPoint, keys: drag.keys)
        updateTicker()
    }

    public func pointerUp(modifiers: DndModifiers = []) {
        guard let drag = model.drag else {
            model.click(modifiers: modifiers)
            render()
            return
        }
        let windowPoint = convert(drag.pointer, to: nil)
        let consumed = delegate?.dndList(self, dragWillEndAt: windowPoint, keys: drag.keys) ?? false
        let move = model.endDrag(consumed: consumed)
        finishDrag()
        render()
        if let move {
            delegate?.dndList(self, didMove: move.keys, before: move.before)
        }
    }

    /// Escape: the drag ends with nothing moved.
    public func cancelDrag() {
        guard model.cancelDrag() else { return }
        finishDrag()
        render()
    }

    private func abortDrag() {
        model.cancelDrag()
        if overlay != nil { finishDrag() }
    }

    private func beginDrag(_ drag: DndListModel.Drag, cursor: CGPoint) {
        let stack = drag.keys.compactMap { mounted[$0] }.prefix(model.config.dragStackCount)
        let grabRow = mounted[drag.grabKey] ?? stack.first
        // The stack is as wide as the selection, not the row: the
        // margins of an inset list stay behind.
        let grabInWindow = grabRow.map {
            $0.convert($0.bounds.insetBy(dx: selectionInset, dy: 0), to: nil)
        } ?? .zero
        let grabRect = window?.convertToScreen(grabInWindow) ?? grabInWindow
        let itemSize = CGSize(
            width: listbox.frame.width - selectionInset * 2,
            height: model.config.itemHeight - model.config.placeholderGap
        )
        var background = NSColor.windowBackgroundColor.cgColor
        var accent = NSColor.controlAccentColor.cgColor
        effectiveAppearance.performAsCurrentDrawingAppearance {
            background = dragBackgroundColor.cgColor
            accent = NSColor.controlAccentColor.cgColor
        }
        overlay = DndDragOverlay(
            parent: window,
            snapshots: stack.compactMap { $0.snapshot() },
            itemSize: itemSize,
            totalCount: drag.keys.count,
            cursor: cursor,
            grabRect: grabRect,
            radius: selectionRadius,
            background: background,
            badgeColor: accent,
            badgeText: drag.keys.count > 1 ? badgeText(drag.keys.count) : nil
        )
        // Wherever the keyboard is, Escape cancels while a drag is up.
        escapeMonitor = NSEvent.addLocalMonitorForEvents(matching: .keyDown) { [weak self] event in
            guard event.keyCode == 53, let self, self.model.isDragging else { return event }
            self.cancelDrag()
            return nil
        }
        delegate?.dndList(self, dragDidBegin: drag.keys)
    }

    private func finishDrag() {
        overlay?.stop()
        overlay = nil
        autoscroll.stop()
        if let escapeMonitor {
            NSEvent.removeMonitor(escapeMonitor)
            self.escapeMonitor = nil
        }
        updateTicker()
        delegate?.dndListDragDidEnd(self)
    }

    // MARK: Frames

    /// One display link drives the three things that move by themselves:
    /// autoscroll, the smooth scroll to a row, and the drag stack. It
    /// runs only while one of them is live.
    private func updateTicker() {
        let wanted = drivesFrames && window != nil
            && (overlay != nil || autoscroll.direction != nil || scrollTarget != nil)
        if wanted, link == nil {
            lastTick = 0
            let link = displayLink(target: self, selector: #selector(tick(_:)))
            link.add(to: .main, forMode: .common)
            self.link = link
        } else if !wanted, let link {
            link.invalidate()
            self.link = nil
        }
    }

    @objc private func tick(_ link: CADisplayLink) {
        let dt = lastTick == 0 ? 1.0 / 60 : link.timestamp - lastTick
        lastTick = link.timestamp
        step(dt: min(max(dt, 0), 0.05))
    }

    /// Advance everything time-driven by `dt` seconds.
    public func step(dt: TimeInterval) {
        let delta = autoscroll.step(dt: dt)
        if delta != 0 { setScrollTop(scrollTop + delta) }

        if let target = scrollTarget {
            // Close 35% of the distance per 60 Hz frame: tight enough to
            // keep up with a held arrow key.
            let goal = min(target, model.maxScrollTop)
            let diff = goal - scrollTop
            if abs(diff) < 1 {
                setScrollTop(goal)
                scrollTarget = nil
            } else {
                setScrollTop(scrollTop + diff * dndLerpFactor(perFrame: 0.35, dt: dt))
            }
        }

        overlay?.step(dt: dt)
        updateTicker()
    }

    // MARK: Keyboard

    public override func keyDown(with event: NSEvent) {
        let key: DndKey?
        switch event.specialKey {
        case .upArrow: key = .up
        case .downArrow: key = .down
        default:
            if event.keyCode == 53 {
                key = .escape
            } else if event.charactersIgnoringModifiers?.lowercased() == "a" {
                key = .a
            } else {
                key = nil
            }
        }
        guard let key, let action = dndKeyAction(key, modifiers(event)), perform(action) else {
            super.keyDown(with: event)
            return
        }
    }

    /// ⌘A arrives here when the app has an Edit > Select All item.
    public override func selectAll(_ sender: Any?) {
        perform(.selectAll)
    }

    /// Run a key action. False when the list does not answer it.
    @discardableResult
    public func perform(_ action: DndKeyAction) -> Bool {
        if model.isDragging {
            if action == .clear { cancelDrag() }
            return true
        }
        syncViewport()
        if action == .navigate(.down), model.config.arrowNavigate, model.navigateDownIsPastEnd,
           delegate?.dndListNavigatePastEnd(self) == true {
            return true
        }
        guard let result = model.handleKey(action) else { return false }
        render()
        if let move = result.move {
            delegate?.dndList(self, didMove: move.keys, before: move.before)
        }
        if let key = result.scrollTo { scrollToKey(key) }
        return true
    }

    // MARK: Harness

    /// What is on screen, in viewport coordinates: for the harness.
    public struct DebugFrame {
        public var scrollTop: CGFloat
        public var listHeight: CGFloat
        public var rows: [(key: String, frame: CGRect, selected: Bool)]
        public var placeholder: CGRect?
        /// The drag stack, lead first, in this view's coordinates.
        public var stack: [CGRect]
        public var stackImages: [CGImage]
        public var mountedCount: Int
    }

    public var debugFrame: DebugFrame {
        let rows = mounted.values
            .sorted { $0.frame.minY < $1.frame.minY }
            .map { (key: $0.key, frame: convert($0.frame, from: listbox), selected: $0.state?.selected ?? false) }
        let stack = (overlay?.itemFrames ?? []).map { rect -> CGRect in
            let inWindow = window?.convertFromScreen(rect) ?? rect
            return convert(inWindow, from: nil)
        }
        return DebugFrame(
            scrollTop: scrollTop,
            listHeight: listbox.frame.height,
            rows: rows,
            placeholder: placeholder.alphaValue > 0 ? placeholder.frame : nil,
            stack: stack,
            stackImages: overlay?.images ?? [],
            mountedCount: mounted.count
        )
    }
}
#endif
