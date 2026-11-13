// The checkbox an item row leads with: an empty rounded square while the
// item is open, filled and ticked once Done, crossed when Cancelled
// (spec/data-model.md "Lifecycle"). `ItemCheckbox` is the drawing,
// `ItemCheckButton` the control that ticks and reopens.

import SwiftUI
import UIKit

struct ItemCheckbox: View {
    /// The workflow state name, as the engine's JSON carries it.
    let state: String

    /// Drawn rather than an SF Symbol, whose corner radius and stroke
    /// are not ours to set. Sized with the text (Dynamic Type).
    @ScaledMetric(relativeTo: .body) private var side: CGFloat = 20
    private static let radius: CGFloat = 0.32
    private static let border: CGFloat = 1.25
    private static let nudge: CGFloat = 1.5

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
        // is level with the middle of the first line's capitals, then a
        // touch lower, where it looks centred on the line as a whole.
        .alignmentGuide(.firstTextBaseline) { $0[.bottom] - side * 0.2 - Self.nudge }
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

/// The checkbox as a control: a tap ticks an open item and reopens a
/// closed one. The hit area is larger than the drawn box.
struct ItemCheckButton: View {
    @EnvironmentObject private var model: AppModel
    let ref: ItemRef
    let state: String

    private var closed: Bool { state == "done" || state == "cancelled" }

    var body: some View {
        Button {
            UIImpactFeedbackGenerator(style: .light).impactOccurred()
            model.setDone(ref, done: !closed)
        } label: {
            ItemCheckbox(state: state)
                .padding(10)
                .contentShape(Rectangle())
                .padding(-10)
        }
        .buttonStyle(.plain)
        .accessibilityLabel(state == "cancelled" ? "Reopen" : closed ? "Mark not done" : "Mark done")
    }
}

/// An item row's layout: the checkbox, then whatever the row says,
/// aligned to its first line. Two taps: the checkbox ticks, the rest of
/// the row opens the item's sheet. The item is the enclosing
/// `ItemLink`'s; without one the row is inert.
struct CheckboxRow<Content: View>: View {
    @Environment(\.itemRef) private var ref
    @Environment(\.openItem) private var openItem
    let state: String
    @ViewBuilder let content: Content

    var body: some View {
        HStack(alignment: .firstTextBaseline, spacing: 12) {
            if let ref {
                ItemCheckButton(ref: ref, state: state)
            } else {
                ItemCheckbox(state: state)
            }
            content
                .frame(maxWidth: .infinity, alignment: .leading)
                .contentShape(Rectangle())
                .onTapGesture { if let ref { openItem(ref) } }
                .accessibilityAddTraits(.isButton)
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
