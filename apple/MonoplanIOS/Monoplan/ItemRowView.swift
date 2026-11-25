// The row every plain item list shows (a list's page in Home): the
// checkbox, the text, and the tags worth saying.

import MonoplanCore
import SwiftUI

/// One item: its text, with the workflow state underneath
/// when there is something beyond the default to say, and the
/// workspace it is in when the page mixes several (the Inbox's).
struct ItemRowView: View {
    let item: Item
    var workspace: String? = nil

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
        if item.state != .backlog && item.state != .todo {
            out.append(item.state.label)
        }
        if let workspace { out.append(workspace) }
        return out
    }
}
