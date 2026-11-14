// The window: native SwiftUI over the engine's typed calls
// (spec/swift-bindings-plan.md "The macOS window"), replacing the
// webview. Deliberately a debug screen for now: a sidebar laid out as
// the web nav is (the lenses
// Starred, Calendar and Done on top, then one group per open workspace,
// Personal first, each a collapsible heading over its Inbox and lists),
// and either the open items of the selected list with a tick and a
// field to add one, or a lens over every workspace: Starred in its
// curated order, the Calendar (the engine's agenda, by day), or Done,
// newest first. A row of any of them can be selected, which opens the
// item's panel at the right (`ItemPanel.swift`): the iOS item page,
// read only but for its tick and star. No board, no drag and drop, no
// item editor; those come from sharing views with iOS.
//
// The chrome is the system's (`MainSplit.swift`): a split view of
// sidebar, list and the panel as inspector in a unified toolbar window.
// The bar shows the name of what is listed and, over the panel, the
// item's controls.
//
// It holds no doc and no state of its own beyond the selections: every
// read is a typed call, made again whenever the engine says a doc
// changed and after each of its own writes. Nothing is optimistic.
//
// While the window is open the app is a regular app (Dock icon, main
// menu); closed, it goes back to a bare status item.

import AppKit
import Combine
import SwiftUI
import MonoplanCore
import MonoplanMacKit

final class MainWindowController: NSWindowController, NSWindowDelegate {
    private let model: MainWindowModel
    private let engine: EngineController
    private let onVisibility: (Bool) -> Void
    /// Whether the profile has an account: asked when the window
    /// shows and when the engine's state moves (a sign-in or sign-out
    /// restarts the engine).
    private let signedIn: () -> Bool
    private var newWorkspacePrompt: NewWorkspacePrompt?
    private var modelSync: AnyCancellable?
    private let split: MainSplitViewController

    init(
        engine: EngineController, signedIn: @escaping () -> Bool, onSignIn: @escaping () -> Void,
        onVisibility: @escaping (Bool) -> Void
    ) {
        model = MainWindowModel(engine: engine)
        self.engine = engine
        self.signedIn = signedIn
        self.onVisibility = onVisibility
        // A toolbar window, as Finder or Notes: the tall unified bar
        // with the large traffic lights. The content view takes the
        // whole frame so the sidebar runs to the top edge under them.
        // The panes and the toolbar are `MainSplitViewController`'s.
        split = MainSplitViewController(model: model)
        split.signedIn = signedIn()
        split.onSignIn = onSignIn
        let window = NSWindow(contentViewController: split)
        window.title = "Monoplan"
        window.styleMask = [.titled, .closable, .miniaturizable, .resizable, .fullSizeContentView]
        window.toolbar = split.toolbar
        window.toolbarStyle = .unified
        window.isReleasedWhenClosed = false
        window.minSize = NSSize(width: 520, height: 360)
        window.setContentSize(NSSize(width: 860, height: 560))
        window.center()
        // After the default frame, so a saved one wins.
        window.setFrameAutosaveName("MonoplanWindow")
        super.init(window: window)
        window.delegate = self
        split.onNewWorkspace = { [weak self] in self?.promptNewWorkspace() }
        split.restoreDividers()
        // The bar's title is the window's, drawn over the list: the
        // name of what is shown. It and the panel's toolbar items
        // follow the model. `objectWillChange` is before the change,
        // hence the hop.
        modelSync = model.objectWillChange
            .receive(on: DispatchQueue.main)
            .sink { [weak self] in
                guard let self, let window = self.window else { return }
                let title = self.model.listTitle
                if window.title != title { window.title = title }
                self.split.sync()
            }
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }

    /// Bring the window forward, on a list when one is given (a tray
    /// row: the item's list in its workspace, nil being Home's), with
    /// the item's panel open when the row names one.
    func show(list: String? = nil, workspace: String? = nil, item: String? = nil) {
        onVisibility(true)
        model.visible = true
        syncSignedIn()
        if let list {
            model.select(workspace: workspace, list: list)
            if let item { model.selectItem(workspace: workspace, id: item) }
        } else {
            model.reload()
        }
        NSApp.activate(ignoringOtherApps: true)
        showWindow(nil)
        window?.makeKeyAndOrderFront(nil)
    }

    /// The engine's state moved (started, stopped, sync).
    func engineStateChanged(_ state: EngineState) {
        model.engineStateChanged(state)
        syncSignedIn()
    }

