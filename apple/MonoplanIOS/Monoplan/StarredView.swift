// The Starred page, pushed from Home (which owns the navigation stack
// and the item sheet): the Focus lens (spec/focus.md; "Starred" is the UI
// name) as the daemon resolves it with `starred --all`: Home's curated
// order, entries from other workspaces included, each row naming its
// list and, when not Personal's, its workspace. The order is the
// position; no number is shown. A row's checkbox ticks the item
// (which drops it from Starred); the row opens its sheet.

import MonoplanMacKit
import SwiftUI

struct StarredView: View {
    @EnvironmentObject private var model: AppModel

    var body: some View {
        Group {
            if model.starred.isEmpty {
                emptyState
            } else {
                List(model.starred, id: \.id) { row in
                    ItemLink(ItemRef(docId: row.docId, itemId: row.id, title: row.text)) {
                        StarredRowView(row: row)
                    }
                }
                .listStyle(.plain)
                .animation(.default, value: model.starred)
            }
        }
        .navigationTitle("Starred")
        .refreshable { model.refreshStarred() }
        .onAppear { model.refreshStarred() }
    }

    private var emptyState: some View {
        ScrollView {
            if let error = model.starredError {
                ContentUnavailableView(
                    "Starred unavailable",
                    systemImage: "exclamationmark.triangle",
                    description: Text(error)
                )
            } else if model.sync == nil, case .connecting(let why) = model.clientState {
                ContentUnavailableView("Starred", systemImage: "star", description: Text(why))
            } else {
                ContentUnavailableView(
                    "Nothing starred",
                    systemImage: "star",
                    description: Text("Items you star show here, in your order.")
                )
            }
        }
    }
}

/// One starred row: the text, and underneath the list it lives in with
/// the workspace after it when that is not Personal.
struct StarredRowView: View {
    let row: FocusRow

    var body: some View {
        CheckboxRow(state: row.state) {
            VStack(alignment: .leading, spacing: 2) {
                Text(row.text)
                    .itemTextSlot()
                if let note {
                    Text(note)
                        .font(.caption)
                        .foregroundStyle(.secondary)
                }
            }
        }
    }

    private var note: String? {
        let parts = [row.listName, row.docName].compactMap { $0 }
        return parts.isEmpty ? nil : parts.joined(separator: " · ")
    }
}
