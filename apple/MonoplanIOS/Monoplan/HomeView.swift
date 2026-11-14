// Home: the root page and the app's one navigation stack; there is
// no tab bar. First the pages that are not one list: Starred, Agenda,
// Map and Done (Starred with its count). Then a section per open workspace (Personal first, as
// `Engine.session` orders them), each with its Inbox (and how many
// Open items it holds) and then its
// active lists in the doc's order, each led by its emoji (a folder
// when it has none); a list opens its Open items.
// The gear at the top right opens Settings (account and sync, with
// Debug a level under it); while the profile has no account, a Sign In
// button sits at the top left and opens the sign-in sheet.
// A floating bar (`BottomBar`: search, and an
// add button that captures a new item into the list being shown, else
// the Personal Inbox) sits under every page but the Map, whose sheet
// has its own search; while its field has
// focus or text, `SearchView` covers the stack. Every page is pushed
// from here, and item rows on any of them tick and open the item's sheet.
// Archived lists and the reserved `events` list (calendar-only,
// spec/events-plan.md) are left out.

import MonoplanCore
import MonoplanMacKit
import SwiftUI

/// One list in one workspace: what a Home row opens.
/// `UserDefaults` key of the list-counts preference (`SettingsView`'s
/// toggle, read by Home).
let listCountsKey = "listCounts"

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

/// The web's place pin (`js/web/src/icons/map-pin.svg`, in the asset
/// catalog as `MapPin`), wherever an item's place is named: `size` is
/// its side beside body text (a field's icon) and grows with Dynamic
/// Type; rows that name the place in caption text pass a smaller one.
/// Takes the foreground style around it.
struct PinIcon: View {
    @ScaledMetric private var side: CGFloat

    init(size: CGFloat = 18) {
        _side = ScaledMetric(wrappedValue: size, relativeTo: .body)
    }

