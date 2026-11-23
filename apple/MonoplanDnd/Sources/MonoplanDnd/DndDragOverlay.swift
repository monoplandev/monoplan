#if canImport(AppKit)
import AppKit
import MonoplanDndCore

/// The drag stack: snapshots of the first few dragged rows following
/// the pointer, each a little behind the one above, with a count badge.
/// The port of `dom/drag-overlay.ts`.
///
/// The web's full-page `position:fixed` overlay is a borderless,
/// transparent, click-through child window over every screen, so the
/// stack can leave the list and the app's window. Its rows are layers
/// moved once per frame.
///
/// Public for a view that is not a `DndListView` and drags its own rows
/// (the Mac Calendar's table, `AgendaTable.swift`), as the web's agenda
/// uses `DndDragOverlay` without the list.
public final class DndDragOverlay {
    private let window: NSWindow
    private let root = CALayer()
    private var items: [CALayer] = []
    /// Bottom-left of each stack item, in screen coordinates.
    private var positions: [CGPoint] = []
    private var badge: CALayer?
    private var cursor: CGPoint
    private let grabOffset: CGPoint
    private let itemSize: CGSize

    /// - Parameters:
    ///   - snapshots: the dragged rows' images, the first `stackCount`.
    ///   - cursor: the pointer, screen coordinates.
    ///   - grabRect: the grabbed row's rect on screen. The stack keeps
    ///     the pointer where it took hold of that row.
    public init(
        parent: NSWindow?,
        snapshots: [CGImage],
        itemSize: CGSize,
        totalCount: Int,
        cursor: CGPoint,
        grabRect: CGRect,
        radius: CGFloat,
        background: CGColor,
        badgeColor: CGColor,
        badgeText: String?
    ) {
        self.cursor = cursor
        self.itemSize = itemSize
        grabOffset = CGPoint(x: grabRect.minX - cursor.x, y: grabRect.minY - cursor.y)

        let frame = NSScreen.screens.reduce(parent?.screen?.frame ?? .zero) { $0.union($1.frame) }
        window = NSWindow(contentRect: frame, styleMask: .borderless, backing: .buffered, defer: false)
        window.isOpaque = false
        window.backgroundColor = .clear
        window.hasShadow = false
        window.ignoresMouseEvents = true
        window.isReleasedWhenClosed = false
        window.animationBehavior = .none
        window.level = .floating

        let scale = parent?.backingScaleFactor ?? 2
        let view = NSView(frame: NSRect(origin: .zero, size: frame.size))
        view.layer = root
        view.wantsLayer = true
        window.contentView = view

        CATransaction.begin()
        CATransaction.setDisableActions(true)
        let lead = CGPoint(x: cursor.x + grabOffset.x, y: cursor.y + grabOffset.y)
        for (i, image) in snapshots.enumerated() {
            // The outer layer casts the shadow, the inner one clips the
            // snapshot to the rounded row.
            let item = CALayer()
            item.anchorPoint = .zero
            item.bounds = CGRect(origin: .zero, size: itemSize)
            item.opacity = Float(1 - Double(i) * 0.1)
            item.zPosition = CGFloat(snapshots.count - i)
            if i == 0 {
                item.shadowColor = NSColor.black.cgColor
                item.shadowOpacity = 0.22
                item.shadowRadius = 16
                item.shadowOffset = CGSize(width: 0, height: -8)
                item.shadowPath = CGPath(
                    roundedRect: item.bounds, cornerWidth: radius, cornerHeight: radius, transform: nil
                )
            }
            let face = CALayer()
            face.frame = item.bounds
            face.cornerRadius = radius
            face.masksToBounds = true
            face.backgroundColor = background
            face.contents = image
            face.contentsScale = scale
            item.addSublayer(face)
            root.addSublayer(item)
            items.append(item)
            positions.append(lead)
        }

        if let badgeText {
            let font = NSFont.systemFont(ofSize: 12, weight: .medium)
            let textSize = (badgeText as NSString).size(withAttributes: [.font: font])
            let pill = CALayer()
            pill.anchorPoint = CGPoint(x: 0.5, y: 0)
            pill.bounds = CGRect(x: 0, y: 0, width: ceil(textSize.width) + 16, height: 20)
            pill.cornerRadius = 10
            pill.backgroundColor = badgeColor
            pill.zPosition = CGFloat(snapshots.count + 1)
            let text = CATextLayer()
            text.string = badgeText
            text.font = font
            text.fontSize = 12
            text.foregroundColor = NSColor.white.cgColor
            text.alignmentMode = .center
            text.contentsScale = scale
            text.frame = CGRect(
                x: 0, y: (20 - ceil(textSize.height)) / 2,
                width: pill.bounds.width, height: ceil(textSize.height)
            )
            pill.addSublayer(text)
            root.addSublayer(pill)
            badge = pill
        }
        place()
        CATransaction.commit()

        // An off-screen parent (the scripted harness) gets no window on
        // screen; the layers still track.
        if let parent, parent.isVisible {
            parent.addChildWindow(window, ordered: .above)
        }
    }

    public func update(cursor: CGPoint) {
        self.cursor = cursor
    }

    /// One frame: the lead item closes most of the distance to the
    /// pointer, each deeper one chases the item above it more slowly.
    public func step(dt: TimeInterval) {
        let target = CGPoint(x: cursor.x + grabOffset.x, y: cursor.y + grabOffset.y)
        for i in positions.indices {
            let goal = i == 0 ? target : positions[i - 1]
            let perFrame = i == 0 ? 0.8 : max(0.6 - Double(i) * 0.03, 0.4)
            let f = dndLerpFactor(perFrame: perFrame, dt: dt)
            positions[i].x += (goal.x - positions[i].x) * f
            positions[i].y += (goal.y - positions[i].y) * f
        }
        CATransaction.begin()
        CATransaction.setDisableActions(true)
        place()
        CATransaction.commit()
    }

    private func place() {
        let origin = window.frame.origin
        for (i, item) in items.enumerated() {
            // Each depth sits 2 px lower than the one above.
            item.position = CGPoint(
                x: positions[i].x - origin.x,
                y: positions[i].y - origin.y - CGFloat(i) * 2
            )
        }
        // The badge rides 12 px above the pointer itself, not the stack.
        badge?.position = CGPoint(x: cursor.x - origin.x, y: cursor.y - origin.y + 12)
    }

    public func stop() {
        window.parent?.removeChildWindow(window)
        window.orderOut(nil)
    }

    /// Each stack item's rect on screen, lead first: for the harness.
    var itemFrames: [CGRect] {
        positions.enumerated().map { i, p in
            CGRect(origin: CGPoint(x: p.x, y: p.y - CGFloat(i) * 2), size: itemSize)
        }
    }

    var images: [CGImage] {
        items.compactMap { layer in
            guard let contents = layer.sublayers?.first?.contents else { return nil }
            return (contents as! CGImage)
        }
    }
}
#endif
