// The checkbox an item row leads with: an empty rounded square while the
// item is open, filled and ticked once Done, crossed when Cancelled
// (spec/data-model.md "Lifecycle"). Display only for now: ticking is
// the next surface, and this is where its tap will land.

import SwiftUI

struct ItemCheckbox: View {
    /// The workflow state name, as the engine's JSON carries it.
    let state: String

    /// Drawn rather than an SF Symbol, whose corner radius and stroke
    /// are not ours to set. Sized with the text (Dynamic Type).
    @ScaledMetric(relativeTo: .body) private var side: CGFloat = 20
    private static let radius: CGFloat = 0.32
    private static let border: CGFloat = 1.25

    var body: some View {
        let shape = RoundedRectangle(cornerRadius: side * Self.radius, style: .continuous)
        ZStack {
            if state == "done" {
                shape.fill(.tint)
                mark("checkmark").foregroundStyle(.white)
            } else {
                shape.strokeBorder(.tertiary, lineWidth: Self.border)
                if state == "cancelled" {
                    mark("xmark").foregroundStyle(.tertiary)
                }
            }
        }
        .frame(width: side, height: side)
        // A shape has no baseline of its own: sit the box so its middle
        // is level with the middle of the first line's capitals.
        .alignmentGuide(.firstTextBaseline) { $0[.bottom] - side * 0.2 }
        .accessibilityElement()
        .accessibilityLabel(label)
    }

    private func mark(_ name: String) -> some View {
        Image(systemName: name)
            .font(.system(size: side * 0.55, weight: .semibold))
    }

    private var label: String {
        switch state {
        case "done": return "Done"
        case "cancelled": return "Cancelled"
        default: return "Not done"
        }
    }
}

/// An item row's layout: the checkbox, then whatever the row says,
/// aligned to its first line.
struct CheckboxRow<Content: View>: View {
    let state: String
    @ViewBuilder let content: Content

    var body: some View {
        HStack(alignment: .firstTextBaseline, spacing: 12) {
            ItemCheckbox(state: state)
            content
        }
        .padding(.vertical, 2)
        // A list starts its separator at the row's first text by
        // default, which leaves the checkbox hanging outside the line;
        // start it at the row's leading edge so it spans the checkbox.
        .alignmentGuide(.listRowSeparatorLeading) { $0[.leading] }
    }
}

extension View {
    /// An item's text in a row: a slot of exactly two lines, kept even
    /// when the text needs one so rows share a height, with anything
    /// past the second line cut off by an ellipsis.
    func itemTextSlot() -> some View {
        lineLimit(2, reservesSpace: true)
            .truncationMode(.tail)
    }
}