    var body: some View {
        Image("MapPin")
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
    case map
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
    @State private var showSignIn = false
    /// The Map page is up: its sheet stands where the bottom bar does.
    @State private var mapShown = false

    /// The device's list-counts preference (Settings), off by default:
    /// count each list's Open items on its row. The Inbox and Starred
    /// rows are counted either way.
    @AppStorage(listCountsKey) private var showListCounts = false

    private var searchActive: Bool { searching || !query.isEmpty }

    /// How many of today's rows Home shows before "Show all" (which
    /// pushes the Calendar page), so a backlog of overdue rows cannot
    /// push the lists off the screen.
    private static let todayLimit = 5

    /// What wants attention today, for the section under Home's menu:
    /// the agenda's Today group (overdue deadlines and tasks whose
    /// planned day has passed folded in, as on the Calendar page) less
    /// what is ticked, so the section shrinks as it is cleared and is
    /// not drawn on a clear day. Unscheduled events are not dated, so
    /// they stay on the Calendar page.
    private var today: [AgendaRow] {
        (model.agenda.first(where: { $0.today })?.rows ?? []).filter { !$0.isDone }
    }

    var body: some View {
        NavigationStack(path: $path) {
            List {
                Section {
                    NavigationLink(value: HomePage.starred) {
                        // How many are starred; nothing is drawn for none.
                        Label { Text("Starred") } icon: { StarIcon() }
                            .counted(model.starred.count)
                    }
                    NavigationLink(value: HomePage.agenda) {
                        Label("Calendar", systemImage: "calendar")
                    }
                    NavigationLink(value: HomePage.map) {
                        Label("Map", systemImage: "map")
                    }
                    NavigationLink(value: HomePage.done) {
                        Label("Done", systemImage: "checkmark.circle")
                    }
                }
                if !today.isEmpty {
                    Section("Today") {
                        ForEach(today.prefix(Self.todayLimit), id: \.id) { row in
                            AgendaRowView(row: row)
                        }
                        if today.count > Self.todayLimit {
                            NavigationLink(value: HomePage.agenda) {
                                Text("Show all \(today.count)")
                                    .foregroundStyle(.secondary)
                            }
                        }
                    }
                }
                ForEach(model.session?.docs ?? [], id: \.docId) { doc in
                    Section {
                        // The Inbox's Open items, counted; nothing is
                        // drawn for an empty one.
                        row(ListRef(docId: doc.docId, listId: "inbox", name: "Inbox"), count: model.inboxCount(doc.docId) ?? 0)
                        ForEach(Lists.active(model.listsByDoc[doc.docId] ?? [])) { list in
                            // Counted only when Settings says so
                            // (the web's "Show list counts in nav").
                            row(
                                ListRef(docId: doc.docId, listId: list.id, name: list.name, icon: list.icon),
                                count: showListCounts ? list.open ?? 0 : 0)
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
                // Signed out (no account, a local-only workspace
                // included): sign in from here, as from Settings.
                if !model.hasAccount {
                    ToolbarItem(placement: .topBarLeading) {
                        Button("Sign In") { showSignIn = true }
                            .disabled(model.busy != nil)
                    }
                }
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
                case .map:
                    PlacesMapView()
                        .onAppear { mapShown = true }
                        .onDisappear { mapShown = false }
                case .done: DoneView()
                case .settings: SettingsView()
                }
            }
            .refreshable {
                model.loadSession()
                model.refreshAgenda()
            }
            .onAppear {
                model.loadSession()
                model.refreshAgenda()
            }
            .animation(.default, value: today)
        }
        .sheet(isPresented: $showSignIn) {
            SignInView()
                .environmentObject(model)
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
        // A capture's item is shown where it landed: its list is pushed
        // unless that is the page already up (under the sheet, when
        // the capture stays open as the item's page), and the list
        // page scrolls to it.
        .onChange(of: model.captured) {
            guard let captured = model.captured, shownList?.key != captured.list.key else { return }
            query = ""
            searching = false
            path.append(captured.list)
        }
        .safeAreaInset(edge: .bottom, spacing: 0) {
            if !mapShown {
                BottomBar(query: $query, searching: $searching, captureList: shownList)
            }
        }
        // The inset on the stack does not reach the scroll views inside
        // it, so every page's content gets the bar's height as a bottom
        // margin: the last row scrolls clear of the bar.
        .contentMargins(.bottom, BottomBar.clearance, for: .scrollContent)
        .opensItems()
    }

    private func row(_ ref: ListRef, count: Int = 0) -> some View {
        NavigationLink(value: ref) {
            Label {
                Text(ref.name)
            } icon: {
                ListIcon(listId: ref.listId, emoji: ref.icon)
            }
            .counted(count)
        }
    }
}

private extension View {
    /// A Home row's label with a count at its trailing edge, inside the
    /// link so it sits before the chevron (`badge` draws after it).
    /// Nothing is drawn for zero.
    func counted(_ count: Int) -> some View {
        HStack {
            self
            if count > 0 {
                Spacer()
                Text("\(count)")
                    .monospacedDigit()
                    .foregroundStyle(.secondary)
            }
        }
    }
}

extension View {
    /// The small navigation title drawn by hand, for a count beside it
    /// in a regular weight: "-" for none, nothing while `count` is
    /// nil (not loaded). Goes with `navigationTitle`, which still names the
    /// page for the back button. `icon` goes before the title: the
    /// page's icon from the Home menu, in the menu's tint. `workspace`
    /// goes between the icon and the title for a list of a shared
    /// workspace, as the web's header has it ("Household > Groceries"):
    /// the workspace's name, muted so the list's leads, then a chevron.
    /// It is the first thing to truncate when the bar is short of room.
    func countedTitle(_ title: String, count: Int?) -> some View {
        countedTitle(title, count: count) { EmptyView() }
    }

    func countedTitle<Icon: View>(
        _ title: String, count: Int?, workspace: String? = nil,
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
                                Text(count == 0 ? "-" : "\(count)")
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

    private func showCaptured(in items: [ItemRow], _ proxy: ScrollViewProxy) {
        guard let captured = model.captured, captured.list.key == list.key,
              items.contains(where: { $0.id == captured.itemId })
        else { return }
        withAnimation { proxy.scrollTo(captured.itemId, anchor: .top) }
        model.showedCaptured()
    }

    var body: some View {
        Group {
            if let items = model.listItems[list.key], !items.isEmpty {
                ScrollViewReader { proxy in
                    List(items) { item in
                        ItemLink(ItemRef(docId: list.docId, itemId: item.id, title: item.text)) {
                            ItemRowView(item: item)
                        }
                    }
                    .listStyle(.plain)
                    .animation(.default, value: items)
                    // A capture into this list: bring its item into
                    // view once the reread has it.
                    .onChange(of: items, initial: true) { showCaptured(in: items, proxy) }
                    .onChange(of: model.captured) { showCaptured(in: items, proxy) }
                }
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
