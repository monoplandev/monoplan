// Home: the root page and the app's one navigation stack; there is
// no tab bar. First the pages that are not one list: Starred, Agenda
// and Done. Then a section per open workspace (Personal first, as
// `Engine.session` orders them), each with its Inbox and then its
// active lists in the doc's order, each led by its emoji (a folder
// when it has none); a list opens its Open items.
// The gear at the top right opens Settings (account and sync, with
// Debug a level under it). A floating bar (`BottomBar`: search, and an
// add button inert so far) sits under every page; while its field has
// focus or text, `SearchView` covers the stack. Every page is pushed
// from here, and item rows on any of them tick and open the item's sheet.
// Archived lists and the reserved `events` list (calendar-only,
// spec/events-plan.md) are left out.

import MonoplanCore
import MonoplanMacKit
import SwiftUI

/// One list in one workspace: what a Home row opens.
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

/// The pages Home pushes that are not one list.
enum HomePage: Hashable {
    case starred
    case agenda
    case done
    case settings
}

struct HomeView: View {
    @EnvironmentObject private var model: AppModel
    @State private var path = NavigationPath()
    /// The bottom bar's search: what is typed, and whether the field
    /// has focus.
    @State private var query = ""
    @FocusState private var searching: Bool

    private var searchActive: Bool { searching || !query.isEmpty }

    var body: some View {
        NavigationStack(path: $path) {
            List {
                Section {
                    NavigationLink(value: HomePage.starred) {
                        Label("Starred", systemImage: "star")
                    }
                    NavigationLink(value: HomePage.agenda) {
                        Label("Agenda", systemImage: "calendar")
                    }
                    NavigationLink(value: HomePage.done) {
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
            .navigationTitle("Home")
            .toolbar {
                ToolbarItem(placement: .topBarTrailing) {
                    NavigationLink(value: HomePage.settings) {
                        Label("Settings", systemImage: "gearshape")
                    }
                }
            }
            .navigationDestination(for: ListRef.self) { ListItemsView(list: $0) }
            .navigationDestination(for: HomePage.self) { page in
                switch page {
                case .starred: StarredView()
                case .agenda: AgendaView()
                case .done: DoneView()
                case .settings: SettingsView()
                }
            }
            .refreshable { model.loadSession() }
            .onAppear { model.loadSession() }
        }
        // Search covers the stack, navigation bar included, and sits
        // under the bar. A page or list result closes it and is pushed
        // onto whatever page search was started from.
        .overlay {
            if searchActive {
                SearchView(query: query) { pick in
                    query = ""
                    searching = false
                    switch pick {
                    case .page(let page): path.append(page)
                    case .list(let list): path.append(list)
                    }
                }
                .transition(.opacity)
            }
        }
        .animation(.default, value: searchActive)
        .onChange(of: query) { model.search(query) }
        .safeAreaInset(edge: .bottom, spacing: 0) {
            BottomBar(query: $query, searching: $searching)
        }
        // The inset on the stack does not reach the scroll views inside
        // it, so every page's content gets the bar's height as a bottom
        // margin: the last row scrolls clear of the bar.
        .contentMargins(.bottom, BottomBar.clearance, for: .scrollContent)
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
