// The checkbox an item row leads with, as the iOS app draws it
// (`MonoplanIOS/Monoplan/ItemCheckbox.swift`): an empty rounded square
// while the item is open, ticked once Done, crossed when Cancelled (spec/data-model.md "Lifecycle"). `ItemCheckbox` is the
// SwiftUI drawing, `ItemCheckButton` the control that ticks and
// reopens, `ItemCheckArt` the same box as an image for the toolbar.
// Unlike iOS's filled accent box, a Done box in a row keeps the open
// box's border and no fill, with the tick in the text colour: a page of
// Done rows should not be a column of accent colour. The toolbar's one
// box, on the item's own surface, is still filled.

import AppKit
import MonoplanCore
import SwiftUI

/// The box's proportions, shared by the view and the image. iOS has a
/// 20 pt box beside 17 pt text; this is the same ratio at the Mac's 13.
private enum CheckMetrics {
    static let side: CGFloat = 15
    static let radius: CGFloat = 0.32
    static let border: CGFloat = 1.25
    static let nudge: CGFloat = 1
}

struct ItemCheckbox: View {
    let state: ItemState

    /// Drawn rather than an SF Symbol, whose corner radius and stroke
    /// are not ours to set. Sized with the text.
    @ScaledMetric(relativeTo: .body) private var side: CGFloat = CheckMetrics.side

    var body: some View {
        let shape = RoundedRectangle(cornerRadius: side * CheckMetrics.radius, style: .continuous)
        ZStack {
            shape.strokeBorder(.tertiary, lineWidth: CheckMetrics.border)
            if state == .done {
                mark(cancelled: false).foregroundStyle(.primary)
            } else if state == .cancelled {
                mark(cancelled: true).foregroundStyle(.tertiary)
            }
        }
        .frame(width: side, height: side)
        // A shape has no baseline of its own: sit the box so its middle
        // is level with the middle of the first line's capitals, then a
        // touch lower, where it looks centred on the line as a whole.
        .alignmentGuide(.firstTextBaseline) { $0[.bottom] - side * 0.2 - CheckMetrics.nudge }
    }

    private func mark(cancelled: Bool) -> some View {
        CheckGlyph(cancelled: cancelled).stroked(side)
    }
}

/// The checkbox's place on a Calendar row that has none (an event's):
/// empty, as wide as the box, so the texts stay in one column.
/// `EventPill` is drawn over it.
struct EventSlot: View {
    @ScaledMetric(relativeTo: .body) private var side: CGFloat = CheckMetrics.side

    var body: some View {
        Color.clear
            .frame(width: side, height: 1)
            .accessibilityHidden(true)
    }
}

/// What an event's Calendar row leads with instead of a checkbox: the
/// short upright bar a calendar leads an event with, grey, as tall as
/// the row and centred in the box's column. Blue while the event is
/// selected, on each of its rows (a span's other days too), as the
/// web's `.upcoming-row[data-picked]`. Display only; the item's panel
/// still marks it done.
struct EventPill: View {
    var picked = false
    @ScaledMetric(relativeTo: .body) private var side: CGFloat = CheckMetrics.side

    var body: some View {
        Capsule()
            .fill(picked ? AnyShapeStyle(Color(nsColor: .linkColor)) : AnyShapeStyle(.tertiary))
            .frame(width: 3)
            .padding(.vertical, 3)
            .frame(width: side)
            .accessibilityHidden(true)
    }
}

/// The checkbox as a control: a click ticks an open item and reopens a
/// closed one (`toggle` gets the new "done").
struct ItemCheckButton: View {
    let state: ItemState
    let toggle: (Bool) -> Void

    var body: some View {
        Button {
            toggle(!state.isClosed)
        } label: {
            ItemCheckbox(state: state).contentShape(Rectangle())
        }
        .buttonStyle(CheckPressStyle())
        .accessibilityLabel(
            state == .cancelled ? Text("Reopen") : state.isClosed ? Text("Mark as not done") : Text("Mark as done"))
    }
}

/// A checkbox button's press, as the web's (`input.task-check[data-pressed]`
/// in `styles.css`, and `RowCheck` in `ItemList.swift`): held down, the
/// box swells to 1.15 in 0.08 s; let go, it springs back over 0.22 s
/// with a little overshoot. In place of `.plain`'s dimming.
struct CheckPressStyle: ButtonStyle {
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    func makeBody(configuration: Configuration) -> some View {
        configuration.label
            .scaleEffect(configuration.isPressed && !reduceMotion ? 1.15 : 1)
            .animation(
                configuration.isPressed
                    ? .easeOut(duration: 0.08)
                    : .timingCurve(0.34, 1.56, 0.64, 1, duration: 0.22),
                value: configuration.isPressed)
    }
}