    /// The toolbar's Sign In, there while there is no account, and
    /// the account's menu in its place with one, headed by the email
    /// the engine gives.
    private func syncSignedIn() {
        let signedIn = signedIn()
        split.signedIn = signedIn
        if !signedIn { split.email = nil }
        split.sync()
        guard signedIn else { return }
        engine.calls?.run({ try $0.account() }) { [weak self] result in
            if case .success(let account) = result { self?.split.email = account?.email }
        }
    }

    /// A doc changed: read again what is shown.
    func docChanged() {
        model.reload()
    }

    /// Monoplan › New Workspace…: ask for a name on the window, have the
    /// engine create and open the doc, and show it. One prompt at a time.
    func promptNewWorkspace() {
        guard let window, newWorkspacePrompt == nil else { return }
        let prompt = NewWorkspacePrompt()
        newWorkspacePrompt = prompt
        prompt.run(on: window) { [weak self] name in
            guard let self else { return }
            self.newWorkspacePrompt = nil
            guard let name else { return }
            self.model.createWorkspace(name: name) { error in
                let alert = NSAlert()
                alert.alertStyle = .warning
                alert.messageText = String(localized: "Could not create the workspace")
                alert.informativeText = error
                alert.beginSheetModal(for: window)
            }
        }
    }

    func windowWillClose(_ notification: Notification) {
        model.visible = false
        onVisibility(false)
    }
}

/// One workspace's group in the sidebar: its heading and its lists.
struct NavSection: Identifiable {
    let workspace: WorkspaceInfo
    /// Every list of the workspace, archived ones included.
    let lists: [ListInfo]
    /// How many Open items its Inbox holds. The Inbox has no
    /// `ListMeta` row, so `lists` does not count it.
    let inboxOpen: Int

    var id: String { workspace.docId }

    /// What a typed call's `workspace` takes: Home is nil, the
    /// engine's default.
    var target: String? { workspace.home ? nil : workspace.docId }

    /// The lists shown under Inbox: active ones, in the
    /// doc's order, without the reserved ids (which have no `ListMeta`
    /// row of their own).
    var userLists: [ListInfo] {
        lists.filter {
            $0.archivedAt == nil && $0.id != MainWindowModel.inbox && $0.id != MainWindowModel.events
        }
    }
}

/// The views over every workspace, on top of the sidebar.
enum Lens: Hashable {
    case starred, agenda, done
}

/// What the list pane shows: a lens, or a list in a workspace.
struct Surface: Equatable {
    var lens: Lens?
    /// The workspace's doc id; nil is Home.
    var workspace: String?
    var list: String
}

/// A Done row: the item, tagged with the workspace it was read from.
struct DoneRow: Identifiable {
    let item: Item
    /// What a typed call's `workspace` takes: nil for Home.
    let workspace: String?
    /// The workspace's name; nil for Home and an unnamed doc.
    let workspaceName: String?

    var id: String { "\(workspace ?? "")/\(item.id)" }
}

/// The item whose panel is open. Rows are selected by item id alone
/// (an id is unique across workspaces: a cross-doc move keeps it); the
/// workspace is what the typed calls for it take.
struct ItemSelection: Equatable {
    let workspace: String?
    let id: String
}

/// A sidebar row: a lens, or a list in a workspace (nil being Home).
enum NavTarget: Hashable {
    case lens(Lens)
    case list(workspace: String?, list: String)
}

/// What the window shows. Main queue throughout; the reads run on the
/// engine's call queue and come back here.
final class MainWindowModel: ObservableObject {
    @Published private(set) var state: EngineState
    /// The sidebar: every open workspace with its lists, Home first.
    @Published private(set) var sections: [NavSection] = []
    /// The doc ids of the workspaces whose group is folded. Local to
    /// this Mac, kept across launches.
    @Published private(set) var collapsed: Set<String>
    /// The selected workspace's doc id; nil is Home.
    @Published private(set) var workspace: String?
    /// The selected list's id.
    @Published private(set) var list = MainWindowModel.inbox
    /// What the rows below were read for. The selection above
    /// (`lens`, `workspace`, `list`) moves at the click, for the
    /// sidebar; this follows once the new rows are read, so the pane
    /// keeps the old surface until then rather than blanking.
    @Published private(set) var shown = Surface(lens: nil, workspace: nil, list: MainWindowModel.inbox)
    @Published private(set) var items: [Item] = []
    /// The lens shown in place of a list, if any. The list selection
    /// under it is kept, though nothing shows it.
    @Published private(set) var lens: Lens?
    /// Starred's rows, always read: the sidebar counts them.
    @Published private(set) var starred: [StarredItem] = [] {
        didSet { starredIds = Set(starred.map(\.id)) }
    }
    /// Their ids, for the star on a starred item's row elsewhere.
    private(set) var starredIds: Set<String> = []
    /// The other lenses' rows, each read only while it is the one shown.
    @Published private(set) var agenda: [AgendaGroup] = []
    @Published private(set) var done: [DoneRow] = []
    /// The selected row's item, if one is: its panel is open.
    @Published private(set) var selected: ItemSelection?
    /// That item's page, once read. An item that cannot be read (hard
    /// deleted, its workspace gone) closes the panel.
    @Published private(set) var detail: ItemInfo?
    /// The last failed read or write, until the next one succeeds.
    @Published private(set) var error: String?

