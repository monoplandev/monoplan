// The checkbox an item row leads with: an empty rounded square while the
// item is open, filled and ticked once Done, crossed when Cancelled
// (spec/data-model.md "Lifecycle"). Display only for now: ticking is
// the next surface, and this is where its tap will land.

import SwiftUI

struct ItemCheckbox: View {
    /// The workflow state name, as the engine's JSON carries it.
    let state: String

    var body: some View {
        Image(systemName: symbol)
            .font(.title3)
            .foregroundStyle(state == "done" ? AnyShapeStyle(.tint) : AnyShapeStyle(.tertiary))
            .accessibilityLabel(label)
    }

    private var symbol: String {
        switch state {
        case "done": return "checkmark.square.fill"
        case "cancelled": return "xmark.square"
        default: return "square"
        }
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