enum ItemCheckArt {
    /// The box as an image for an AppKit control. Drawn on demand, so
    /// it follows the appearance and the accent colour.
    static func image(_ state: ItemState, accessibilityDescription: String?) -> NSImage {
        let side = CheckMetrics.side
        let image = NSImage(size: NSSize(width: side, height: side), flipped: false) { rect in
            let radius = side * CheckMetrics.radius
            // Tertiary is too faint on a dark toolbar: secondary there.
            let dark = NSAppearance.currentDrawing().bestMatch(from: [.aqua, .darkAqua]) == .darkAqua
            let stroke = dark ? NSColor.secondaryLabelColor : NSColor.tertiaryLabelColor
            if state == .done {
                NSColor.controlAccentColor.setFill()
                NSBezierPath(roundedRect: rect, xRadius: radius, yRadius: radius).fill()
                mark(cancelled: false, colour: .white, in: rect)
            } else {
                let inset = rect.insetBy(dx: CheckMetrics.border / 2, dy: CheckMetrics.border / 2)
                let path = NSBezierPath(
                    roundedRect: inset, xRadius: radius - CheckMetrics.border / 2,
                    yRadius: radius - CheckMetrics.border / 2)
                path.lineWidth = CheckMetrics.border
                stroke.setStroke()
                path.stroke()
                if state == .cancelled { mark(cancelled: true, colour: stroke, in: rect) }
            }
            return true
        }
        image.accessibilityDescription = accessibilityDescription
        return image
    }

    /// A row's box (`ItemCheckbox`, not the toolbar's filled one), drawn
    /// centred in `bounds` in the current context, y up. For rows that
    /// are AppKit: a hosted SwiftUI box per row costs frames when a
    /// list recycles a windowful of rows at once.
    static func drawRowBox(_ state: ItemState, in bounds: NSRect) {
        let side = CheckMetrics.side
        let rect = NSRect(x: bounds.midX - side / 2, y: bounds.midY - side / 2, width: side, height: side)
        let radius = side * CheckMetrics.radius - CheckMetrics.border / 2
        let path = NSBezierPath(
            roundedRect: rect.insetBy(dx: CheckMetrics.border / 2, dy: CheckMetrics.border / 2),
            xRadius: radius, yRadius: radius)
        path.lineWidth = CheckMetrics.border
        NSColor.tertiaryLabelColor.setStroke()
        path.stroke()
        if state == .done {
            mark(cancelled: false, colour: .labelColor, in: rect)
        } else if state == .cancelled {
            mark(cancelled: true, colour: .tertiaryLabelColor, in: rect)
        }
    }

    /// `CheckGlyph`'s strokes, flipped into AppKit's y-up drawing.
    private static func mark(cancelled: Bool, colour: NSColor, in rect: NSRect) {
        let unit = rect.width / 16
        let path = NSBezierPath()
        for stroke in CheckGlyph.strokes(cancelled: cancelled) {
            for (i, point) in stroke.enumerated() {
                let p = NSPoint(x: rect.minX + point.x * unit, y: rect.maxY - point.y * unit)
                if i == 0 { path.move(to: p) } else { path.line(to: p) }
            }
        }
        path.lineWidth = rect.width / 8
        path.lineCapStyle = .round
        path.lineJoinStyle = .round
        colour.setStroke()
        path.stroke()
    }
}

/// The web's tick and cross (`js/web/src/icons/check.svg`, and the
/// `.task-check` cross in `styles.css`): strokes on a 16-unit grid
/// that spans the whole box, stroked an eighth of its side, rounded.
struct CheckGlyph: Shape {
    let cancelled: Bool

    /// The strokes as polylines in grid units, y down.
    static func strokes(cancelled: Bool) -> [[CGPoint]] {
        cancelled
            ? [[CGPoint(x: 5, y: 5), CGPoint(x: 11, y: 11)], [CGPoint(x: 11, y: 5), CGPoint(x: 5, y: 11)]]
            : [[CGPoint(x: 3.5, y: 8.5), CGPoint(x: 6.5, y: 11.5), CGPoint(x: 12.5, y: 5.5)]]
    }

    func path(in rect: CGRect) -> Path {
        let unit = rect.width / 16
        var path = Path()
        for stroke in Self.strokes(cancelled: cancelled) {
            path.addLines(stroke.map { CGPoint(x: rect.minX + $0.x * unit, y: rect.minY + $0.y * unit) })
        }
        return path
    }

    /// Stroked for a box `side` across.
    func stroked(_ side: CGFloat) -> some Shape {
        stroke(style: StrokeStyle(lineWidth: side / 8, lineCap: .round, lineJoin: .round))
    }
}
