// The row every plain item list shows (a list's page in Home): the
// checkbox, the text, and the tags worth saying.

import MonoplanMacKit
import SwiftUI

/// One item: its text, with the workflow state underneath
/// when there is something beyond the default to say.
struct ItemRowView: View {
    let item: ItemRow

    var body: some View {
        CheckboxRow(state: item.state) {
            VStack(alignment: .leading, spacing: 4) {
                Text(item.text)
                    .itemTextSlot()
                if !tags.isEmpty {
                    Text(tags.joined(separator: "  ·  "))
                        .font(.footnote)
                        .foregroundStyle(.secondary)
                }
            }
        }
    }

    private var tags: [String] {
        var out: [String] = []
        if item.state != "backlog" && item.state != "todo" {
            out.append(Self.stateLabel(item.state))
        }
        return out
    }

    static func stateLabel(_ state: String) -> String {
        switch state {
        case "in_progress": return "In progress"
        case "review": return "Review"
        case "done": return "Done"
        case "cancelled": return "Cancelled"
        case "todo": return "To do"
        default: return "Backlog"
        }
    }
}
