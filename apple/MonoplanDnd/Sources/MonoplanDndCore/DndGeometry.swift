import CoreGraphics
import Foundation

public enum DndDirection: Equatable {
    case up
    case down
}

/// The scroll viewport as the model sees it. Every point the model takes
/// is in viewport coordinates: origin at the viewport's top left, y down,
/// not scrolled. List coordinates are `y + scrollTop`.
public struct DndViewport: Equatable {
    public var scrollTop: CGFloat
    public var width: CGFloat
    public var height: CGFloat
    /// The strip at the top of the viewport that content scrolls under
    /// (a toolbar). At rest `scrollTop` is `-topInset`: the first row
    /// sits just below the strip.
    public var topInset: CGFloat

    public init(scrollTop: CGFloat = 0, width: CGFloat = 0, height: CGFloat = 0, topInset: CGFloat = 0) {
        self.scrollTop = scrollTop
        self.width = width
        self.height = height
        self.topInset = topInset
    }

    public func contains(_ p: CGPoint) -> Bool {
        p.x >= 0 && p.x <= width && p.y >= 0 && p.y <= height
    }
}

/// Fixed-height windowing, the port of `core/virtualization.ts` without
/// the expanded row (expansion is not ported).
public struct DndVirtualization {
    public var itemHeight: CGFloat
    public var overscan: Int

    public init(itemHeight: CGFloat, overscan: Int = 2) {
        self.itemHeight = itemHeight
        self.overscan = overscan
    }

    /// Indices to render: the rows in view plus `overscan` either side.
    public func range(scrollTop: CGFloat, viewportHeight: CGFloat, count: Int) -> Range<Int> {
        let first = Int((max(scrollTop, 0) / itemHeight).rounded(.down))
        let start = min(max(first - overscan, 0), count)
        let visible = Int((max(viewportHeight, 0) / itemHeight).rounded(.up))
        let end = min(start + visible + overscan * 2, count)
        return start..<end
    }

    public func totalHeight(count: Int) -> CGFloat {
        CGFloat(count) * itemHeight
    }

    public func top(of index: Int) -> CGFloat {
        CGFloat(index) * itemHeight
    }
}

/// Edge autoscroll during a drag, the port of `dom/autoscroll.ts` with
/// the speed in px per second rather than per animation frame (the web's
/// 12 px per frame at 60 Hz). Starts at 20% of the top speed and reaches
/// it linearly over `acceleration`.
public struct DndAutoscroll {
    public var buffer: CGFloat
    public var confine: Bool
    public var maxSpeed: CGFloat = 720
    public var acceleration: TimeInterval = 0.5

    public private(set) var direction: DndDirection?
    private var elapsed: TimeInterval = 0

    public init(buffer: CGFloat = 32, confine: Bool = false) {
        self.buffer = buffer
        self.confine = confine
    }

    /// Call on every pointer move of a drag.
    public mutating func update(pointer: CGPoint, viewport: DndViewport) {
        if pointer.x < 0 || pointer.x > viewport.width {
            stop()
        } else if confine, pointer.y < 0 || pointer.y > viewport.height {
            stop()
        } else if pointer.y < viewport.topInset + buffer {
            start(.up)
        } else if pointer.y > viewport.height - buffer {
            start(.down)
        } else {
            stop()
        }
    }

    public mutating func stop() {
        direction = nil
        elapsed = 0
    }

    private mutating func start(_ dir: DndDirection) {
        guard direction != dir else { return }
        direction = dir
        elapsed = 0
    }

    /// Advance by `dt` and return the scroll delta to apply (0 at rest).
    public mutating func step(dt: TimeInterval) -> CGFloat {
        guard let direction else { return 0 }
        let t = min(elapsed / acceleration, 1)
        elapsed += dt
        let distance = maxSpeed * CGFloat(0.2 + 0.8 * t) * CGFloat(dt)
        return direction == .up ? -distance : distance
    }
}

/// The CSS `ease` curve, cubic-bezier(0.25, 0.1, 0.25, 1): progress in
/// 0...1 to eased progress.
public func dndEase(_ x: Double) -> Double {
    guard x > 0 else { return 0 }
    guard x < 1 else { return 1 }
    func bezier(_ t: Double, _ a: Double, _ b: Double) -> Double {
        let u = 1 - t
        return 3 * u * u * t * a + 3 * u * t * t * b + t * t * t
    }
    // x(t) is monotonic: bisect for t, then read y.
    var lo = 0.0
    var hi = 1.0
    for _ in 0..<24 {
        let mid = (lo + hi) / 2
        if bezier(mid, 0.25, 0.25) < x { lo = mid } else { hi = mid }
    }
    return bezier((lo + hi) / 2, 0.1, 1)
}

/// The offsets of a slide from `delta` to 0 over `steps` equal steps,
/// eased, each rounded to a whole device pixel. Rows that slide as
/// neighbours then stay edge to edge on every frame: at a fractional
/// offset the compositor blends each row's edge with what is behind
/// it, and a light seam shows between two rows of the same colour.
public func dndSlideSteps(delta: CGFloat, steps: Int, scale: CGFloat) -> [CGFloat] {
    (0..<max(steps, 1)).map { i in
        let left = 1 - dndEase(Double(i) / Double(max(steps, 1)))
        return (delta * CGFloat(left) * scale).rounded() / scale
    }
}

/// A frame-rate independent form of the web's "close `perFrame` of the
/// remaining distance each 60 Hz frame" lerp.
public func dndLerpFactor(perFrame: Double, dt: TimeInterval) -> CGFloat {
    CGFloat(1 - pow(1 - perFrame, dt * 60))
}
