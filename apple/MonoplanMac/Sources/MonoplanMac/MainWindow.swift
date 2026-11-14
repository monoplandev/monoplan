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
    private let onVisibility: (Bool) -> Void
    private var newWorkspacePrompt: NewWorkspacePrompt?
    private var modelSync: AnyCancellable?
    private let split: MainSplitViewController

    init(engine: EngineController, onVisibility: @escaping (Bool) -> Void) {
        model = MainWindowModel(engine: engine)
        self.onVisibility = onVisibility
        // A toolbar window, as Finder or Notes: the tall unified bar
        // with the large traffic lights. The content view takes the
        // whole frame so the sidebar runs to the top edge under them.
        // The panes and the toolbar are `MainSplitViewController`'s.
        split = MainSplitViewController(model: model)
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
    @Published private(set) var items: [Item] = []
    /// The lens shown in place of a list, if any. The list selection
    /// under it is kept, though nothing shows it.
    @Published private(set) var lens: Lens?
    /// Each lens's rows, read only while it is the one shown.
    @Published private(set) var starred: [StarredItem] = []
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
            return lists.first { $0.id == list }?.name ?? list
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
        items = []
        clearLenses()
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
        clearLenses()
        clearItem()
        self.workspace = workspace
        self.list = list
        // The old list's rows are not this one's.
        items = []
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
        calls.run({ engine -> Snapshot in
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
                NavSection(
                    workspace: workspace,
                    lists: try engine.lists(workspace: workspace.home ? nil : workspace.docId, filter: .all))
            }
            let lists = sections.first { $0.target == selected }?.lists ?? []
            let known = list == Self.inbox || lists.contains { $0.id == list }
            let shown = !gone && known ? list : Self.inbox
            var snapshot = Snapshot(
                sections: sections, workspace: selected,
                list: shown, items: [])
            switch lens {
            case .starred:
                snapshot.starred = try engine.starred()
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
            switch result {
            case .success(let snapshot):
                self.sections = snapshot.sections
                self.workspace = snapshot.workspace
                self.list = snapshot.list
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
        let workspace = workspace
        write { try $0.setDone(workspace: workspace, id: item.id, done: true) }
    }

    /// Tick or untick a lens's row, in its own workspace.
    func setDone(workspace: String?, id: String, done: Bool) {
        write { try $0.setDone(workspace: workspace, id: id, done: done) }
    }

    func setStarred(workspace: String?, id: String, starred: Bool) {
        write { try $0.setStarred(workspace: workspace, id: id, starred: starred) }
    }

    /// Take a row out of Starred.
    func unstar(_ row: StarredItem) {
        write { try $0.setStarred(workspace: row.docId, id: row.id, starred: false) }
    }

    func add(_ text: String) {
        let text = text.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !text.isEmpty else { return }
        let workspace = workspace, list = list
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

struct MainWindowView: View {
    @ObservedObject var model: MainWindowModel
    @State private var draft = ""
    /// Which of the window's panes this is (`MainSplitViewController`
    /// hosts each apart).
    enum Pane { case sidebar, list }
    let pane: Pane

    var body: some View {
        switch pane {
        case .sidebar: sidebar
        case .list: items.frame(maxWidth: .infinity, maxHeight: .infinity)
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
                Label { Text("Starred") } icon: { StarOutline() }.tag(NavTarget.lens(.starred))
                Label("Calendar", systemImage: "calendar").tag(NavTarget.lens(.agenda))
                Label("Done", systemImage: "checkmark").tag(NavTarget.lens(.done))
            }
            ForEach(model.sections) { section in
                Section(isExpanded: Binding(
                    get: { !model.collapsed.contains(section.id) },
                    set: { model.setCollapsed(section.id, !$0) }
                )) {
                    Text("Inbox").tag(NavTarget.list(workspace: section.target, list: MainWindowModel.inbox))
                    ForEach(section.userLists) { list in
                        HStack {
                            Text(verbatim: [list.icon, list.name].compactMap { $0 }.joined(separator: " "))
                                .lineLimit(1)
                            Spacer()
                            if list.open > 0 {
                                Text(verbatim: "\(list.open)").foregroundStyle(.secondary).monospacedDigit()
                            }
                        }
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
            switch model.lens {
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
            List(selection: selection { _ in model.workspace }) {
                ForEach(model.items) { item in
                    HStack(alignment: .firstTextBaseline, spacing: 8) {
                        Button {
                            model.tick(item)
                        } label: {
                            Image(systemName: "circle")
                        }
                        .buttonStyle(.plain)
                        .accessibilityLabel(Text("Mark as done"))
                        Text(verbatim: item.text)
                        Spacer()
                        if let when = item.when ?? item.deadline {
                            Text(verbatim: when).foregroundStyle(.secondary).monospacedDigit()
                        }
                    }
                    .tag(item.id)
                }
                if model.items.isEmpty {
                    Text("Nothing here").foregroundStyle(.secondary)
                }
            }
            .listStyle(.inset)
            Divider()
            TextField("Add an item", text: $draft)
                .textFieldStyle(.roundedBorder)
                .onSubmit {
                    model.add(draft)
                    draft = ""
                }
                .padding(8)
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
                        agendaRow(row).tag(row.id)
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
    }

    private func agendaRow(_ row: AgendaEntry) -> some View {
        let done = row.state == .done
        return HStack(alignment: .firstTextBaseline, spacing: 8) {
            Button {
                model.setDone(workspace: row.docId, id: row.id, done: !done)
            } label: {
                Image(systemName: done ? "checkmark.circle.fill" : "circle")
            }
            .buttonStyle(.plain)
            .accessibilityLabel(Text("Mark as done"))
            if TodayMenu.isTimed(row), let when = row.when {
                Text(verbatim: String(when.dropFirst(11).prefix(5)))
                    .foregroundStyle(.secondary)
                    .monospacedDigit()
            }
            Text(verbatim: row.text)
                .foregroundStyle(done ? .secondary : .primary)
            if let name = row.docName {
                Text(verbatim: name).font(.footnote).foregroundStyle(.tertiary).lineLimit(1)
            }
            Spacer()
            switch row.tone {
            case .overdue: Text("overdue").font(.footnote).foregroundStyle(.red)
            case .warning: Text("due today").font(.footnote).foregroundStyle(.orange)
            case .neutral: EmptyView()
            }
        }
    }

    /// Starred: Home's curated order over every workspace. A row has
    /// its tick, its text, where it lives (the list, and the workspace
    /// when it is not Personal's) and a star that takes it out.
    private var starred: some View {
        List(selection: selection { id in model.starred.first { $0.id == id }?.docId }) {
            ForEach(model.starred, id: \.id) { row in
                let done = row.state == .done
                HStack(alignment: .firstTextBaseline, spacing: 8) {
                    Button {
                        model.setDone(workspace: row.docId, id: row.id, done: !done)
                    } label: {
                        Image(systemName: done ? "checkmark.circle.fill" : "circle")
                    }
                    .buttonStyle(.plain)
                    .accessibilityLabel(Text("Mark as done"))
                    Text(verbatim: row.text)
                        .foregroundStyle(done ? .secondary : .primary)
                    Spacer()
                    Text(verbatim: [row.listName, row.docName].compactMap { $0 }.joined(separator: " · "))
                        .font(.footnote)
                        .foregroundStyle(.tertiary)
                        .lineLimit(1)
                    Button {
                        model.unstar(row)
                    } label: {
                        StarGlyph(starred: true)
                    }
                    .buttonStyle(.plain)
                    .accessibilityLabel(Text("Remove star"))
                }
                .tag(row.id)
            }
            if model.starred.isEmpty {
                Text("Nothing starred").foregroundStyle(.secondary)
            }
        }
        .listStyle(.inset)
    }

    /// Done: every workspace's newest Done items, newest first. The
    /// tick puts a row back; the stamp is when it entered Done.
    private var done: some View {
        List(selection: selection { id in model.done.first { $0.item.id == id }?.workspace }) {
            ForEach(model.done) { row in
                HStack(alignment: .firstTextBaseline, spacing: 8) {
                    Button {
                        model.setDone(workspace: row.workspace, id: row.item.id, done: false)
                    } label: {
                        Image(systemName: "checkmark.circle.fill")
                    }
                    .buttonStyle(.plain)
                    .accessibilityLabel(Text("Mark as not done"))
                    Text(verbatim: row.item.text)
                        .foregroundStyle(.secondary)
                    if let name = row.workspaceName {
                        Text(verbatim: name).font(.footnote).foregroundStyle(.tertiary).lineLimit(1)
                    }
                    Spacer()
                    Text(verbatim: Self.stamp(row.item.lifecycleAt))
                        .font(.footnote)
                        .foregroundStyle(.secondary)
                        .monospacedDigit()
                }
                .tag(row.item.id)
            }
            if model.done.isEmpty {
                Text("Nothing here").foregroundStyle(.secondary)
            }
        }
        .listStyle(.inset)
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
