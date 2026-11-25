// The Inbox page, pushed from Home (which owns the navigation stack
// and the item sheet): every open workspace's Inbox as one queue
// (`Engine.inbox`, spec/inbox-plan.md "One Inbox"). Each doc keeps its
// own Inbox; this is a read across them, as Assigned is. Oldest first,
// or newest first as this device prefers (the "..." menu's Order,
// `inboxNewestFirstKey`). The Inbox is a queue in arrival
// order, so its rows are never dragged to a new place.
// With more than one workspace open, the filter at the top right
// narrows the rows: "All Workspaces" (the default), or any number of
// workspaces checked at once (a device preference, `inboxFilterKey`;
// one no longer open is ignored). The engine applies it
// (`Engine.inbox(filter:)`), so a change reads the rows again. The
// title never names a workspace: it is one Inbox. A row of a
// shared workspace names it at its trailing edge. The bottom bar's
// add button captures into Personal's Inbox while Personal is shown,
// else the first shown workspace's (`HomeView`). A row's checkbox ticks the item
// (which drops it from the page); the row opens its sheet.
// A row swiped left is selected and the page is then selecting, as a
// list's page is (`ListItemsView`): the picked rows are starred or
// binned (asked first) whatever their workspace, and moved to another
// list only while they are all of one.

import MonoplanCore
import SwiftUI

/// `UserDefaults` key of the Inbox page's workspace filter: the picked
/// doc ids comma-separated, or absent / empty for every workspace
/// (`AppModel.inboxFilterIds`).
let inboxFilterKey = "inboxFilter"

struct InboxView: View {
    @EnvironmentObject private var model: AppModel
    /// The page is selecting rows (the list's edit mode). Home's, so
    /// its bottom bar can make way for the selection's toolbar.
    @Binding var selecting: Bool

    /// The device's Inbox order (`inboxNewestFirstKey`): reverses the
    /// rows on screen, never a doc.
    @AppStorage(inboxNewestFirstKey) private var inboxNewestFirst = false
    /// The workspace filter as stored; the engine applies it.
    @AppStorage(inboxFilterKey) private var storedFilter = ""

    /// The keys of the rows picked while selecting.
    @State private var selection: Set<String> = []
    /// The list picker is up, for where the picked rows move to.
    @State private var pickingList = false
    /// "Move to Bin" for the picked rows is waiting on its confirmation.
    @State private var confirmingBin = false

    /// The open workspaces picked; none is All Workspaces.
    private var picked: [String] { model.inboxPicked(storedFilter) }

    /// The rows in the order shown: as the engine narrowed and merged
    /// them, newest first when the device says so.
    private var rows: [InboxEntry] {
        inboxNewestFirst ? Array(model.inbox.reversed()) : model.inbox
    }

    /// A row names its workspace when that is a shared one; a
    /// Personal row names nothing.
    private func workspaceName(_ entry: InboxEntry) -> String? {
        model.sharedWorkspaceName(model.docId(of: entry))
    }

    /// The picked rows, in the order shown.
    private var pickedRows: [ItemRef] {
        rows.filter { selection.contains($0.key) }.map(\.ref)
    }

    /// The one workspace every picked row is in, when there is one:
    /// a move stays inside a workspace.
    private var pickedDoc: String? {
        let docs = Set(rows.filter { selection.contains($0.key) }.map { model.docId(of: $0) })
        return docs.count == 1 ? docs.first : nil
    }

    /// A capture into an Inbox, or "Show in List" on an Inbox item,
    /// of a workspace the filter hides: show every workspace again, so
    /// its row can be scrolled to (`showCaptured`).
    private func unhideCaptured() {
        guard let captured = model.captured, captured.list.listId == "inbox",
              !model.inboxShown(storedFilter).contains(captured.list.docId)
        else { return }
        storedFilter = ""
    }

    /// Scroll the captured item's row into view once the reread has
    /// it. A capture lands at the end of the queue, so a last row is
    /// brought up from the bottom edge.
    private func showCaptured(_ proxy: ScrollViewProxy) {
        guard let captured = model.captured, captured.list.listId == "inbox" else { return }
        guard let row = rows.first(where: {
            $0.item.id == captured.itemId && model.docId(of: $0) == captured.list.docId
        }) else { return }
        let anchor: UnitPoint = rows.last?.key == row.key ? .bottom : .top
        withAnimation { proxy.scrollTo(row.key, anchor: anchor) }
        model.showedCaptured()
    }

    /// Start selecting, with this row picked (nil: none yet).
    private func select(_ key: String?) {
        withAnimation { selecting = true }
        guard let key else { return }
        selection = [key]
        // Again once the list is in edit mode, in case entering it
        // dropped what was picked before it.
        DispatchQueue.main.async { selection.insert(key) }
    }

    private func endSelecting() {
        withAnimation { selecting = false }
    }

