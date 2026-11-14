// Home: the root page and the app's one navigation stack; there is
// no tab bar. First the pages that are not one list: Starred, Agenda,
// Map, Done and Bin (Starred with its count). Then a section per open workspace (Personal first, as
// `Engine.session` orders them), each with its Inbox (and how many
// Open items it holds) and then its
// active lists in the doc's order, each led by its emoji (a folder
// when it has none); a list opens its Open items.
// The gear at the top right opens Settings (account and sync, with
// Debug a level under it). With an account, the avatar at the top left
// is the account's menu: its email and New Workspace (named in an
// alert); a shared workspace's section header has a "..." menu with
// Delete Workspace, which asks for the name typed back, as the Mac
// app's does. The folder beside it makes a new list:
// a popover under it takes the name (and, with more than one workspace
// open, which one: Personal unless changed) and the new list's page
// is pushed. While the profile has no account, a Sign In
// button sits at the top left and opens the sign-in sheet.
// A floating bar (`BottomBar`: search, and an
// add button that captures a new item into the list being shown, else
// the Personal Inbox) sits under every page but the Map, whose sheet
// has its own search; while its field has
// focus or text, `SearchView` covers the stack. Every page is pushed
// from here, and item rows on any of them tick and open the item's sheet.
// A user-created list's page has a "..." menu at the top right: rename
// (an alert), choose emoji (the system emoji keyboard,
// `EmojiKeyboard.swift`) and archive (confirmed; the page pops).
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
    case bin
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
    /// The new-list popover is up.
    @State private var showNewList = false
    /// An action of Home's that failed; shown in an alert.
    @State private var failure: Failure?
    /// The New Workspace alert is up, over what is typed in it.
    @State private var namingWorkspace = false
    @State private var workspaceName = ""
    /// The workspace Delete is asking about, and what has been typed
    /// back of its name.
    @State private var deleting: Deleting?
    @State private var deleteTyped = ""

    struct Failure {
        let title: String
        let message: String
    }

    struct Deleting {
        let docId: String
        let name: String
    }

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
                    // Only while something is binned; search still
                    // reaches the empty page.
                    if !model.binned.isEmpty {
                        NavigationLink(value: HomePage.bin) {
                            Label("Bin", systemImage: "trash")
                        }
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
                        ForEach((model.listsByDoc[doc.docId] ?? []).userLists) { list in
                            // Counted only when Settings says so
                            // (the web's "Show list counts in nav").
                            row(
                                ListRef(docId: doc.docId, listId: list.id, name: list.name, icon: list.icon),
                                count: showListCounts ? Int(list.open) : 0)
                        }
                    } header: {
                        // A shared workspace's title is led by the
                        // shared glyph, as its lists' badges are.
                        if let shared = model.sharedWorkspaceName(doc.docId) {
                            HStack {
                                HStack(spacing: 4) {
                                    SharedIcon()
                                    Text(shared)
                                }
                                .accessibilityElement(children: .ignore)
                                .accessibilityLabel("\(shared), shared workspace")
                                Spacer()
                                // The workspace's own actions.
                                Menu {
                                    Button("Delete Workspace…", systemImage: "trash", role: .destructive) {
                                        deleteTyped = ""
                                        deleting = Deleting(docId: doc.docId, name: shared)
                                    }
                                } label: {
                                    Image(systemName: "ellipsis")
                                        .font(.body)
                                        .frame(width: 44, height: 28, alignment: .trailing)
                                        .contentShape(Rectangle())
                                        .accessibilityLabel("\(shared) options")
                                }
                            }
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
                } else if let session = model.session {
                    // Signed in: the account's menu stands there.
                    ToolbarItem(placement: .topBarLeading) {
                        Menu {
                            Section(session.email) {
                                Button("New Workspace…", systemImage: "plus") {
                                    workspaceName = ""
                                    namingWorkspace = true
                                }
                            }
                        } label: {
                            Label("Account", systemImage: "person.crop.circle")
                        }
                    }
                }
                if let session = model.session, !session.docs.isEmpty {
                    ToolbarItem(placement: .topBarTrailing) {
                        Button { showNewList = true } label: {
                            Label("New List", systemImage: "folder.badge.plus")
                        }
                        // A popover on the phone too, not the sheet a
                        // compact width would make of it.
                        .popover(isPresented: $showNewList) {
                            NewListPopover { name, docId in
                                showNewList = false
                                createList(name: name, in: docId)
                            }
                            .environmentObject(model)
                            .presentationCompactAdaptation(.popover)
                        }
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
                case .bin: BinView()
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
        .alert(
            failure?.title ?? "",
            isPresented: Binding(get: { failure != nil }, set: { if !$0 { failure = nil } })
        ) {
            Button("OK", role: .cancel) {}
        } message: {
            Text(failure?.message ?? "")
        }
        .alert("New Workspace", isPresented: $namingWorkspace) {
            TextField("Name", text: $workspaceName)
            Button("Cancel", role: .cancel) {}
            Button("Create") {
                let name = workspaceName.trimmingCharacters(in: .whitespacesAndNewlines)
                model.createWorkspace(name: name) { error in
                    if let error { failure = Failure(title: "Could not create workspace", message: error) }
                }
            }
            .disabled(workspaceName.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
        }
        // Cross-device and final, so a tap through is not enough: the
        // name is typed back first.
        .alert(
            "Delete “\(deleting?.name ?? "")”?",
            isPresented: Binding(get: { deleting != nil }, set: { if !$0 { deleting = nil } }),
            presenting: deleting
        ) { target in
            TextField("Type “\(target.name)” to confirm", text: $deleteTyped)
                .autocorrectionDisabled()
            Button("Cancel", role: .cancel) {}
            Button("Delete", role: .destructive) {
                model.deleteWorkspace(target.docId) { error in
                    if let error { failure = Failure(title: "Could not delete workspace", message: error) }
                }
            }
            .disabled(deleteTyped.trimmingCharacters(in: .whitespacesAndNewlines) != target.name)
        } message: { _ in
            Text("Its lists and items are removed from every device, and anyone it is shared with loses access. This cannot be undone.")
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

    /// Make the list and push its page; a failure is said in an alert.
    private func createList(name: String, in docId: String) {
        model.addList(name: name, inDoc: docId) { list, error in
            if let list {
                path.append(list)
            } else {
                failure = Failure(title: "Could not create list", message: error ?? "")
            }
        }
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

/// The new-list popover: the name, focused as it opens, under it a
/// row for the workspace the list goes in (Personal unless changed; not
/// drawn while only one is open; a shared one led by the shared glyph),
/// and Create at the bottom. Return creates too. `create`
/// gets the trimmed name and the workspace's doc id.
private struct NewListPopover: View {
    @EnvironmentObject private var model: AppModel
    let create: (String, String) -> Void

    @State private var name = ""
    /// The workspace picked; nil until one is, which reads as Personal.
    @State private var picked: String?
    @FocusState private var focused: Bool

    /// The shared glyph for a menu entry. A menu draws an asset image
    /// at its natural size (the SVG's, far too big), so it is drawn
    /// down to a symbol's size here, as a template the menu tints.
    private static let menuSharedIcon: UIImage = {
        let size = CGSize(width: 20, height: 20)
        guard let source = UIImage(named: "Users") else { return UIImage() }
        return UIGraphicsImageRenderer(size: size).image { _ in
            source.draw(in: CGRect(origin: .zero, size: size))
        }.withRenderingMode(.alwaysTemplate)
    }()

    private var docs: [(id: String, name: String, shared: Bool)] {
        (model.session?.docs ?? []).map {
            let shared = model.sharedWorkspaceName($0.docId)
            return ($0.docId, shared ?? $0.name ?? "Personal", shared != nil)
        }
    }

    private var docId: String? {
        picked ?? model.session?.primaryDocId ?? docs.first?.id
    }

    private var trimmed: String {
        name.trimmingCharacters(in: .whitespacesAndNewlines)
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            TextField("New list", text: $name)
                .font(.title3)
                .focused($focused)
                .submitLabel(.done)
                .onSubmit(submit)
            Divider()
            if docs.count > 1, let current = docs.first(where: { $0.id == docId }) {
                Menu {
                    Picker("Workspace", selection: Binding(get: { docId ?? "" }, set: { picked = $0 })) {
                        ForEach(docs, id: \.id) { doc in
                            if doc.shared {
                                Label { Text(doc.name) } icon: { Image(uiImage: Self.menuSharedIcon) }
                                    .tag(doc.id)
                            } else {
                                Text(doc.name).tag(doc.id)
                            }
                        }
                    }
                } label: {
                    HStack(spacing: 6) {
                        if current.shared { SharedIcon() }
                        Text(current.name).lineLimit(1)
                        Spacer()
                        Image(systemName: "chevron.up.chevron.down")
                            .font(.footnote)
                            .foregroundStyle(.secondary)
                    }
                    .contentShape(Rectangle())
                }
                .tint(.primary)
                .accessibilityLabel("Workspace, \(current.name)")
                Divider()
            }
            Button(action: submit) {
                Text("Create").frame(maxWidth: .infinity)
            }
            .buttonStyle(.borderedProminent)
            .disabled(trimmed.isEmpty)
        }
        .padding(16)
        .frame(width: 300)
        .onAppear { focused = true }
    }

    private func submit() {
        guard !trimmed.isEmpty, let docId else { return }
        create(trimmed, docId)
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
    @Environment(\.dismiss) private var dismiss
    let list: ListRef

    /// The rename alert is up, over what is typed in it.
    @State private var renaming = false
    @State private var renameText = ""
    /// The emoji keyboard is up, picking the list's emoji.
    @State private var pickingEmoji = false
    /// The user has no emoji keyboard to pick with; said in an alert.
    @State private var noEmojiKeyboard = false
    @State private var confirmArchive = false
    @State private var archiveError: String?

    /// The list as it is now: the page was pushed with the name and
    /// emoji of that moment, and either can be changed from here (or
    /// from another device) while it is up.
    private var current: ListRef {
        guard let row = model.listsByDoc[list.docId]?.first(where: { $0.id == list.listId }) else { return list }
        return ListRef(docId: list.docId, listId: list.listId, name: row.name, icon: row.icon)
    }

    /// Only a user-created list can be renamed, given an emoji or
    /// archived.
    private var editable: Bool { list.listId != "inbox" && list.listId != "events" }

    private func showCaptured(in items: [Item], _ proxy: ScrollViewProxy) {
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
                    // A long press and drag moves a row: the list parts
                    // to show where it will land (`onMove`), and the
                    // drop is the list's order from then on.
                    List {
                        ForEach(items) { item in
                            ItemLink(ItemRef(docId: list.docId, itemId: item.id, title: item.text)) {
                                ItemRowView(item: item)
                            }
                        }
                        .onMove { model.reorderItem(in: list, from: $0, to: $1) }
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
            current.name, count: model.listItems[list.key]?.count,
            workspace: model.sharedWorkspaceName(list.docId)
        ) {
            ListIcon(listId: list.listId, emoji: current.icon)
        }
        .toolbar {
            if editable {
                ToolbarItem(placement: .topBarTrailing) {
                    Menu {
                        Button {
                            renameText = current.name
                            renaming = true
                        } label: {
                            Label("Rename", systemImage: "pencil")
                        }
                        Button {
                            if EmojiKeyboard.available {
                                pickingEmoji = true
                            } else {
                                noEmojiKeyboard = true
                            }
                        } label: {
                            Label("Choose Emoji", systemImage: "face.smiling")
                        }
                        Divider()
                        Button { confirmArchive = true } label: {
                            Label("Archive", systemImage: "archivebox")
                        }
                    } label: {
                        Label("List options", systemImage: "ellipsis")
                    }
                }
            }
        }
        // The emoji keyboard's unseen field; a pick is written at once.
        .background {
            EmojiKeyboardField(isActive: $pickingEmoji, canRemove: !(current.icon ?? "").isEmpty) { emoji in
                model.setListIcon(list, icon: emoji)
            }
            .frame(width: 0, height: 0)
        }
        .alert("Rename List", isPresented: $renaming) {
            TextField("Name", text: $renameText)
            Button("Cancel", role: .cancel) {}
            Button("Rename") {
                let name = renameText.trimmingCharacters(in: .whitespacesAndNewlines)
                if name != current.name { model.renameList(list, to: name) }
            }
            .disabled(renameText.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
        }
        .alert("No emoji keyboard", isPresented: $noEmojiKeyboard) {
            Button("OK", role: .cancel) {}
        } message: {
            Text("Turn on the Emoji keyboard in Settings to choose an emoji.")
        }
        .confirmationDialog(
            "Archive \(current.name)?", isPresented: $confirmArchive, titleVisibility: .visible
        ) {
            Button("Archive") {
                model.archiveList(list) { error in
                    if let error { archiveError = error } else { dismiss() }
                }
            }
        } message: {
            Text("The list leaves Home. Its items are kept.")
        }
        .alert(
            "Could not archive list",
            isPresented: Binding(get: { archiveError != nil }, set: { if !$0 { archiveError = nil } })
        ) {
            Button("OK", role: .cancel) {}
        } message: {
            Text(archiveError ?? "")
        }
        .refreshable { model.refreshList(list) }
        .onAppear { model.refreshList(list) }
        .onChange(of: model.changeCount) { model.refreshList(list) }
    }

    /// The page's name for the back button: the list's emoji, when it
    /// has one, before its name.
    private var title: String {
        let list = current
        guard let icon = list.icon, !icon.isEmpty else { return list.name }
        return "\(icon) \(list.name)"
    }

    private var emptyState: some View {
        ScrollView {
            if let error = model.listErrors[list.key] {
                ContentUnavailableView(
                    "\(current.name) unavailable",
                    systemImage: "exclamationmark.triangle",
                    description: Text(error)
                )
            } else if model.listItems[list.key] == nil {
                ProgressView().padding(.top, 80)
            } else {
                ContentUnavailableView(
                    "Nothing open",
                    systemImage: list.listId == "inbox" ? "tray" : "folder",
                    description: Text("Open items in \(current.name) show here.")
                )
            }
        }
    }
}