    /// Whether the window is open: a closed one reads nothing.
    var visible = false

    private let engine: EngineController
    /// Counts selections, so a read that was in flight when the
    /// selection moved is dropped rather than shown under the new one.
    private var generation = 0
    /// Set by a selection, taken by the read that follows it: unfold
    /// the group the selection is in. Only a selection does this, so a
    /// group folded by hand while on one of its lists stays folded.
    private var reveal = false

    private static let collapsedKey = "MonoplanNavCollapsed"
    static let inbox = "inbox"
    static let events = "events"
    /// `MONOPLAN_TIMING=1` logs each reload's timings to stderr.
    static let timing = ProcessInfo.processInfo.environment["MONOPLAN_TIMING"] != nil
    /// Far enough out to be the whole future, as on iOS.
    static let agendaDays: UInt32 = 36_500
    /// How many Done rows the lens shows, newest first.
    static let doneLimit = 200

    init(engine: EngineController) {
        self.engine = engine
        state = engine.state
        collapsed = Set(UserDefaults.standard.stringArray(forKey: Self.collapsedKey) ?? [])
    }

    /// The selected list's name, for the items' heading.
    var listTitle: String {
        let lens = shown.lens, workspace = shown.workspace, list = shown.list
        switch lens {
        case .starred: return String(localized: "Starred")
        case .agenda: return String(localized: "Calendar")
        case .done: return String(localized: "Done")
        case nil: break
        }
        switch list {
        case Self.inbox: return String(localized: "Inbox")
        default:
            let lists = sections.first { $0.target == workspace }?.lists ?? []
            // A list's emoji leads its name, as in the sidebar.
            guard let found = lists.first(where: { $0.id == list }) else { return list }
            return [found.icon, found.name].compactMap { $0 }.filter { !$0.isEmpty }.joined(separator: " ")
        }
    }

    func engineStateChanged(_ state: EngineState) {
        let wasRunning: Bool
        if case .connected = self.state { wasRunning = true } else { wasRunning = false }
        self.state = state
        // A new engine (a start, a sign-in or sign-out): everything
        // shown is the old one's. A sync report alone changes no doc.
        if case .connected = state, !wasRunning { reload() }
        if case .connecting = state { clear() }
    }

    func select(workspace: String?) {
        select(workspace: workspace, list: Self.inbox)
    }

    func select(list: String) {
        select(workspace: workspace, list: list)
    }

    /// Show a lens.
    func select(lens: Lens) {
        if lens == self.lens {
            reload()
            return
        }
        self.lens = lens
        clearItem()
        generation += 1
        reload()
    }

    /// Select a row's item, opening its panel; nil closes it.
    func selectItem(workspace: String?, id: String?) {
        guard let id else { return clearItem() }
        guard id != selected?.id else { return }
        selected = ItemSelection(workspace: workspace, id: id)
        detail = nil
        reload()
    }

    func closeItem() {
        clearItem()
    }

    private func clearItem() {
        selected = nil
        detail = nil
    }

    /// A shared workspace's name, for the panel; nil for Home, which
    /// a tray row names by its id.
    func sharedWorkspaceName(_ workspace: String?) -> String? {
        guard let section = sections.first(where: { $0.workspace.docId == workspace }),
              !section.workspace.home
        else { return nil }
        return section.workspace.name
    }

    func select(workspace: String?, list: String) {
        // Events surface only on the Calendar (spec/events-plan.md):
        // a tray row for one opens it rather than a bare list.
        if list == Self.events { return select(lens: .agenda) }
        reveal = true
        if lens == nil, workspace == self.workspace, list == self.list {
            reload()
            return
        }
        lens = nil
        clearItem()
        self.workspace = workspace
        self.list = list
        generation += 1
        reload()
    }

    /// Fold or unfold a workspace's group.
    func setCollapsed(_ docId: String, _ folded: Bool) {
        if folded { collapsed.insert(docId) } else { collapsed.remove(docId) }
        UserDefaults.standard.set(collapsed.sorted(), forKey: Self.collapsedKey)
    }

    private func clearLenses() {
        starred = []
        agenda = []
        done = []
    }

