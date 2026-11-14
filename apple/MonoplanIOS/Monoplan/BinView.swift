// The Bin page, pushed from Home (which owns the navigation stack and
// the item sheet): every open workspace's binned items, newest binned
// first (`bin show --json` per doc, spec/cli.md §Bin), each row naming
// the list it would go back to and, when not Personal's, its
// workspace. A row opens the item's sheet, where it can be restored or
// deleted; a swipe or a long press does either from the list. "Empty"
// deletes the lot. Both deletes are for good, so both ask first.

import MonoplanMacKit
import SwiftUI

struct BinView: View {
    @EnvironmentObject private var model: AppModel
    @Environment(\.dismiss) private var dismiss
    @Environment(\.openItem) private var openItem
    /// The row whose "Delete" is waiting on its confirmation.
    @State private var deleting: BinnedItem?
    @State private var confirmingEmpty = false

    var body: some View {
        Group {
            if model.binned.isEmpty {
                emptyState
            } else {
                List(model.binned) { item in
                    Button { openItem(item.ref) } label: {
                        BinnedRowView(
                            item: item,
                            listName: model.listName(item.row.listId, inDoc: item.docId),
                            workspace: model.sharedWorkspaceName(item.docId))
                    }
                    .swipeActions(edge: .leading) {
                        restoreButton(item).tint(.accentColor)
                    }
                    // No full swipe: a delete here is for good.
                    .swipeActions(edge: .trailing, allowsFullSwipe: false) {
                        deleteButton(item).tint(.red)
                    }
                    .contextMenu {
                        restoreButton(item)
                        deleteButton(item)
                    }
                    // No dividers between item rows, as `CheckboxRow` has it.
                    .listRowSeparator(.hidden)
                }
                // Plain, so a row reads as a row rather than a tinted link.
                .buttonStyle(.plain)
                .listStyle(.plain)
                .animation(.default, value: model.binned)
            }
        }
        .navigationTitle("Bin")
        .countedTitle("Bin", count: model.binLoaded && model.binError == nil ? model.binned.count : nil) {
            Image(systemName: "trash")
        }
        .toolbar {
            ToolbarItem(placement: .topBarTrailing) {
                Button("Empty") { confirmingEmpty = true }
                    .disabled(model.binned.isEmpty)
            }
        }
        .confirmationDialog(
            "Delete this item for good?",
            isPresented: Binding(get: { deleting != nil }, set: { if !$0 { deleting = nil } }),
            titleVisibility: .visible,
            presenting: deleting
        ) { item in
            Button("Delete", role: .destructive) { model.deleteBinned(item.ref) }
        } message: { _ in
            Text("It is removed from every device and cannot be restored.")
        }
        .confirmationDialog(emptyTitle, isPresented: $confirmingEmpty, titleVisibility: .visible) {
            // Nothing is left to show: back to Home at once.
            Button("Empty Bin", role: .destructive) {
                model.emptyBin()
                dismiss()
            }
        } message: {
            Text("They are removed from every device and cannot be restored.")
        }
        .refreshable { model.refreshBin() }
        .onAppear { model.refreshBin() }
    }

    private var emptyTitle: String {
        let count = model.binned.count
        return count == 1 ? "Delete 1 item for good?" : "Delete all \(count) items for good?"
    }

    private func restoreButton(_ item: BinnedItem) -> some View {
        Button("Restore", systemImage: "arrow.uturn.backward") {
            model.setBinned(item.ref, binned: false)
        }
    }

    private func deleteButton(_ item: BinnedItem) -> some View {
        Button("Delete", systemImage: "trash", role: .destructive) { deleting = item }
    }

    private var emptyState: some View {
        ScrollView {
            if let error = model.binError {
                ContentUnavailableView(
                    "Bin unavailable",
                    systemImage: "exclamationmark.triangle",
                    description: Text(error)
                )
            } else if model.sync == nil, case .connecting(let why) = model.clientState {
                ContentUnavailableView("Bin", systemImage: "trash", description: Text(why))
            } else if !model.binLoaded {
                ProgressView().padding(.top, 80)
            } else {
                ContentUnavailableView(
                    "Bin is empty",
                    systemImage: "trash",
                    description: Text("Items you move to the bin show here until you delete them.")
                )
            }
        }
    }
}

/// One binned item: its text, and underneath the list it lives in, the
/// workspace when that is not Personal, and how long ago it was binned.
struct BinnedRowView: View {
    let item: BinnedItem
    let listName: String?
    let workspace: String?

    var body: some View {
        VStack(alignment: .leading, spacing: 2) {
            Text(item.row.text)
                .lineLimit(2)
                .truncationMode(.tail)
            Text(note)
                .font(.caption)
                .foregroundStyle(.secondary)
                .lineLimit(1)
        }
        .padding(.vertical, 2)
        .frame(maxWidth: .infinity, alignment: .leading)
        .contentShape(Rectangle())
    }

    private var note: String {
        let binnedAt = Date(timeIntervalSince1970: TimeInterval(item.row.binnedAt) / 1000)
        // A stamp a moment ahead of this clock (another device's) reads
        // as now, not as "in 2 seconds".
        let ago = min(binnedAt, Date()).formatted(.relative(presentation: .named))
        return ([listName, workspace].compactMap { $0 } + [ago]).joined(separator: " · ")
    }
}
