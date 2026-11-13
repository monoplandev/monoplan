// Home: the root page and the app's one navigation stack; there is
// no tab bar. First the pages that are not one list: Starred, Agenda
// and Done. Then a section per open workspace (Personal first, as
// `Engine.session` orders them), each with its Inbox and then its
// active lists in the doc's order, each led by its emoji (a folder
// when it has none); a list opens its Open items.
// The gear at the top right opens Settings (account and sync, with
// Debug a level under it). A floating bar (`BottomBar`: search, and an
// add button that captures a new item into the list being shown, else
// the Personal Inbox) sits under every page; while its field has
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

/// The web's shared glyph (`js/web/src/icons/users.svg`, in the asset
/// catalog as `Users`: two people, one behind the other), sized to sit
/// beside footnote text: a little over its cap height, and growing
/// with it under Dynamic Type. Takes the foreground style around it.
struct SharedIcon: View {
    @ScaledMetric(relativeTo: .footnote) private var side: CGFloat = 15

    var body: some View {
        Image("Users")
            .resizable()
            .scaledToFit()
            .frame(width: side, height: side)
            .accessibilityHidden(true)
    }
}

/// What marks a list as a shared workspace's wherever it is named away
/// from Home's sections (the item page's list field, the list editor's
/// rows): the shared glyph and the workspace's name, muted, after the list's own name. Draws nothing for
/// the Personal workspace (`AppModel.sharedWorkspaceName` is nil). It
/// is the first thing to truncate when the row is short of room.
struct SharedWorkspaceBadge: View {
    let name: String?

    var body: some View {
        if let name {
            HStack(spacing: 4) {
                SharedIcon()
                Text(name).lineLimit(1)
            }
            .font(.footnote)
            .foregroundStyle(.secondary)
            .layoutPriority(-1)
            .accessibilityElement(children: .ignore)
            .accessibilityLabel("Shared workspace \(name)")
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

/// The web's star (`js/web/src/icons/star.svg`, in the asset catalog as
/// `Star`), sized to sit where a symbol would as a label's icon.
struct StarIcon: View {
    var size: CGFloat = 22

    var body: some View {
        Image("Star")
            .resizable()
            .scaledToFit()
            .frame(width: size, height: size)
    }
}

struct HomeView: View {
    @EnvironmentObject private var model: AppModel
    @State private var path = NavigationPath()
    /// The bottom bar's search: what is typed, and whether the field
    /// has focus.
    @State private var query = ""
    @FocusState private var searching: Bool
    /// The list page on screen, if one is: where the add button
    /// captures to.
    @State private var shownList: ListRef?

    private var searchActive: Bool { searching || !query.isEmpty }

    var body: some View {
        NavigationStack(path: $path) {
            List {
                Section {
                    NavigationLink(value: HomePage.starred) {
                        Label { Text("Starred") } icon: { StarIcon() }
                    }
                    NavigationLink(value: HomePage.agenda) {
                        Label("Calendar", systemImage: "calendar")
                    }
                    NavigationLink(value: HomePage.done) {
                        Label("Done", systemImage: "checkmark.circle")
                    }
                }
                ForEach(model.session?.docs ?? [], id: \.docId) { doc in
                    Section {
                        row(ListRef(docId: doc.docId, listId: "inbox", name: "Inbox"))
                        ForEach(Lists.active(model.listsByDoc[doc.docId] ?? [])) { list in
                            row(ListRef(docId: doc.docId, listId: list.id, name: list.name, icon: list.icon))
                        }
                    } header: {
                        // A shared workspace's title is led by the
                        // shared glyph, as its lists' badges are.
                        if let shared = model.sharedWorkspaceName(doc.docId) {
                            HStack(spacing: 4) {
                                SharedIcon()
                                Text(shared)
                            }
                            .accessibilityElement(children: .ignore)
                            .accessibilityLabel("\(shared), shared workspace")
                        } else {
                            Text(doc.name ?? "Personal")
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
            .navigationDestination(for: ListRef.self) { list in
                ListItemsView(list: list)
                    .onAppear { shownList = list }
                    .onDisappear { if shownList == list { shownList = nil } }
            }
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
            BottomBar(query: $query, searching: $searching, captureList: shownList)
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

extension View {
    /// The small navigation title drawn by hand, for a count beside it
    /// in a regular weight: "-" for none, a "+" after it when `more`
    /// rows are still to load, nothing while `count` is nil (not
    /// loaded). Goes with `navigationTitle`, which still names the
    /// page for the back button. `icon` goes before the title: the
    /// page's icon from the Home menu, in the menu's tint. `workspace`
    /// goes between the icon and the title for a list of a shared
    /// workspace, as the web's header has it ("Household > Groceries"):
    /// the workspace's name, muted so the list's leads, then a chevron.
    /// It is the first thing to truncate when the bar is short of room.
    func countedTitle(_ title: String, count: Int?, more: Bool = false) -> some View {
        countedTitle(title, count: count, more: more) { EmptyView() }
    }

    func countedTitle<Icon: View>(
        _ title: String, count: Int?, more: Bool = false, workspace: String? = nil,
        @ViewBuilder icon: () -> Icon
    ) -> some View {
        let icon = icon()
        return navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .principal) {
                    HStack(spacing: 6) {
                        icon
                            .font(.headline)
                            .foregroundStyle(.tint)
                            .accessibilityHidden(true)
                        HStack(alignment: .firstTextBaseline, spacing: 6) {
                            if let workspace {
                                HStack(alignment: .firstTextBaseline, spacing: 4) {
                                    Text(workspace)
                                        .font(.headline)
                                        .lineLimit(1)
                                    Image(systemName: "chevron.right")
                                        .font(.caption.weight(.semibold))
                                        .accessibilityHidden(true)
                                }
                                .foregroundStyle(.secondary)
                                .layoutPriority(-1)
                            }
                            Text(title)
                                .font(.headline)
                                .lineLimit(1)
                            if let count {
                                Text(count == 0 ? "-" : "\(count)\(more ? "+" : "")")
                                    .font(.body.monospacedDigit())
                                    .foregroundStyle(.secondary)
                                    .layoutPriority(1)
                            }
                        }
                    }
                    .accessibilityElement(children: .combine)
                    .accessibilityAddTraits(.isHeader)
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
        // The list's icon as Home's menu shows it (emoji, the Inbox's
        // tray, else a folder), its name, and how many Open items it
        // has, once they have loaded; a shared workspace's list has the
        // workspace's name before its own.
        .countedTitle(
            list.name, count: model.listItems[list.key]?.count,
            workspace: model.sharedWorkspaceName(list.docId)
        ) {
            ListIcon(listId: list.listId, emoji: list.icon)
        }
        .refreshable { model.refreshList(list) }
        .onAppear { model.refreshList(list) }
        .onChange(of: model.changeCount) { model.refreshList(list) }
    }

    /// The page's name for the back button: the list's emoji, when it
    /// has one, before its name.
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