    private func clear() {
        sections = []
        items = []
        clearLenses()
        clearItem()
        generation += 1
    }

    private struct Snapshot {
        var sections: [NavSection]
        var lens: Lens?
        var workspace: String?
        var list: String
        var items: [Item]
        var starred: [StarredItem] = []
        var agenda: [AgendaGroup] = []
        var done: [DoneRow] = []
        /// The selection the read was made for, and its item.
        var selected: ItemSelection?
        var detail: ItemInfo?
    }

    /// Read everything shown, in one turn of the call queue so the
    /// parts agree with each other.
    func reload() {
        guard visible, let calls = engine.calls else { return }
        let workspace = workspace, list = list, generation = generation, lens = lens
        let picked = selected
        let asked = Date()
        var began = asked, ended = asked
        calls.run({ engine -> Snapshot in
            began = Date()
            defer { ended = Date() }
            let workspaces = try engine.workspaces()
            // A workspace that has gone (deleted, left, another
            // account's) falls back to Home, and a list that has gone
            // to the Inbox, rather than failing the read.
            // Home is the nil selection, whether it was asked for by
            // its id (a tray row carries one) or not at all.
            let found = workspaces.first { $0.docId == workspace }
            let selected = found.map { $0.home ? nil : $0.docId } ?? nil
            let gone = workspace != nil && found == nil
            let sections = try workspaces.map { workspace in
                let target = workspace.home ? nil : workspace.docId
                return NavSection(
                    workspace: workspace,
                    lists: try engine.lists(workspace: target, filter: .all),
                    inboxOpen: try engine.items(workspace: target, list: Self.inbox, includeDone: false).count)
            }
            let lists = sections.first { $0.target == selected }?.lists ?? []
            let known = list == Self.inbox || lists.contains { $0.id == list }
            let shown = !gone && known ? list : Self.inbox
            var snapshot = Snapshot(
                sections: sections, lens: lens, workspace: selected,
                list: shown, items: [])
            snapshot.starred = try engine.starred()
            switch lens {
            case .starred:
                break
            case .agenda:
                snapshot.agenda = try engine.agenda(days: Self.agendaDays)
            case .done:
                // The engine reads Done a workspace at a time: the
                // newest of each, merged, newest first.
                let rows = try sections.flatMap { section in
                    try engine.done(workspace: section.target, limit: UInt32(Self.doneLimit)).map {
                        DoneRow(item: $0, workspace: section.target, workspaceName: section.workspace.name)
                    }
                }
                snapshot.done = Array(
                    rows.sorted { $0.item.lifecycleAt > $1.item.lifecycleAt }.prefix(Self.doneLimit))
            case nil:
                snapshot.items = try engine.items(workspace: selected, list: shown, includeDone: false)
            }
            if let picked {
                snapshot.selected = picked
                // Whatever the reason it cannot be read, there is no
                // page to show: the panel closes.
                snapshot.detail = try? engine.item(workspace: picked.workspace, id: picked.id)
            }
            return snapshot
        }) { [weak self] result in
            guard let self, generation == self.generation else { return }
            if Self.timing, case .success(let snapshot) = result {
                let landed = Date()
                var rows: Int = snapshot.items.count + snapshot.done.count
                for group in snapshot.agenda { rows += group.rows.count }
                if snapshot.lens == .starred { rows += snapshot.starred.count }
                let workspaces: Int = snapshot.sections.count
                // Two turns of the main queue, so SwiftUI's update for
                // the rows set below has run in between.
                DispatchQueue.main.async {
                    DispatchQueue.main.async {
                        let ms = { (a: Date, b: Date) -> String in
                            String(format: "%.1f", b.timeIntervalSince(a) * 1000)
                        }
                        var line = "reload: queued " + ms(asked, began) + " ms, read " + ms(began, ended)
                        line += " ms, to main " + ms(ended, landed) + " ms, render " + ms(landed, Date())
                        line += " ms, \(rows) rows, \(workspaces) workspaces\n"
                        FileHandle.standardError.write(Data(line.utf8))
                    }
                }
            }
            switch result {
            case .success(let snapshot):
                self.sections = snapshot.sections
                self.workspace = snapshot.workspace
                self.list = snapshot.list
                self.shown = Surface(lens: snapshot.lens, workspace: snapshot.workspace, list: snapshot.list)
                self.items = snapshot.items
                self.starred = snapshot.starred
                self.agenda = snapshot.agenda
                self.done = snapshot.done
                // An item selected since the read went out keeps its
                // (empty) panel until its own read lands.
                if snapshot.selected == self.selected {
                    self.detail = snapshot.detail
                    if snapshot.detail == nil { self.selected = nil }
                }
                self.error = nil
                if self.reveal {
                    self.reveal = false
                    if let shown = snapshot.sections.first(where: { $0.target == snapshot.workspace }),
                       self.collapsed.contains(shown.id) {
                        self.setCollapsed(shown.id, false)
                    }
                }
            case .failure(EngineError.Stopped):
                // The engine went under the read; its end clears the
                // window and the next one fills it.
                break
            case .failure(let error):
                self.error = engineErrorMessage(error)
            }
        }
    }

