// The Starred page, pushed from Home (which owns the navigation stack
// and the item sheet): the Focus lens (spec/focus.md; "Starred" is the UI
// name) as the engine resolves it with `starred --all`: Home's curated
// order, entries from other workspaces included, each row naming its
// list and, when not Personal's, its workspace. The order is the
// position; no number is shown, and a long press and drag changes it. A row's checkbox ticks the item
// (which drops it from Starred); the row opens its sheet.
// A row swiped left is selected and the page is then selecting, as a
// list's page is (`ListItemsView`); "Select Items" at the top right
// starts it with nothing picked. The picked rows are unstarred, or
// binned (asked first); they can be of any workspace, so there is no
// move.

import MonoplanCore
import SwiftUI

struct StarredView: View {
    @EnvironmentObject private var model: AppModel
    /// The page is selecting rows (the list's edit mode). Home's, so
    /// its bottom bar can make way for the selection's toolbar.
    @Binding var selecting: Bool

    /// The ids of the rows picked while selecting.
    @State private var selection: Set<String> = []
    /// "Move to Bin" for the picked rows is waiting on its confirmation.
    @State private var confirmingBin = false

    /// The picked rows, in Starred's order.
    private var picked: [ItemRef] {
        model.starred.filter { selection.contains($0.id) }
            .map { ItemRef(docId: $0.docId, itemId: $0.id, title: $0.text) }
    }

    /// Start selecting, with this row picked (nil: none yet).
    private func select(_ id: String?) {
        withAnimation { selecting = true }
        guard let id else { return }
        selection = [id]
        // Again once the list is in edit mode, in case entering it
        // dropped what was picked before it.
        DispatchQueue.main.async { selection.insert(id) }
    }

    private func endSelecting() {
        withAnimation { selecting = false }
    }

    var body: some View {
        Group {
            if model.starred.isEmpty {
                emptyState
            } else {
                // A long press and drag moves a row, as on a list's
                // page; the drop is Starred's order from then on. Not
                // while selecting, where every row would grow a handle.
                List(selection: $selection) {
                    ForEach(model.starred, id: \.id) { row in
                        ItemLink(ItemRef(docId: row.docId, itemId: row.id, title: row.text)) {
                            StarredRowView(row: row)
                        }
                        // While selecting, a tap anywhere on the row
                        // is the list's: it picks the row.
                        .allowsHitTesting(!selecting)
                        .swipeActions(edge: .trailing, allowsFullSwipe: true) {
                            Button {
                                select(row.id)
                            } label: {
                                Label("Select", systemImage: "checkmark.circle")
                            }
                            .tint(.accentColor)
                        }
                    }
                    .onMove(perform: selecting ? nil : { model.moveStarred(from: $0, to: $1) })
                }
                .listStyle(.plain)
                .environment(\.editMode, .constant(selecting ? .active : .inactive))
                .animation(.default, value: model.starred)
                .environment(\.duplicatesStarred, true)
            }
        }
        .navigationTitle("Starred")
        .countedTitle(String(localized: "Starred"), count: model.starredError == nil ? model.starred.count : nil) {
            StarIcon(size: 19)
        }
        .navigationBarBackButtonHidden(selecting)
        .toolbar {
            if selecting {
                selectionToolbar
            } else {
                ToolbarItem(placement: .topBarTrailing) {
                    Button {
                        select(nil)
                    } label: {
                        Label("Select Items", systemImage: "checkmark.circle")
                    }
                    .disabled(model.starred.isEmpty)
                }
            }
        }
        .confirmationDialog(
            "Move the selected items to the Bin?", isPresented: $confirmingBin, titleVisibility: .visible
        ) {
            Button("Move to Bin", role: .destructive) {
                model.binItems(picked)
                endSelecting()
            }
        } message: {
            Text("They can be restored from the Bin.")
        }
        .refreshable { model.refreshStarred() }
        .onAppear { model.refreshStarred() }
        // Nothing stays picked once selecting is over, and a page that
        // goes away gives Home its bottom bar back.
        .onChange(of: selecting) { if !selecting { selection = [] } }
        .onDisappear { selecting = false }
    }

    /// The bars while selecting: pick every row (or none) and Done at
    /// the top, and at the foot, as one group, what is done to the
    /// picked rows.
    @ToolbarContentBuilder
    private var selectionToolbar: some ToolbarContent {
        let all = Set(model.starred.map(\.id))
        let allPicked = !all.isEmpty && selection.isSuperset(of: all)
        ToolbarItem(placement: .topBarLeading) {
            Button(allPicked ? String(localized: "Deselect All") : String(localized: "Select All")) {
                selection = allPicked ? [] : all
            }
        }
        ToolbarItem(placement: .topBarTrailing) {
            Button("Done", systemImage: "checkmark", role: .confirm) { endSelecting() }
        }
        ToolbarSpacer(.flexible, placement: .bottomBar)
        ToolbarItemGroup(placement: .bottomBar) {
            Button {
                model.unstarItems(picked)
                endSelecting()
            } label: {
                Label("Unstar", systemImage: "star.slash")
            }
            .disabled(selection.isEmpty)
            Button(role: .destructive) {
                confirmingBin = true
            } label: {
                Label("Move to Bin", systemImage: "trash")
            }
            .disabled(selection.isEmpty)
        }
        ToolbarSpacer(.flexible, placement: .bottomBar)
    }

    private var emptyState: some View {
        ScrollView {
            if let error = model.starredError {
                ContentUnavailableView(
                    "Starred unavailable",
                    systemImage: "exclamationmark.triangle",
                    description: Text(error)
                )
            } else if model.sync == nil, case .connecting(let why) = model.engineState {
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
    let row: StarredItem

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
        let parts = [NoList.display(row.listName, listId: row.listId), row.docName].compactMap { $0 }
        return parts.isEmpty ? nil : parts.joined(separator: " · ")
    }
}
