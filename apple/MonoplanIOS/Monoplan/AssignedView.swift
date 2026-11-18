// The Assigned page, pushed from Home (which owns the navigation stack
// and the item sheet): the open items assigned to this account in
// every open workspace (`Engine.assigned`, spec/sharing-plan.md
// "Assignment"), dated ones first by their earliest date, then the
// rest newest first. Each row names its date when it has one, its list
// and, when not Personal's, its workspace. A row's checkbox ticks the
// item (which drops it from the page); the row opens its sheet.

import MonoplanCore
import SwiftUI

struct AssignedView: View {
    @EnvironmentObject private var model: AppModel

    var body: some View {
        Group {
            if model.assigned.isEmpty {
                emptyState
            } else {
                List {
                    ForEach(model.assigned, id: \.key) { row in
                        ItemLink(row.ref) {
                            AssignedRowView(
                                item: row.item,
                                listName: model.listName(
                                    row.item.listId, inDoc: row.docId, dated: row.item.when != nil),
                                workspace: model.sharedWorkspaceName(row.docId))
                        }
                    }
                }
                .listStyle(.plain)
                .animation(.default, value: model.assigned)
            }
        }
        .navigationTitle("Assigned")
        .countedTitle(String(localized: "Assigned"), count: model.assignedError == nil ? model.assigned.count : nil) {
            Image(systemName: "person")
        }
        .refreshable { model.refreshAssigned() }
        .onAppear { model.refreshAssigned() }
    }

    private var emptyState: some View {
        ScrollView {
            if let error = model.assignedError {
                ContentUnavailableView(
                    "Assigned unavailable",
                    systemImage: "exclamationmark.triangle",
                    description: Text(error)
                )
            } else if model.sync == nil, case .connecting(let why) = model.engineState {
                ContentUnavailableView("Assigned", systemImage: "person", description: Text(why))
            } else {
                ContentUnavailableView(
                    "Nothing assigned",
                    systemImage: "person",
                    description: Text("Items assigned to you show here, from every workspace.")
                )
            }
        }
    }
}

/// One assigned row: the text, and underneath its date (the earlier of
/// its planned day and deadline) when it has one, the list it lives in
/// and, when that is not Personal's, its workspace.
struct AssignedRowView: View {
    let item: Item
    let listName: String?
    var workspace: String? = nil

    var body: some View {
        CheckboxRow(state: item.state) {
            VStack(alignment: .leading, spacing: 2) {
                Text(item.text)
                    .itemTextSlot()
                if let note {
                    Text(note)
                        .font(.caption)
                        .foregroundStyle(.secondary)
                        .lineLimit(1)
                }
            }
        }
    }

    private var note: String? {
        let day = item.assignedDate.map { WhenText.dayLabel(String($0.prefix(10))) }
        let parts = [day, listName, workspace].compactMap { $0 }
        return parts.isEmpty ? nil : parts.joined(separator: " · ")
    }
}