    /// A write, then the reread that shows it. The engine's `Changed`
    /// would bring the reread too; this one is ordered after the write.
    private func write(_ body: @escaping (Engine) throws -> Void) {
        guard let calls = engine.calls else {
            error = engineErrorMessage(EngineError.Stopped)
            return
        }
        calls.run(body) { [weak self] result in
            guard let self else { return }
            if case .failure(let error) = result {
                self.error = engineErrorMessage(error)
            } else {
                self.reload()
            }
        }
    }

    func tick(_ item: Item) {
        let workspace = shown.workspace
        write { try $0.setDone(workspace: workspace, id: item.id, done: true) }
    }

    /// Tick or untick a lens's row, in its own workspace.
    func setDone(workspace: String?, id: String, done: Bool) {
        write { try $0.setDone(workspace: workspace, id: id, done: done) }
    }

    /// Retitle an item, with what the new title does to its pending
    /// date suggestion (`WhenSuggest.edit`, as on iOS).
    func setText(workspace: String?, item: ItemInfo, text: String) {
        let id = item.item.id
        let suggestion: SuggestionChange
        switch WhenSuggest.edit(
            title: text, was: item.item.text, when: item.item.when, pending: item.item.whenSuggested,
            closed: item.closedAt != nil)
        {
        case .keep: suggestion = .keep
        case .clear: suggestion = .clear
        case .set(let when): suggestion = .set(when: when)
        }
        write { try $0.setText(workspace: workspace, id: id, text: text, suggestion: suggestion) }
    }

    /// The lists an item of `workspace` can be moved to: its Inbox,
    /// then its active lists in the doc's order.
    func moveLists(workspace: String?) -> [PanelList] {
        let lists = sections.first { $0.target == workspace }?.userLists ?? []
        return [PanelList(id: Self.inbox, name: String(localized: "Inbox"), icon: nil)]
            + lists.map { PanelList(id: $0.id, name: $0.name, icon: $0.icon) }
    }

    /// Move an item to another list of its own workspace
    /// (`Engine.moveItem`).
    func moveItem(workspace: String?, id: String, list: String) {
        write { try $0.moveItem(workspace: workspace, id: id, list: list) }
    }

    /// Move an item to any workflow state (`Engine.setState`).
    func setState(workspace: String?, id: String, state: ItemState) {
        write { try $0.setState(workspace: workspace, id: id, state: state) }
    }

    func setStarred(workspace: String?, id: String, starred: Bool) {
        write { try $0.setStarred(workspace: workspace, id: id, starred: starred) }
    }

    /// Copy an item beside itself in its own list
    /// (`Engine.duplicateItem`). On Starred the copy is starred too,
    /// under the row it came from, as on iOS.
    func duplicate(workspace: String?, id: String) {
        let star = shown.lens == .starred
        write { _ = try $0.duplicateItem(workspace: workspace, id: id, star: star) }
    }

    /// Send an item to the Bin (`Engine.setBinned`); its panel shuts
    /// with it. No confirmation: the Bin restores it.
    func bin(workspace: String?, id: String) {
        if selected?.id == id { clearItem() }
        write { try $0.setBinned(workspace: workspace, id: id, binned: true) }
    }

    func add(_ text: String) {
        let text = text.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !text.isEmpty else { return }
        guard shown.lens == nil else { return }
        let workspace = shown.workspace, list = shown.list
        write { _ = try $0.addItem(workspace: workspace, list: list, text: text, top: false) }
    }

    /// Create a workspace and show it; `failed` gets the reason when
    /// the engine or the server refuses.
    func createWorkspace(name: String, failed: @escaping (String) -> Void) {
        guard let calls = engine.calls else {
            return failed(engineErrorMessage(EngineError.Stopped))
        }
        calls.run({ try $0.createWorkspace(name: name) }) { [weak self] result in
            switch result {
            case .success(let docId): self?.select(workspace: docId)
            case .failure(let error): failed(engineErrorMessage(error))
            }
        }
    }
}

/// `UserDefaults` key of the list-counts preference (the General
/// pane's toggle), the same key as iOS.
let listCountsKey = "listCounts"

