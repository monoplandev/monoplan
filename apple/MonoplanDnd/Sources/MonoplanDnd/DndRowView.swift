#if canImport(AppKit)
import AppKit
import MonoplanDndCore

/// What a row's content is told to draw.
public struct DndRowState: Equatable {
    public var selected: Bool
    /// First / last row of a contiguous selected run.
    public var selFirst: Bool
    public var selLast: Bool
    /// The list has the keyboard: selection is live, not dormant.
    public var emphasized: Bool
}

/// A view inside a row that takes its own clicks (so a press on it
/// neither selects nor drags the row). Buttons, editable text fields
/// and text views count without it.
public protocol DndInteractiveView: NSView {}

/// The web's `role="option"` container: one fixed-height slot. It owns
/// the slot's frame, its move animation and the selection chrome; the
/// host's content view fills it.
final class DndRowView: NSView {
    var key = ""
    private(set) var state: DndRowState?
    let content: NSView
    var selectionRadius: CGFloat = 4
    /// The selection's margin from the row's left and right edges.
    var selectionInset: CGFloat = 0
    var selectionColor: NSColor = .selectedContentBackgroundColor
    var dormantSelectionColor: NSColor = .unemphasizedSelectedContentBackgroundColor
    override var isFlipped: Bool { true }

    init(content: NSView) {
        self.content = content
        super.init(frame: .zero)
        wantsLayer = true
        layerContentsRedrawPolicy = .onSetNeedsDisplay
        content.frame = bounds
        content.autoresizingMask = [.width, .height]
        addSubview(content)
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError("init(coder:) is not used") }

    /// Returns true when the state changed (the content needs telling).
    func apply(_ next: DndRowState) -> Bool {
        guard next != state else { return false }
        state = next
        // Selected rows paint above their neighbours while they slide.
        layer?.zPosition = next.selected ? 1 : 0
        needsDisplay = true
        return true
    }

    func reset() {
        state = nil
        layer?.removeAllAnimations()
        layer?.zPosition = 0
    }

    /// Put the row at `frame`. Animated, the row is at its new frame at
    /// once and slides there from where it was: an additive animation on
    /// the layer's position, so a second move mid-slide composes with the
    /// first instead of restarting it (the web's `transition: top 0.15s`).
    func move(to frame: NSRect, animated: Bool) {
        guard frame != self.frame else { return }
        // AppKit moves the backing layer with the frame, in the parent's
        // (flipped) coordinates: the layer's own delta is the slide.
        let from = layer?.position.y
        self.frame = frame
        guard animated, let layer, let from else { return }
        let delta = from - layer.position.y
        guard delta != 0 else { return }
        // Whole-pixel steps, 120 a second (`dndSlideSteps` says why).
        let steps = Int(DndListView.moveDuration * 120)
        let slide = CAKeyframeAnimation(keyPath: "position.y")
        slide.values = dndSlideSteps(delta: delta, steps: steps, scale: window?.backingScaleFactor ?? 2)
        slide.keyTimes = (0...steps).map { NSNumber(value: Double($0) / Double(steps)) }
        slide.calculationMode = .discrete
        slide.isAdditive = true
        slide.duration = DndListView.moveDuration
        layer.add(slide, forKey: nil)
    }

    override func draw(_ dirtyRect: NSRect) {
        guard let state, state.selected else { return }
        (state.emphasized ? selectionColor : dormantSelectionColor).setFill()
        let r = selectionRadius
        roundedPath(bounds.insetBy(dx: selectionInset, dy: 0), top: state.selFirst ? r : 0, bottom: state.selLast ? r : 0).fill()
    }

    /// A rect with its top pair and bottom pair of corners rounded
    /// separately (flipped coordinates: minY is the top).
    private func roundedPath(_ rect: NSRect, top: CGFloat, bottom: CGFloat) -> NSBezierPath {
        let path = NSBezierPath()
        path.move(to: NSPoint(x: rect.minX + top, y: rect.minY))
        path.line(to: NSPoint(x: rect.maxX - top, y: rect.minY))
        path.appendArc(from: NSPoint(x: rect.maxX, y: rect.minY), to: NSPoint(x: rect.maxX, y: rect.minY + top), radius: top)
        path.line(to: NSPoint(x: rect.maxX, y: rect.maxY - bottom))
        path.appendArc(from: NSPoint(x: rect.maxX, y: rect.maxY), to: NSPoint(x: rect.maxX - bottom, y: rect.maxY), radius: bottom)
        path.line(to: NSPoint(x: rect.minX + bottom, y: rect.maxY))
        path.appendArc(from: NSPoint(x: rect.minX, y: rect.maxY), to: NSPoint(x: rect.minX, y: rect.maxY - bottom), radius: bottom)
        path.line(to: NSPoint(x: rect.minX, y: rect.minY + top))
        path.appendArc(from: NSPoint(x: rect.minX, y: rect.minY), to: NSPoint(x: rect.minX + top, y: rect.minY), radius: top)
        path.close()
        return path
    }

    /// The row as drawn now, for the drag stack: the part the selection
    /// covers, without the margins either side of it.
    func snapshot() -> CGImage? {
        let rect = bounds.insetBy(dx: selectionInset, dy: 0)
        guard rect.width > 0, rect.height > 0,
              let rep = bitmapImageRepForCachingDisplay(in: rect) else { return nil }
        cacheDisplay(in: rect, to: rep)
        return rep.cgImage
    }
}

/// The web's `role="listbox"`: the scrolled document view. It takes
/// every click in a row that is not on an interactive view, so rows
/// select and drag from anywhere on them.
final class DndListbox: NSView {
    weak var owner: DndListView?

    override var isFlipped: Bool { true }

    override func hitTest(_ point: NSPoint) -> NSView? {
        guard let hit = super.hitTest(point) else { return nil }
        var view: NSView? = hit
        while let current = view, current !== self {
            if Self.isInteractive(current) { return hit }
            view = current.superview
        }
        return self
    }

    static func isInteractive(_ view: NSView) -> Bool {
        if view is DndInteractiveView || view is NSButton || view is NSTextView { return true }
        if let field = view as? NSTextField { return field.isEditable || field.isSelectable }
        return false
    }

    override func mouseDown(with event: NSEvent) { owner?.mouseDown(with: event) }
    override func mouseDragged(with event: NSEvent) { owner?.mouseDragged(with: event) }
    override func mouseUp(with event: NSEvent) { owner?.mouseUp(with: event) }
    override func menu(for event: NSEvent) -> NSMenu? { owner?.contextMenu(for: event) }
}
#endif
