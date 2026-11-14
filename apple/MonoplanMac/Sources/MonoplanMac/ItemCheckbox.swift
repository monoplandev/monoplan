// The checkbox an item row leads with, as the iOS app draws it
// (`MonoplanIOS/Monoplan/ItemCheckbox.swift`): an empty rounded square
// while the item is open, filled and ticked once Done, crossed when
// Cancelled (spec/data-model.md "Lifecycle"). `ItemCheckbox` is the
// SwiftUI drawing, `ItemCheckButton` the control that ticks and
// reopens, `ItemCheckArt` the same box as an image for the toolbar.

import AppKit
import MonoplanCore
import SwiftUI

/// The box's proportions, shared by the view and the image. iOS has a
/// 20 pt box beside 17 pt text; this is the same ratio at the Mac's 13.
private enum CheckMetrics {
    static let side: CGFloat = 15
    static let radius: CGFloat = 0.32
    static let border: CGFloat = 1.25
    static let mark: CGFloat = 0.55
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
            if state == .done {
                shape.fill(.tint)
                mark("checkmark").foregroundStyle(.white)
            } else {
                shape.strokeBorder(.tertiary, lineWidth: CheckMetrics.border)
                if state == .cancelled {
                    mark("xmark").foregroundStyle(.tertiary)
                }
            }
        }
        .frame(width: side, height: side)
        // A shape has no baseline of its own: sit the box so its middle
        // is level with the middle of the first line's capitals, then a
        // touch lower, where it looks centred on the line as a whole.
        .alignmentGuide(.firstTextBaseline) { $0[.bottom] - side * 0.2 - CheckMetrics.nudge }
    }

    private func mark(_ name: String) -> some View {
        Image(systemName: name)
            .font(.system(size: side * CheckMetrics.mark, weight: .semibold))
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
        .buttonStyle(.plain)
        .accessibilityLabel(
            state == .cancelled ? Text("Reopen") : state.isClosed ? Text("Mark as not done") : Text("Mark as done"))
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
                mark("checkmark", colour: .white, in: rect)
            } else {
                let inset = rect.insetBy(dx: CheckMetrics.border / 2, dy: CheckMetrics.border / 2)
                let path = NSBezierPath(
                    roundedRect: inset, xRadius: radius - CheckMetrics.border / 2,
                    yRadius: radius - CheckMetrics.border / 2)
                path.lineWidth = CheckMetrics.border
                stroke.setStroke()
                path.stroke()
                if state == .cancelled { mark("xmark", colour: stroke, in: rect) }
            }
            return true
        }
        image.accessibilityDescription = accessibilityDescription
        return image
    }

    private static func mark(_ name: String, colour: NSColor, in rect: NSRect) {
        let configuration = NSImage.SymbolConfiguration(
            pointSize: rect.width * CheckMetrics.mark, weight: .semibold
        ).applying(.init(paletteColors: [colour]))
        guard
            let symbol = NSImage(systemSymbolName: name, accessibilityDescription: nil)?
                .withSymbolConfiguration(configuration)
        else { return }
        let size = symbol.size
        symbol.draw(
            in: NSRect(
                x: rect.midX - size.width / 2, y: rect.midY - size.height / 2,
                width: size.width, height: size.height))
    }
}