struct MainWindowView: View {
    @ObservedObject var model: MainWindowModel
    /// The device's list-counts preference (Settings), off by default:
    /// whether a list's row carries its Open count, as the web's "Show
    /// list counts in nav".
    @AppStorage(listCountsKey) private var showListCounts = false
    /// Which of the window's panes this is (`MainSplitViewController`
    /// hosts each apart).
    enum Pane { case sidebar, list }
    let pane: Pane

    var body: some View {
        switch pane {
        case .sidebar: sidebar
        // The lists draw no background of their own
        // (`scrollContentBackground`): the pane's shows through.
        case .list:
            items.frame(maxWidth: .infinity, maxHeight: .infinity)
                .background(Color(nsColor: PaneColor.list).ignoresSafeArea())
        }
    }

    /// The web nav's shape: the lenses over every workspace on top
    /// (Starred, Calendar, Done), then one group per open workspace, stacked, Home
    /// ("Personal") first: a heading that folds the group, then its
    /// Inbox and lists. A list row carries its workspace, so picking
    /// one in another group moves both at once.
    private var sidebar: some View {
        List(selection: Binding<NavTarget?>(
            get: { model.lens.map(NavTarget.lens) ?? .list(workspace: model.workspace, list: model.list) },
            set: {
                switch $0 {
                case .lens(let lens): model.select(lens: lens)
                case .list(let workspace, let list): model.select(workspace: workspace, list: list)
                case nil: break
                }
            }
        )) {
            Section {
                // Starred and the Inbox are counted whatever the
                // preference says, as on iOS.
                HStack {
                    Label { Text("Starred") } icon: { StarOutline() }
                    Spacer()
                    count(model.starred.count)
                }
                .navRow()
                .tag(NavTarget.lens(.starred))
                Label("Calendar", systemImage: "calendar").navRow().tag(NavTarget.lens(.agenda))
                Label("Done", systemImage: "checkmark.circle").navRow().tag(NavTarget.lens(.done))
            }
            ForEach(model.sections) { section in
                Section(isExpanded: Binding(
                    get: { !model.collapsed.contains(section.id) },
                    set: { model.setCollapsed(section.id, !$0) }
                )) {
                    HStack {
                        Label("Inbox", systemImage: "tray")
                        Spacer()
                        count(section.inboxOpen)
                    }
                    .navRow()
                    .tag(NavTarget.list(workspace: section.target, list: MainWindowModel.inbox))
                    ForEach(section.userLists) { list in
                        HStack {
                            // The list's emoji as its icon, a folder
                            // without one, as on iOS: the names line
                            // up under Inbox's.
                            Label {
                                Text(verbatim: list.name).lineLimit(1)
                            } icon: {
                                if let icon = list.icon, !icon.isEmpty {
                                    Text(verbatim: icon)
                                } else {
                                    Image(systemName: "folder")
                                }
                            }
                            Spacer()
                            if showListCounts { count(Int(list.open)) }
                        }
                        .navRow()
                        .tag(NavTarget.list(workspace: section.target, list: list.id))
                    }
                } header: {
                    if section.workspace.home {
                        Text("Personal")
                    } else {
                        Text(verbatim: section.workspace.name ?? section.workspace.docId)
                    }
                }
            }
        }
        .listStyle(.sidebar)
        // Rows of 28 pt, between the system's small (24) and medium
        // (32), which has no such step: the small size, with each row
        // (`navRow`) at the medium's type and 4 pt taller.
        .environment(\.sidebarRowSize, .small)
    }

    /// A starred item's mark on its row, in every list but Starred
    /// itself. Not a control: the item's panel stars and unstars.
    private var starMark: some View {
        StarGlyph(starred: true, size: 13)
            .accessibilityLabel(Text("Starred"))
    }

    /// A sidebar row's count; nothing is drawn for none.
    @ViewBuilder
    private func count(_ count: Int) -> some View {
        if count > 0 {
            Text(verbatim: "\(count)").foregroundStyle(.secondary).monospacedDigit()
        }
    }

    private var items: some View {
        VStack(alignment: .leading, spacing: 0) {
            if let error = model.error {
                Text(verbatim: error)
                    .font(.footnote)
                    .foregroundStyle(.red)
                    .padding(.horizontal, 12)
                    .padding(.bottom, 6)
            }
            switch model.shown.lens {
            case .starred: starred
            case .agenda: agenda
            case .done: done
            case nil: list
            }
        }
    }

    /// A list's row selection: the selected item's id, a pick opening
    /// that item's panel. `workspace` says where a row's item lives.
    /// A click on no row closes the panel.
    private func selection(_ workspace: @escaping (String) -> String?) -> Binding<String?> {
        Binding(
            get: { model.selected?.id },
            set: { id in model.selectItem(workspace: id.flatMap(workspace), id: id) })
    }