    var body: some View {
        Group {
            if rows.isEmpty {
                emptyState
            } else {
                ScrollViewReader { proxy in
                    List(selection: $selection) {
                        ForEach(rows, id: \.key) { row in
                            ItemLink(row.ref) {
                                ItemRowView(item: row.item, workspace: workspaceName(row))
                            }
                            // While selecting, a tap anywhere on the
                            // row is the list's: it picks the row.
                            .allowsHitTesting(!selecting)
                            .swipeActions(edge: .trailing, allowsFullSwipe: true) {
                                Button {
                                    select(row.key)
                                } label: {
                                    Label("Select", systemImage: "checkmark.circle")
                                }
                                .tint(.accentColor)
                            }
                        }
                    }
                    .listStyle(.plain)
                    .environment(\.editMode, .constant(selecting ? .active : .inactive))
                    .animation(.default, value: rows)
                    .onChange(of: rows, initial: true) { showCaptured(proxy) }
                    .onChange(of: model.captured) { showCaptured(proxy) }
                }
            }
        }
        .navigationTitle("Inbox")
        .countedTitle(
            String(localized: "Inbox"), count: model.inboxError == nil ? rows.count : nil
        ) {
            CatalogIcon(name: "Inbox")
        }
        .navigationBarBackButtonHidden(selecting)
        .toolbar {
            if selecting {
                selectionToolbar
            } else {
                ToolbarItemGroup(placement: .topBarTrailing) {
                    if model.doneWorkspaceOptions.count > 1 {
                        filterMenu
                    }
                    Menu {
                        Button {
                            select(nil)
                        } label: {
                            Label("Select Items", systemImage: "checkmark.circle")
                        }
                        .disabled(rows.isEmpty)
                        Picker("Order", selection: $inboxNewestFirst) {
                            Label("Oldest First", systemImage: "arrow.down").tag(false)
                            Label("Newest First", systemImage: "arrow.up").tag(true)
                        }
                        .pickerStyle(.inline)
                    } label: {
                        Label("Inbox options", systemImage: "ellipsis")
                    }
                }
            }
        }
        .confirmationDialog(
            "Move the selected items to the Bin?", isPresented: $confirmingBin, titleVisibility: .visible
        ) {
            Button("Move to Bin", role: .destructive) {
                model.binItems(pickedRows)
                endSelecting()
            }
        } message: {
            Text("They can be restored from the Bin.")
        }
        // Where the picked rows go: the item page's list editor, over
        // their workspace's lists.
        .sheet(isPresented: $pickingList) {
            if let docId = pickedDoc {
                NavigationStack {
                    ItemListEditor(
                        itemTitle: nil, lists: model.lists(inDoc: docId), current: "inbox",
                        workspace: model.sharedWorkspaceName(docId),
                        pick: { target in
                            guard target.listId != "inbox" else { return }
                            model.moveItems(pickedRows.map(\.itemId), inDoc: docId, toList: target.listId)
                            endSelecting()
                        },
                        done: { pickingList = false })
                }
                .environmentObject(model)
            }
        }
        .onChange(of: model.captured, initial: true) { unhideCaptured() }
        .onChange(of: storedFilter) { model.refreshInbox() }
        .refreshable { model.refreshInbox() }
        .onAppear { model.refreshInbox() }
        // Nothing stays picked once selecting is over, and a page that
        // goes away gives Home its bottom bar back.
        .onChange(of: selecting) { if !selecting { selection = [] } }
        .onDisappear { selecting = false }
    }

    /// The filter button: "All Workspaces", then a check per open
    /// workspace (Personal first), any number at once; the menu stays
    /// open while checks change. Tinted while narrowed, as the
    /// Calendar's is.
    private var filterMenu: some View {
        let picked = picked
        let narrowed = !picked.isEmpty
        return Menu {
            Toggle(isOn: Binding(get: { !narrowed }, set: { _ in storedFilter = "" })) {
                Text("All Workspaces")
            }
            Section {
                ForEach(model.doneWorkspaceOptions) { workspace in
                    Toggle(isOn: Binding(
                        get: { picked.contains(workspace.docId) },
                        set: { _ in storedFilter = model.inboxFilterToggling(workspace.docId, in: storedFilter) }
                    )) {
                        Text(verbatim: workspace.name)
                    }
                }
            }
        } label: {
            Image(systemName: "line.3.horizontal.decrease")
                .fontWeight(narrowed ? .semibold : .regular)
                .foregroundStyle(narrowed ? AnyShapeStyle(.tint) : AnyShapeStyle(.primary))
        }
        .menuActionDismissBehavior(.disabled)
        .accessibilityLabel("Filter")
        .accessibilityValue(
            narrowed
                ? model.doneWorkspaceOptions.filter { picked.contains($0.docId) }.map(\.name)
                    .formatted(.list(type: .and))
                : String(localized: "All Workspaces"))
    }

    /// The bars while selecting: pick every row (or none) and Done at
    /// the top, and at the foot what is done to the picked rows.
    @ToolbarContentBuilder
    private var selectionToolbar: some ToolbarContent {
        let all = Set(rows.map(\.key))
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
                pickingList = true
            } label: {
                Label("Move", systemImage: "folder")
            }
            .disabled(pickedDoc == nil)
            Button {
                model.starItems(pickedRows)
                endSelecting()
            } label: {
                Label("Star", systemImage: "star")
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
            if let error = model.inboxError {
                ContentUnavailableView(
                    "\(String(localized: "Inbox")) unavailable",
                    systemImage: "exclamationmark.triangle",
                    description: Text(error)
                )
            } else if model.sync == nil, case .connecting(let why) = model.engineState {
                ContentUnavailableView("Inbox", systemImage: "tray", description: Text(why))
            } else {
                ContentUnavailableView(
                    "Nothing open",
                    systemImage: "tray",
                    description: Text("Open items in \(String(localized: "Inbox")) show here.")
                )
            }
        }
    }
}
