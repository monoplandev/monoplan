// Browse: the root page and the app's one navigation stack; there is
// no tab bar. First the pages that are not one list: Starred, Agenda
// and Done. Then a section per open workspace (Personal first, as
// `Engine.session` orders them), each with its Inbox and then its
// active lists in the doc's order, each led by its emoji (a folder
// when it has none); a list opens its Open items.
// The gear at the top right opens Debug, which stands in for settings
// until there are any. A floating bar (`BottomBar`: search and add,
// both inert so far) sits under every page. Every page is pushed from
// here, and item rows on any of them tick and open the item's sheet.
// Archived lists and the reserved `events` list (calendar-only,
// spec/events-plan.md) are left out.

import MonoplanCore
import MonoplanMacKit
import SwiftUI

/// One list in one workspace: what a Browse row opens.
struct ListRef: Hashable {
    let docId: String
    let listId: String
    let name: String
    /// The list's emoji, when it has one.
    var icon: String? = nil

    var key: String { "\(docId)/\(listId)" }
}

/// A list's icon: its emoji when the user picked one, else the built-in
/// glyph (a tray for the Inbox, a folder for any other list).
struct ListIcon: View {
    let listId: String
    let emoji: String?

    var body: some View {
        if let emoji, !emoji.isEmpty {
            Text(emoji)
        } else {
            Image(systemName: listId == "inbox" ? "tray" : "folder")
        }
    }
}

struct BrowseView: View {
    @EnvironmentObject private var model: AppModel

    private enum Page: Hashable {
        case starred
        case agenda
        case done
        case debug
    }

    var body: some View {
        NavigationStack {
            List {
                Section {
                    NavigationLink(value: Page.starred) {
                        Label("Starred", systemImage: "star")
                    }
                    NavigationLink(value: Page.agenda) {
                        Label("Agenda", systemImage: "calendar")
                    }
                    NavigationLink(value: Page.done) {
                        Label("Done", systemImage: "checkmark.circle")
                    }
                }
                ForEach(model.session?.docs ?? [], id: \.docId) { doc in
                    Section(doc.name ?? "Personal") {
                        row(ListRef(docId: doc.docId, listId: "inbox", name: "Inbox"))
                        ForEach(Lists.active(model.listsByDoc[doc.docId] ?? [])) { list in
                            row(ListRef(docId: doc.docId, listId: list.id, name: list.name, icon: list.icon))
                        }
                    }
                }
                if model.session == nil, case .connecting(let why) = model.clientState {
                    Section { Text(why).foregroundStyle(.secondary) }
                }
            }
            .listStyle(.insetGrouped)
            .navigationTitle("Browse")
            .toolbar {
                ToolbarItem(placement: .topBarTrailing) {
                    NavigationLink(value: Page.debug) {
                        Label("Settings", systemImage: "gearshape")
                    }
                }
            }
            .navigationDestination(for: ListRef.self) { ListItemsView(list: $0) }
            .navigationDestination(for: Page.self) { page in
                switch page {
                case .starred: StarredView()
                case .agenda: AgendaView()
                case .done: DoneView()
                case .debug: DebugView()
                }
            }
            .refreshable { model.loadSession() }
            .onAppear { model.loadSession() }
        }
        .safeAreaInset(edge: .bottom, spacing: 0) { BottomBar() }
        .opensItems()
    }

    private func row(_ ref: ListRef) -> some View {
        NavigationLink(value: ref) {
            Label {
                Text(ref.name)
            } icon: {
                ListIcon(listId: ref.listId, emoji: ref.icon)
            }
        }
    }
}

/// One list's Open items, in list order, for any list in any workspace
/// (an Inbox included). Refetched on every doc change while shown.
struct ListItemsView: View {
    @EnvironmentObject private var model: AppModel
    let list: ListRef

    var body: some View {
        Group {
            if let items = model.listItems[list.key], !items.isEmpty {
                List(items) { item in
                    ItemLink(ItemRef(docId: list.docId, itemId: item.id, title: item.text)) {
                        ItemRowView(item: item)
                    }
                }
                .listStyle(.plain)
                .animation(.default, value: items)
            } else {
                emptyState
            }
        }
        .navigationTitle(title)
        .navigationBarTitleDisplayMode(.inline)
        .refreshable { model.refreshList(list) }
        .onAppear { model.refreshList(list) }
        .onChange(of: model.changeCount) { model.refreshList(list) }
    }

    /// The header: the list's emoji, when it has one, before its name.
    private var title: String {
        guard let icon = list.icon, !icon.isEmpty else { return list.name }
        return "\(icon) \(list.name)"
    }

    private var emptyState: some View {
        ScrollView {
            if let error = model.listErrors[list.key] {
                ContentUnavailableView(
                    "\(list.name) unavailable",
                    systemImage: "exclamationmark.triangle",
                    description: Text(error)
                )
            } else if model.listItems[list.key] == nil {
                ProgressView().padding(.top, 80)
            } else {
                ContentUnavailableView(
                    "Nothing open",
                    systemImage: list.listId == "inbox" ? "tray" : "folder",
                    description: Text("Open items in \(list.name) show here.")
                )
            }
        }
    }
}