    private var list: some View {
        VStack(spacing: 0) {
            List(selection: selection { _ in model.shown.workspace }) {
                // Rows are identified by position, not by item: going
                // from one list to another then rewrites the rows on
                // screen in place, where item identity tears every one
                // down and builds it again (a few ms each). The tag
                // keeps selection on the item.
                ForEach(Array(model.items.enumerated()), id: \.offset) { _, item in
                    HStack(alignment: .firstTextBaseline, spacing: 8) {
                        ItemCheckButton(state: item.state) { _ in model.tick(item) }
                        Text(verbatim: item.text)
                            .lineLimit(1)
                            .truncationMode(.tail)
                        Spacer()
                        if model.starredIds.contains(item.id) { starMark }
                        if let when = item.when ?? item.deadline {
                            Text(verbatim: when).foregroundStyle(.secondary).monospacedDigit().fixedSize()
                        }
                    }
                    .itemRow()
                    .itemMenu(model, workspace: model.shown.workspace, id: item.id)
                    .tag(item.id)
                }
                if model.items.isEmpty {
                    Text("Nothing here").foregroundStyle(.secondary)
                }
            }
            .listStyle(.inset)
            .scrollContentBackground(.hidden)
        }
    }

    /// The Calendar: the engine's agenda as it groups it. Today first
    /// and always there (overdue rows folded in), Unscheduled events
    /// after it, then each non-empty day. A row has its tick, its clock
    /// time when timed, its text, the workspace when it is not
    /// Personal's, and the tone's note.
    private var agenda: some View {
        List(selection: selection { id in
            model.agenda.lazy.flatMap(\.rows).first { $0.id == id }?.docId
        }) {
            ForEach(model.agenda, id: \.day) { group in
                Section {
                    ForEach(group.rows, id: \.id) { row in
                        agendaRow(row).itemRow().itemMenu(model, workspace: row.docId, id: row.id).tag(row.id)
                    }
                    if group.rows.isEmpty {
                        Text("Nothing today").foregroundStyle(.secondary)
                    }
                } header: {
                    Text(verbatim: Self.heading(for: group))
                }
            }
        }
        .listStyle(.inset)
        .scrollContentBackground(.hidden)
    }

    private func agendaRow(_ row: AgendaEntry) -> some View {
        let done = row.state == .done
        return HStack(alignment: .firstTextBaseline, spacing: 8) {
            ItemCheckButton(state: row.state) { model.setDone(workspace: row.docId, id: row.id, done: $0) }
            if TodayMenu.isTimed(row), let when = row.when {
                Text(verbatim: String(when.dropFirst(11).prefix(5)))
                    .foregroundStyle(.secondary)
                    .monospacedDigit()
                    .fixedSize()
            }
            Text(verbatim: row.text)
                .lineLimit(1)
                .truncationMode(.tail)
                .foregroundStyle(done ? .secondary : .primary)
            if let name = row.docName {
                Text(verbatim: name).font(.footnote).foregroundStyle(.tertiary).lineLimit(1)
            }
            Spacer()
            if model.starredIds.contains(row.id) { starMark }
            switch row.tone {
            case .overdue: Text("overdue").font(.footnote).foregroundStyle(.red).fixedSize()
            case .warning: Text("due today").font(.footnote).foregroundStyle(.orange).fixedSize()
            case .neutral: EmptyView()
            }
        }
    }

    /// Starred: Home's curated order over every workspace. A row has
    /// its tick, its text and where it lives (the list, and the
    /// workspace when it is not Personal's). No star on a row: every
    /// one here has it. The item's panel takes it off.
    private var starred: some View {
        List(selection: selection { id in model.starred.first { $0.id == id }?.docId }) {
            ForEach(model.starred, id: \.id) { row in
                let done = row.state == .done
                HStack(alignment: .firstTextBaseline, spacing: 8) {
                    ItemCheckButton(state: row.state) { model.setDone(workspace: row.docId, id: row.id, done: $0) }
                    Text(verbatim: row.text)
                        .lineLimit(1)
                        .truncationMode(.tail)
                        .foregroundStyle(done ? .secondary : .primary)
                    Spacer()
                    Text(verbatim: [row.listName, row.docName].compactMap { $0 }.joined(separator: " · "))
                        .font(.footnote)
                        .foregroundStyle(.tertiary)
                        .lineLimit(1)
                }
                .itemRow()
                .itemMenu(model, workspace: row.docId, id: row.id)
                .tag(row.id)
            }
            if model.starred.isEmpty {
                Text("Nothing starred").foregroundStyle(.secondary)
            }
        }
        .listStyle(.inset)
        .scrollContentBackground(.hidden)
    }

