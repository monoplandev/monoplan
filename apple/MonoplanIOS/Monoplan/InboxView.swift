// The Inbox tab: the primary capture list (`inbox`, spec/data-model.md)
// as the engine reports it, Open items in list order; a row opens the
// item's sheet (`ItemView`). Read-only for now:
// capture and lifecycle moves are the next surface.

import MonoplanMacKit
import SwiftUI

struct InboxView: View {
    @EnvironmentObject private var model: AppModel

    var body: some View {
        NavigationStack {
            Group {
                if model.inbox.isEmpty {
                    emptyState
                } else {
                    List(model.inbox) { item in
                        ItemLink(ItemRef(docId: nil, itemId: item.id, title: item.text)) {
                            ItemRowView(item: item)
                        }
                    }
                    .listStyle(.plain)
                }
            }
            .navigationTitle("Inbox")
            .refreshable { model.refreshInbox() }
            .onAppear { model.refreshInbox() }
        }
        .opensItems()
    }

    private var emptyState: some View {
        ScrollView {
            if let error = model.inboxError {
                ContentUnavailableView(
                    "Inbox unavailable",
                    systemImage: "exclamationmark.triangle",
                    description: Text(error)
                )
            } else if model.sync == nil, case .connecting(let why) = model.clientState {
                ContentUnavailableView("Inbox", systemImage: "tray", description: Text(why))
            } else {
                ContentUnavailableView(
                    "Inbox is empty",
                    systemImage: "tray",
                    description: Text("Captured items will show here.")
                )
            }
        }
    }
}

/// One item: its text, with the workflow state and any dates underneath
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
        if let when = item.when { out.append("@\(when)") }
        if let deadline = item.deadline { out.append("due \(deadline)") }
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