    /// Done: every workspace's newest Done items, newest first. The
    /// tick puts a row back; the stamp is when it entered Done.
    private var done: some View {
        List(selection: selection { id in model.done.first { $0.item.id == id }?.workspace }) {
            ForEach(model.done) { row in
                HStack(alignment: .firstTextBaseline, spacing: 8) {
                    ItemCheckButton(state: row.item.state) {
                        model.setDone(workspace: row.workspace, id: row.item.id, done: $0)
                    }
                    Text(verbatim: row.item.text)
                        .lineLimit(1)
                        .truncationMode(.tail)
                        .foregroundStyle(.secondary)
                    if let name = row.workspaceName {
                        Text(verbatim: name).font(.footnote).foregroundStyle(.tertiary).lineLimit(1)
                    }
                    Spacer()
                    Text(verbatim: Self.stamp(row.item.lifecycleAt))
                        .font(.footnote)
                        .foregroundStyle(.secondary)
                        .monospacedDigit()
                        .fixedSize()
                }
                .itemRow()
                .itemMenu(model, workspace: row.workspace, id: row.item.id)
                .tag(row.item.id)
            }
            if model.done.isEmpty {
                Text("Nothing here").foregroundStyle(.secondary)
            }
        }
        .listStyle(.inset)
        .scrollContentBackground(.hidden)
    }

    /// A Done row's moment (unix ms), as a short date and time.
    static func stamp(_ millis: Int64) -> String {
        Date(timeIntervalSince1970: TimeInterval(millis) / 1000)
            .formatted(date: .abbreviated, time: .shortened)
    }

    /// "Today · 14 Nov", "Unscheduled", or the day spelled out
    /// ("Friday 20 Nov").
    static func heading(for group: AgendaGroup) -> String {
        if group.today {
            let date = group.day.flatMap { Agenda.start(of: $0) } ?? Date()
            return String(localized: "Today · \(date.formatted(.dateTime.day().month(.abbreviated)))")
        }
        guard let text = group.day else { return String(localized: "Unscheduled") }
        guard let date = Agenda.start(of: text) else { return text }
        return date.formatted(.dateTime.weekday(.wide).day().month(.abbreviated))
    }
}

private extension View {
    /// A sidebar row's size: see the sidebar's `sidebarRowSize`.
    func navRow() -> some View {
        font(.body).imageScale(.medium).frame(minHeight: 20)
    }

    /// An item row's size, fixed: 32 pt, the web's row height
    /// (`ROW_HEIGHT` in `js/web/src/Workspace.tsx`). The list adds 4 pt
    /// above and below the content. The line between two rows is the
    /// web's too.
    func itemRow() -> some View {
        frame(height: 24)
            .listRowSeparatorTint(Color(nsColor: PaneColor.rowDivider))
    }

    /// An item row's right-click menu: the same actions as the panel's
    /// "…" button in the toolbar (`MainSplitViewController`).
    func itemMenu(_ model: MainWindowModel, workspace: String?, id: String) -> some View {
        contextMenu {
            Button("Duplicate") { model.duplicate(workspace: workspace, id: id) }
            Divider()
            Button("Move to Bin") { model.bin(workspace: workspace, id: id) }
        }
    }
}

/// The New Workspace sheet: a standard alert with a name field, Create
/// enabled once the name has a non-blank character. Return in the field
/// is Create (the alert's default button), Escape is Cancel.
final class NewWorkspacePrompt: NSObject, NSTextFieldDelegate {
    private let alert = NSAlert()
    private let field = NSTextField(frame: NSRect(x: 0, y: 0, width: 260, height: 24))

    override init() {
        super.init()
        alert.messageText = String(localized: "New Workspace")
        alert.informativeText = String(localized: "Create a shareable workspace")
        field.placeholderString = String(localized: "Name")
        field.delegate = self
        alert.accessoryView = field
        alert.addButton(withTitle: String(localized: "Create"))
        alert.addButton(withTitle: String(localized: "Cancel"))
        alert.buttons[0].isEnabled = false
    }

    /// `completion` gets the trimmed name, or nil on Cancel.
    func run(on window: NSWindow, completion: @escaping (String?) -> Void) {
        alert.window.initialFirstResponder = field
        alert.beginSheetModal(for: window) { [field] response in
            let name = field.stringValue.trimmingCharacters(in: .whitespacesAndNewlines)
            completion(response == .alertFirstButtonReturn && !name.isEmpty ? name : nil)
        }
    }

    func controlTextDidChange(_ obj: Notification) {
        alert.buttons[0].isEnabled = !field.stringValue.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
    }
}
