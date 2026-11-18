// The window: native SwiftUI over the engine's typed calls
// (spec/swift-bindings-plan.md "The macOS window"), replacing the
// webview. Deliberately a debug screen for now: a sidebar laid out as
// the web nav is (the lenses
// Starred, Calendar and Done on top, then one group per open workspace,
// Personal first, each a heading row over its Inbox and lists: the row
// folds the group, and each has a "…" menu: Export and Import JSON
// (`WorkspaceJSON.swift`), and on a shared workspace Share…
// (`SharingSheet.swift`) and Delete or Leave; a list's row has
// Delete, its items to the Inbox or the Bin, and the Inbox's row the
// device's Inbox order, oldest or newest first),
// and either the open items of the selected list with a tick and a
// field to add one, or a lens over every workspace: Starred in its
// curated order, the Calendar (the engine's agenda, by day), Done,
// newest first, or the Bin, newest binned first (its sidebar row only
// while something is binned, as on the web and iOS). A row of any of them can be selected, which opens the
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
        // The bar draws the title itself, as a button that renames the
        // list (`MainSplitViewController`'s title item); the window's
        // own title stays set for the Window menu and Mission Control.
        window.titleVisibility = .hidden
        window.isReleasedWhenClosed = false
        window.minSize = NSSize(width: 520, height: 360)
        window.setContentSize(NSSize(width: 860, height: 560))
        window.center()
        // After the default frame, so a saved one wins.
        window.setFrameAutosaveName("MonoplanWindow")
        super.init(window: window)
        window.delegate = self
        split.restoreDividers()
        // The window's title is the name of what is shown. It, the
        // bar's title item and the panel's toolbar items follow the
        // model. `objectWillChange` is before the change,
        // hence the hop.
        modelSync = model.objectWillChange
            .receive(on: DispatchQueue.main)
            .sink { [weak self] in
                guard let self, let window = self.window else { return }
                let title = self.model.windowTitle
                if window.title != title { window.title = title }
                self.split.sync()
            }
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }

    /// Bring the window forward, on a list when one is given (a tray
    /// row: the item's list in its workspace, nil being Home's), with
    /// the item's panel open when the row names one. A dated item with
    /// no list has no row in Inbox, so it opens over the Calendar
    /// (spec/inbox-plan.md).
    func show(
        list: String? = nil, workspace: String? = nil, item: String? = nil, onCalendar: Bool = false
    ) {
        onVisibility(true)
        model.visible = true
        syncSignedIn()
        if let list {
            if onCalendar {
                model.select(lens: .agenda)
            } else {
                model.select(workspace: workspace, list: list)
            }
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

    /// The toolbar's Sign In, there while there is no account.
    private func syncSignedIn() {
        split.signedIn = signedIn()
        split.sync()
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
    /// doc's order, without the reserved id (which has no `ListMeta`
    /// row of its own).
    var userLists: [ListInfo] {
        lists.filter { $0.archivedAt == nil && $0.id != MainWindowModel.inbox }
    }
}

/// The views over every workspace, on top of the sidebar.
enum Lens: Hashable {
    case starred, agenda, done, bin
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
    /// The workspace's doc id, Home's too: a typed call's `workspace`
    /// takes it.
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
    /// Counts the presses of the toolbar's "Today": each one scrolls
    /// the Calendar back to Today's header, or Overdue's above it
    /// (`AgendaTable`).
    @Published private(set) var todayJumps = 0
    @Published private(set) var done: [DoneRow] = []
    /// The Done lens's filter as chosen (`DoneFilter.swift`), kept
    /// verbatim; what is applied and shown is `doneFilter`.
    @Published private(set) var doneFilterChoice = DoneFilterStore.load()
    /// The Bin's rows, every workspace's, newest binned first. Always
    /// read: the sidebar shows the Bin only while it has any.
    @Published private(set) var binned: [BinEntry] = []
    /// The selected row's item, if one is: its panel is open.
    @Published private(set) var selected: ItemSelection?
    /// The panel's item page. On a new selection the old page stays
    /// until the new one is read, so the panel and its toolbar items
    /// swap in one go rather than blanking (as `shown` does for the
    /// rows). An item that cannot be read (hard deleted, its workspace
    /// gone) closes the panel.
    @Published private(set) var detail: ItemInfo?
    /// The selection `detail` was read for: `selected` once it lands.
    @Published private(set) var detailOf: ItemSelection?

    /// The panel's item when it is the selected one, for the panel's
    /// and toolbar's writes: nil in the moment between a click and the
    /// new item's read, when the panel still shows the old item.
    var current: (selection: ItemSelection, item: ItemInfo)? {
        guard let selected, let detail, detailOf == selected else { return nil }
        return (selected, detail)
    }
    /// The last failed read or write, until the next one succeeds.
    @Published private(set) var error: String?
    /// The account's id; nil while the profile has none (local only).
    @Published private(set) var accountId: String?
    /// Whether the account owns each shared workspace, by doc id, from
    /// its member list (`Engine.workspaceMembers`): a workspace row's
    /// "…" menu offers Delete to an owner and Leave to anyone else, and
    /// neither while this is unknown (offline, or still loading), as
    /// on iOS and the web.
    @Published private(set) var ownsWorkspace: [String: Bool] = [:]
    /// How many members each shared workspace has, by doc id, from the
    /// same member list: Delete's confirm on a list says who will see
    /// the items it sends to the Inbox.
    @Published private(set) var memberCounts: [String: Int] = [:]
    /// Each shared workspace's members, by doc id, from the same member
    /// list: who an item there can be assigned to, and the email an
    /// assignee's account id shows as (spec/sharing-plan.md
    /// "Assignment"). Absent while the list is not known.
    @Published private(set) var members: [String: [WorkspaceMember]] = [:]
    /// Shared workspaces whose role is being asked for now.
    private var rolesLoading: Set<String> = []

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
    /// `MONOPLAN_TIMING=1` logs each reload's timings to stderr.
    static let timing = ProcessInfo.processInfo.environment["MONOPLAN_TIMING"] != nil
    /// Far enough out to be the whole future, as on iOS.
    static let agendaDays: UInt32 = 36_500
    /// How far back the Calendar reaches: the days gone by above
    /// Today, a year of them, as on iOS (`AppModel.agendaPastDays`).
    static let agendaPastDays: UInt32 = 365
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
        case .bin: return String(localized: "Bin")
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

    /// The window's title: the list's name with its Open count, as the
    /// sidebar row has it. Starred and the Bin count their rows; the
    /// Calendar and Done have no count (Done's rows are capped). None
    /// for zero.
    var windowTitle: String {
        let count = titleCount
        return count > 0 ? "\(listTitle) (\(count))" : listTitle
    }

    /// The sidebar row's glyph for what is shown, before the bar's
    /// title: the lenses' and Inbox's. A user list has none here (its
    /// emoji, if any, leads `listTitle`).
    var titleIcon: NSImage? {
        switch shown.lens {
        case .starred: return StarArt.outline
        case .agenda: return NavArt.calendar
        case .done: return NavArt.done
        case .bin: return NavArt.bin
        case nil: return shown.list == Self.inbox ? NavArt.inbox : nil
        }
    }

    /// The count beside the bar's title, muted after the name.
    var titleCount: Int {
        switch shown.lens {
        case .starred: return starred.count
        case .bin: return binned.count
        case .agenda, .done: return 0
        case nil: return items.count
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

    /// The toolbar's "Today": the Calendar scrolls back to Today.
    func jumpToToday() {
        todayJumps += 1
    }

    /// Select a row's item, opening its panel; nil closes it.
    func selectItem(workspace: String?, id: String?) {
        guard let id else { return clearItem() }
        guard id != selected?.id else { return }
        let picked = ItemSelection(workspace: workspace, id: id)
        selected = picked
        readDetail(picked)
    }

    /// Read just the selected item's page: a click on another row
    /// changes nothing else, so the full `reload` is not needed.
    private func readDetail(_ picked: ItemSelection) {
        guard visible, let calls = engine.calls else { return }
        calls.run({ engine -> ItemInfo? in
            try? engine.item(workspace: picked.workspace, id: picked.id)
        }) { [weak self] result in
            // A newer selection, or the panel shut, since the read
            // went out: its own read (or none) decides.
            guard let self, self.selected == picked else { return }
            switch result {
            case .success(let info?): self.setDetail(info, of: picked)
            case .success(nil): self.clearItem()
            case .failure: break
            }
        }
    }

    private func setDetail(_ info: ItemInfo, of selection: ItemSelection) {
        detail = info
        detailOf = selection
    }

    func closeItem() {
        clearItem()
    }

    private func clearItem() {
        selected = nil
        detail = nil
        detailOf = nil
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
        binned = []
    }

    private func clear() {
        sections = []
        ownsWorkspace = [:]
        memberCounts = [:]
        items = []
        clearLenses()
        clearItem()
        generation += 1
    }

    private struct Snapshot {
        var sections: [NavSection]
        var accountId: String?
        var lens: Lens?
        var workspace: String?
        var list: String
        var items: [Item]
        var starred: [StarredItem] = []
        var agenda: [AgendaGroup] = []
        var done: [DoneRow] = []
        var binned: [BinEntry] = []
        /// The selection the read was made for, and its item.
        var selected: ItemSelection?
        var detail: ItemInfo?
    }

    /// Read everything shown, in one turn of the call queue so the
    /// parts agree with each other.
    func reload() {
        guard visible, let calls = engine.calls else { return }
        let workspace = workspace, list = list, generation = generation, lens = lens
        let picked = selected, doneChoice = doneFilterChoice
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
            snapshot.accountId = try engine.account()?.accountId
            snapshot.starred = try engine.starred()
            snapshot.binned = try engine.binned()
            switch lens {
            case .starred, .bin:
                break
            case .agenda:
                snapshot.agenda = try engine.agenda(days: Self.agendaDays, pastDays: Self.agendaPastDays)
            case .done:
                // One read over every workspace, merged and filtered
                // in Rust; the filter narrowed to the sections just
                // read, so the two agree.
                let filter = Self.effectiveDoneFilter(doneChoice, sections: sections)
                snapshot.done = try engine.doneEverywhere(filter: filter, limit: UInt32(Self.doneLimit)).map {
                    entry in
                    let section = sections.first { $0.id == entry.docId }
                    return DoneRow(
                        item: entry.item, workspace: entry.docId, workspaceName: section?.workspace.name)
                }
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
                if snapshot.lens == .bin { rows += snapshot.binned.count }
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
                self.accountId = snapshot.accountId
                self.refreshRoles()
                self.workspace = snapshot.workspace
                self.list = snapshot.list
                self.shown = Surface(lens: snapshot.lens, workspace: snapshot.workspace, list: snapshot.list)
                self.items = snapshot.items
                self.starred = snapshot.starred
                self.agenda = snapshot.agenda
                self.done = snapshot.done
                self.binned = snapshot.binned
                // An item selected since the read went out keeps the
                // old page until its own read lands.
                if let picked = snapshot.selected, picked == self.selected {
                    if let detail = snapshot.detail {
                        self.setDetail(detail, of: picked)
                    } else {
                        self.clearItem()
                    }
                }
                self.error = nil
                // The Bin emptied while shown: its sidebar row goes, so
                // the list under it comes back.
                if snapshot.lens == .bin, snapshot.binned.isEmpty, self.lens == .bin {
                    self.select(workspace: snapshot.workspace, list: snapshot.list)
                    return
                }
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
        moveLists(section(workspace))
    }

    /// The section of a workspace named either way a row names one:
    /// as a typed call takes it (nil for Home), or by its doc id, which
    /// the Calendar, Starred and Bin rows and the tray give for Home
    /// too.
    private func section(_ workspace: String?) -> NavSection? {
        sections.first { $0.target == workspace || $0.workspace.docId == workspace }
    }

    private func moveLists(_ section: NavSection?) -> [PanelList] {
        [PanelList(id: Self.inbox, name: String(localized: "Inbox"), icon: nil)]
            + (section?.userLists ?? []).map { PanelList(id: $0.id, name: $0.name, icon: $0.icon) }
    }

    /// Every other open workspace with the lists an item of
    /// `workspace` can be moved to there, in the sidebar's order.
    func otherMoveLists(workspace: String?) -> [PanelListGroup] {
        let own = section(workspace)?.id
        return sections.filter { $0.id != own }.map {
            PanelListGroup(
                workspace: $0.target, name: workspaceLabel($0.target), shared: !$0.workspace.home,
                lists: moveLists($0))
        }
    }

    /// A workspace's name as the sidebar heads it.
    private func workspaceLabel(_ workspace: String?) -> String {
        guard let found = section(workspace)?.workspace, !found.home
        else { return String(localized: "Personal") }
        return found.name ?? found.docId
    }

    /// Move an item to a list of another workspace
    /// (`Engine.moveItemAcross`, spec/sharing-plan.md "Cross-doc
    /// move"): it keeps its id and its star, and one Undo takes it
    /// back. Asks first only when the audience changes, as the web
    /// does: into a workspace with other members, or out of one.
    /// The panel follows the item to its new workspace.
    func moveItemAcross(from source: String?, id: String, to target: String?, list: String, on window: NSWindow?) {
        let exposes = hasOtherMembers(target), removes = hasOtherMembers(source)
        let move = { [weak self] in
            guard let self else { return }
            guard let calls = engine.calls else {
                error = engineErrorMessage(EngineError.Stopped)
                return
            }
            calls.run({
                try $0.moveItemAcross(item: ItemRef(workspace: source, id: id), to: target, list: list)
            }) { [weak self] result in
                guard let self else { return }
                if case .failure(let error) = result {
                    self.error = engineErrorMessage(error)
                    return
                }
                if selected == ItemSelection(workspace: source, id: id) {
                    let moved = ItemSelection(workspace: target, id: id)
                    if detailOf == selected { detailOf = moved }
                    selected = moved
                }
                reload()
            }
        }
        guard exposes || removes else { return move() }
        MoveConfirm.across(
            to: workspaceLabel(target),
            exposes: exposes ? .some(target.flatMap { memberCounts[$0] }) : nil,
            removesFrom: removes ? workspaceLabel(source) : nil,
            on: window, then: move)
    }

    /// Move an item to another list of its own workspace
    /// (`Engine.moveItem`).
    func moveItem(workspace: String?, id: String, list: String) {
        write { try $0.moveItem(workspace: workspace, id: id, list: list) }
    }

    /// Set or clear an item's place (`Engine.setPlace`). `exact` writes
    /// a known place as it is; otherwise the engine may fill a bare
    /// label from a known place of that label (spec/place-plan.md).
    func setPlace(workspace: String?, id: String, place: Place?, exact: Bool = false) {
        write { try $0.setPlace(workspace: workspace, id: id, place: place, exact: exact) }
    }

    /// Set or clear an item's planned date, and its duration in the
    /// same call (`Engine.setWhen`).
    func setWhen(workspace: String?, id: String, when: String?, duration: DurationChange) {
        write { try $0.setWhen(workspace: workspace, id: id, when: when, duration: duration) }
    }

    /// Assign an item to one member of its workspace by account id, or
    /// unassign it (nil) (`Engine.setAssignee`).
    func setAssignee(workspace: String?, id: String, assignee: String?) {
        write { try $0.setAssignee(workspace: workspace, id: id, assignee: assignee) }
    }

    /// What the item panel's assignee row offers for an item of
    /// `workspace` (spec/sharing-plan.md "Assignment"): nil for Home,
    /// where there is no row; otherwise whether the workspace has
    /// anyone else in it, and its members when the list is known.
    func assignment(workspace: String?) -> PanelAssignment? {
        guard let workspace, sharedWorkspaceName(workspace) != nil else { return nil }
        return PanelAssignment(others: hasOtherMembers(workspace), members: members[workspace])
    }

    /// Mark a dated item an event, or a task again (`Engine.setEvent`).
    func setEvent(workspace: String?, id: String, event: Bool) {
        write { try $0.setEvent(workspace: workspace, id: id, event: event) }
    }

    /// Set an item's length in whole minutes, or clear it (nil)
    /// (`Engine.setDuration`).
    func setDuration(workspace: String?, id: String, minutes: Int?) {
        write { try $0.setDuration(workspace: workspace, id: id, minutes: minutes.map { UInt32(clamping: $0) }) }
    }

    /// The account's known places matching `query` (all when empty),
    /// most recently used first, as the iOS place editor offers them.
    /// Nothing comes back on failure.
    func knownPlaces(matching query: String, limit: UInt32 = 6, completion: @escaping ([PlaceSuggestion]) -> Void) {
        guard let calls = engine.calls else { return }
        calls.run({ try $0.places(query: query, limit: limit) }) { result in
            if case .success(let rows) = result { completion(rows) }
        }
    }

    /// Search's hits for `query` over every open workspace
    /// (`Engine.find`, spec/search.md), ranked. `completion` gets nil
    /// on failure.
    func find(_ query: String, limit: UInt32, completion: @escaping ([FindHit]?) -> Void) {
        guard let calls = engine.calls else { return completion(nil) }
        calls.run({ try $0.find(query: query, limit: limit) }) { result in
            completion(try? result.get())
        }
    }

    /// Show what a search hit names. A list opens as a sidebar row
    /// would. An item opens its panel over where its row is: an Open
    /// one in its list (an event's is the Calendar), a closed one in
    /// Done, a binned one in the Bin.
    func open(_ hit: FindHit) {
        // A hit names its doc by id, Home's included; the calls take
        // nil for Home.
        let workspace = sections.first { $0.workspace.docId == hit.docId }?.target
        switch hit.kind {
        case .list:
            select(workspace: workspace, list: hit.id)
        case .item:
            switch hit.state {
            case .binned?: select(lens: .bin)
            case .done?, .cancelled?: select(lens: .done)
            default: select(workspace: workspace, list: hit.listId ?? Self.inbox)
            }
            selectItem(workspace: workspace, id: hit.id)
        }
    }

    /// The shown list when it can be renamed from the title: a
    /// user-created list (the reserved ones and the lenses cannot).
    var renamableList: (workspace: String?, list: String, name: String)? {
        let workspace = shown.workspace, list = shown.list
        guard shown.lens == nil, list != Self.inbox,
              let found = sections.first(where: { $0.target == workspace })?.lists
                .first(where: { $0.id == list })
        else { return nil }
        return (workspace, list, found.name)
    }

    /// Rename a user-created list (`Engine.renameList`). A blank or
    /// unchanged name writes nothing, so no empty undo step.
    func renameList(workspace: String?, list: String, name: String) {
        let name = name.trimmingCharacters(in: .whitespacesAndNewlines)
        let current = sections.first { $0.target == workspace }?.lists.first { $0.id == list }?.name
        guard !name.isEmpty, name != current else { return }
        write { try $0.renameList(workspace: workspace, list: list, name: name) }
    }

    /// Delete a user-created list (`Engine.deleteList`): every item it
    /// held goes to the end of its workspace's Inbox queue, and with
    /// `.toBin` to the Bin too. One undo step (Edit › Undo) restores
    /// the list and its items. The caller has asked (`ListConfirm`).
    /// With the list shown, the reread falls back to the Inbox, where
    /// the items went.
    func deleteList(workspace: String?, list: String, mode: DeleteMode) {
        if lens == nil, workspace == self.workspace, list == self.list { clearItem() }
        write { try $0.deleteList(workspace: workspace, list: list, mode: mode) }
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

    /// Take an item out of the Bin, back to its list and state.
    func restore(workspace: String?, id: String) {
        write { try $0.setBinned(workspace: workspace, id: id, binned: false) }
    }

    /// Hard-delete a binned item (`Engine.deleteBinned`); its panel
    /// shuts first. The caller has asked (`BinConfirm`).
    func deleteBinned(workspace: String?, id: String) {
        if selected?.id == id { clearItem() }
        write { try $0.deleteBinned(workspace: workspace, id: id) }
    }

    /// Hard-delete everything in the Bin, a workspace at a time
    /// (`Engine.emptyBin`): only those with something binned. The
    /// caller has asked (`BinConfirm`).
    func emptyBin() {
        if let selected, binned.contains(where: { $0.id == selected.id }) { clearItem() }
        for docId in Set(binned.map(\.docId)) {
            write { _ = try $0.emptyBin(workspace: docId) }
        }
    }

    /// What an item with no list is filed under, by whether it has a
    /// date: Inbox holds only undated items, a dated one lives on the
    /// Calendar (spec/inbox-plan.md "Membership"). A label only: the
    /// list id is `inbox` either way.
    static func noListLabel(dated: Bool) -> String {
        dated ? String(localized: "Calendar") : String(localized: "Inbox")
    }

    /// The name of the list a binned item would go back to, for its
    /// row; nil when that list is not known.
    func binnedListName(_ entry: BinEntry) -> String? {
        if entry.listId == Self.inbox { return Self.noListLabel(dated: entry.when != nil) }
        let section = sections.first { $0.workspace.docId == entry.docId }
        guard let list = section?.lists.first(where: { $0.id == entry.listId }) else { return nil }
        return [list.icon, list.name].compactMap { $0 }.filter { !$0.isEmpty }.joined(separator: " ")
    }

    /// The name of the list a Done item is on, for its row: the list,
    /// without its emoji, then the workspace when it is not Personal's. An item whose
    /// list row has gone reads as the Inbox's, where it shows.
    func doneListName(_ row: DoneRow) -> String {
        let listId = row.item.listId
        let name: String
        if listId != Self.inbox,
            let list = section(row.workspace)?.lists.first(where: { $0.id == listId })
        {
            name = list.name
        } else {
            name = Self.noListLabel(dated: row.item.when != nil)
        }
        return [name, row.workspaceName].compactMap { $0 }.joined(separator: " · ")
    }

    // MARK: The Done filter

    /// The Done filter in effect: the choice narrowed to the workspaces
    /// that are open and the lists on offer, as the web's
    /// `effectiveDoneFilter` and iOS's `AppModel.doneFilter` have it (a
    /// list of an unchecked workspace does not empty the page from
    /// behind a popover that no longer shows it). Before any workspace
    /// is read the choice stands.
    var doneFilter: DoneFilter {
        Self.effectiveDoneFilter(doneFilterChoice, sections: sections)
    }

    static func effectiveDoneFilter(_ choice: DoneFilter, sections: [NavSection]) -> DoneFilter {
        guard !sections.isEmpty else { return choice }
        let docs = choice.docs.filter { doc in sections.contains { $0.id == doc } }
        let lists = choice.lists.filter { key in
            guard let section = sections.first(where: { key.hasPrefix($0.id + "/") }),
                  docs.isEmpty || docs.contains(section.id)
            else { return false }
            let listId = key.dropFirst(section.id.count + 1)
            return listId == inbox || section.lists.contains { $0.id == listId }
        }
        return DoneFilter(docs: docs, lists: lists)
    }

    /// The workspaces the Done filter offers: every open one, Personal
    /// first.
    var doneWorkspaceOptions: [DoneWorkspaceOption] {
        sections.map {
            DoneWorkspaceOption(docId: $0.id, name: workspaceLabel($0.id), shared: !$0.workspace.home)
        }
    }

    /// The lists the Done filter offers: those of the checked
    /// workspaces, or of every one when none is checked; each
    /// workspace's Inbox, then its lists in their order, archived ones
    /// included (their finished items are still on the page).
    var doneListOptions: [DoneListOption] {
        let docs = doneFilter.docs
        return sections
            .filter { docs.isEmpty || docs.contains($0.id) }
            .flatMap { section -> [DoneListOption] in
                let workspace = section.workspace.home ? nil : workspaceLabel(section.id)
                let inbox = DoneListOption(
                    docId: section.id, listId: Self.inbox, name: String(localized: "Inbox"),
                    icon: nil, workspace: workspace, archived: false)
                return [inbox] + section.lists.filter { $0.id != Self.inbox }.map {
                    DoneListOption(
                        docId: section.id, listId: $0.id, name: $0.name,
                        icon: $0.icon, workspace: workspace, archived: $0.archivedAt != nil)
                }
            }
    }

    /// Check or uncheck a workspace in the Done filter.
    func toggleDoneWorkspace(_ docId: String) {
        editDoneFilter { $0.docs.toggle(docId) }
    }

    /// Check or uncheck a list (`DoneListOption.key`) in the Done filter.
    func toggleDoneList(_ key: String) {
        editDoneFilter { $0.lists.toggle(key) }
    }

    func clearDoneFilter() {
        editDoneFilter { $0 = DoneFilter(docs: [], lists: []) }
    }

    /// An edit starts from the filter in effect, so a toggle also drops
    /// the stale ids (a closed workspace, a deleted list) the choice
    /// was carrying.
    private func editDoneFilter(_ edit: (inout DoneFilter) -> Void) {
        var next = doneFilter
        edit(&next)
        doneFilterChoice = next
        DoneFilterStore.save(next)
        reload()
    }

    /// Where a Calendar row's item lives, for the line under its text:
    /// the list, without its emoji, then the workspace when it is not
    /// Personal's. An item with no list and a date lives on the
    /// Calendar itself, so that says nothing; one there by its deadline
    /// alone reads as the Inbox's.
    func agendaListName(_ row: AgendaEntry) -> String? {
        var name: String?
        if row.listId != Self.inbox {
            name = sections.first { $0.workspace.docId == row.docId }?
                .lists.first { $0.id == row.listId }?.name
        }
        if name == nil, row.when == nil { name = Self.noListLabel(dated: false) }
        let parts = [name, row.docName].compactMap { $0 }.filter { !$0.isEmpty }
        return parts.isEmpty ? nil : parts.joined(separator: " · ")
    }

    func add(_ text: String) {
        let text = text.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !text.isEmpty else { return }
        guard shown.lens == nil else { return }
        let workspace = shown.workspace, list = shown.list
        write { _ = try $0.addItem(workspace: workspace, list: list, text: text, top: false) }
    }

    /// Create a list at the end of a workspace's lists
    /// (`Engine.addList`) and show it; `failed` gets the reason when
    /// the engine refuses.
    func addList(workspace: String?, name: String, failed: @escaping (String) -> Void) {
        let name = name.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !name.isEmpty else { return }
        guard let calls = engine.calls else {
            return failed(engineErrorMessage(EngineError.Stopped))
        }
        calls.run({ try $0.addList(workspace: workspace, name: name) }) { [weak self] result in
            switch result {
            case .success(let id): self?.select(workspace: workspace, list: id)
            case .failure(let error): failed(engineErrorMessage(error))
            }
        }
    }

    /// Delete a workspace the account owns (`Engine.deleteWorkspace`):
    /// gone from every device and every member. `failed` gets why not.
    func deleteWorkspace(_ docId: String, failed: @escaping (String) -> Void) {
        guard let calls = engine.calls else {
            return failed(engineErrorMessage(EngineError.Stopped))
        }
        calls.run({ try $0.deleteWorkspace(docId: docId) }) { [weak self] result in
            switch result {
            case .success:
                self?.ownsWorkspace[docId] = nil
                self?.reload()
            case .failure(let error): failed(engineErrorMessage(error))
            }
        }
    }

    /// Rename a shared workspace (`Engine.renameWorkspace`), for every
    /// member. A blank or unchanged name writes nothing. `failed` gets
    /// why not.
    func renameWorkspace(_ docId: String, name: String, failed: @escaping (String) -> Void) {
        let name = name.trimmingCharacters(in: .whitespacesAndNewlines)
        let current = sections.first { $0.id == docId }?.workspace.name
        guard !name.isEmpty, name != current else { return }
        guard let calls = engine.calls else {
            return failed(engineErrorMessage(EngineError.Stopped))
        }
        calls.run({ try $0.renameWorkspace(workspace: docId, name: name) }) { [weak self] result in
            switch result {
            case .success: self?.reload()
            case .failure(let error): failed(engineErrorMessage(error))
            }
        }
    }

    /// A workspace as JSON (`Engine.exportJson`), for Export JSON….
    /// `done` gets it, or why not, on the main queue.
    func exportJson(workspace: String?, done: @escaping (Result<String, Error>) -> Void) {
        guard let calls = engine.calls else { return done(.failure(EngineError.Stopped)) }
        calls.run({ try $0.exportJson(workspace: workspace) }, completion: done)
    }

    /// Add an export's lists and items to a workspace
    /// (`Engine.importJson`), for Import JSON…. `done` gets the counts,
    /// or why not, on the main queue.
    func importJson(
        workspace: String?, json: String, done: @escaping (Result<ImportCounts, Error>) -> Void
    ) {
        guard let calls = engine.calls else { return done(.failure(EngineError.Stopped)) }
        calls.run({ try $0.importJson(workspace: workspace, json: json) }) { [weak self] result in
            if case .success = result { self?.reload() }
            done(result)
        }
    }

    /// Leave a shared workspace the account does not own
    /// (`Engine.leaveWorkspace`). The engine forgets it once the
    /// server's `DocClosed` lands, and the `Changed` that follows
    /// rereads; the reread here only makes it sooner.
    func leaveWorkspace(_ docId: String, failed: @escaping (String) -> Void) {
        sharing({ try $0.leaveWorkspace(docId: docId) }) { [weak self] result in
            switch result {
            case .success:
                self?.ownsWorkspace[docId] = nil
                self?.reload()
            case .failure(let error): failed(engineErrorMessage(error))
            }
        }
    }

    /// One of the engine's sharing calls (`Engine.workspaceMembers`,
    /// `createInvite`, ...). Each is a round trip to the server, so it
    /// runs on a queue of its own rather than the doc calls' serial
    /// one: an unreachable server must not hold up a tick. `done` gets
    /// the result on the main queue.
    func sharing<T>(
        _ body: @escaping (Engine) throws -> T,
        done: @escaping (Result<T, Error>) -> Void
    ) {
        guard let engine = engine.calls?.engine else {
            DispatchQueue.main.async { done(.failure(EngineError.Stopped)) }
            return
        }
        DispatchQueue.global(qos: .userInitiated).async {
            let result = Result { try body(engine) }
            DispatchQueue.main.async { done(result) }
        }
    }

    /// What a member list says of the account's role on a workspace:
    /// the Sharing sheet reports each list it loads, so the menu
    /// follows a change it shows.
    func noteMembers(_ members: [WorkspaceMember], of docId: String) {
        guard let me = accountId, !me.isEmpty else { return }
        ownsWorkspace[docId] = members.first(where: { $0.accountId == me })?.owner ?? false
        memberCounts[docId] = members.count
        self.members[docId] = members.sorted { $0.email < $1.email }
    }

    /// Whether a workspace has members besides the account: false for
    /// Home; a shared one whose member list is not known yet (offline,
    /// still loading) counts as having some, so Delete's note errs on
    /// saying who will see.
    func hasOtherMembers(_ workspace: String?) -> Bool {
        guard let workspace,
              sections.contains(where: { $0.workspace.docId == workspace && !$0.workspace.home })
        else { return false }
        return (memberCounts[workspace] ?? 2) > 1
    }

    /// Ask for the role on every shared workspace not yet known, and
    /// forget the ones that have closed. Once per workspace per run of
    /// the window's model, not per reload: roles only change by
    /// transfer, which is not built.
    private func refreshRoles() {
        guard accountId != nil else {
            ownsWorkspace = [:]
            memberCounts = [:]
            members = [:]
            return
        }
        let shared = Set(sections.filter { !$0.workspace.home }.map(\.id))
        for gone in ownsWorkspace.keys where !shared.contains(gone) { ownsWorkspace[gone] = nil }
        for gone in memberCounts.keys where !shared.contains(gone) { memberCounts[gone] = nil }
        for gone in members.keys where !shared.contains(gone) { members[gone] = nil }
        for docId in shared where ownsWorkspace[docId] == nil && !rolesLoading.contains(docId) {
            rolesLoading.insert(docId)
            sharing({ try $0.workspaceMembers(docId: docId) }) { [weak self] result in
                guard let self else { return }
                rolesLoading.remove(docId)
                // A failure leaves it unknown; the next reload asks again.
                if case .success(let members) = result { noteMembers(members, of: docId) }
            }
        }
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

/// `UserDefaults` key of the Inbox order preference (the Inbox row's
/// right-click menu): newest first when set, else the queue's own
/// oldest first (spec/inbox-plan.md). Display only; the same key as
/// iOS.
let inboxNewestFirstKey = "inboxNewestFirst"

struct MainWindowView: View {
    @ObservedObject var model: MainWindowModel
    /// The device's list-counts preference (Settings), off by default:
    /// whether a list's row carries its Open count, as the web's "Show
    /// list counts in nav".
    @AppStorage(listCountsKey) private var showListCounts = false
    /// The device's Inbox order (`inboxNewestFirstKey`): reverses an
    /// Inbox's rows on screen, never the doc.
    @AppStorage(inboxNewestFirstKey) private var inboxNewestFirst = false
    /// The workspace whose Sharing sheet is up.
    @State private var sharing: SharingTarget?
    /// The workspace row under the pointer: its "…" and chevron show.
    @State private var hoveredWorkspace: String?
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
        let current = model.lens.map(NavTarget.lens) ?? .list(workspace: model.workspace, list: model.list)
        // Every row's label is regular, selected or not. A sidebar
        // `List` whose selection is set on its first render draws that
        // row bold and keeps it bold after the selection moves on
        // (macOS 26, probed 2026-11-15); the explicit weight on each
        // row (`navRow`) overrides it.
        func row<V: View>(_ target: NavTarget, _ content: V) -> some View {
            content.navRow().tag(target)
        }
        return List(selection: Binding<NavTarget?>(
            get: { current },
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
                row(.lens(.starred), HStack {
                    Label { Text("Starred") } icon: { StarOutline() }
                    Spacer()
                    count(model.starred.count)
                })
                row(.lens(.agenda), Label { Text("Calendar") } icon: { NavIcon(image: NavArt.calendar) })
                row(.lens(.done), Label { Text("Done") } icon: { NavIcon(image: NavArt.done) })
                // Only while something is binned, as the web's and iOS's
                // nav; counted, as the web's.
                if !model.binned.isEmpty {
                    row(.lens(.bin), HStack {
                        Label { Text("Bin") } icon: { NavIcon(image: NavArt.bin) }
                        Spacer()
                        count(model.binned.count)
                    }
                    .contextMenu { emptyBinButton })
                }
            }
            ForEach(model.sections) { section in
                Section {
                    workspaceRow(section)
                    if !model.collapsed.contains(section.id) {
                        row(.list(workspace: section.target, list: MainWindowModel.inbox), HStack {
                            Label { Text("Inbox") } icon: { NavIcon(image: NavArt.inbox) }
                            Spacer()
                            count(section.inboxOpen)
                        }
                        .contextMenu { inboxOrderPicker })
                        ForEach(section.userLists) { list in
                            row(.list(workspace: section.target, list: list.id), HStack {
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
                            .contextMenu { listMenu(section, list) })
                        }
                    }
                }
            }
        }
        .listStyle(.sidebar)
        // Opaque: the pane's own colour, to the window's top, under
        // the bar.
        .scrollContentBackground(.hidden)
        .background(Color(nsColor: PaneColor.sidebar).ignoresSafeArea())
        // Rows of 28 pt, between the system's small (24) and medium
        // (32), which has no such step: the small size, with each row
        // (`navRow`) at the medium's type and 4 pt taller.
        .environment(\.sidebarRowSize, .small)
        .sheet(item: $sharing) { target in
            SharingSheet(target: target, model: model)
        }
    }

    /// A workspace's heading, a row of its own the size of a list's
    /// (not the sidebar's section header, whose small type and hover
    /// chevron are the system's and take no menu): the name (regular,
    /// muted), then at the trailing edge the "…" menu and the fold
    /// chevron, both muted and shown only while the pointer is over
    /// the row, as the system header's chevron. A click anywhere on it
    /// folds the group. The "…" menu is the web's workspace menu:
    /// Share… and Rename… (a name field in an alert) on a shared
    /// workspace, Export JSON… and Import JSON… on
    /// every one (Personal included), then on a shared workspace
    /// Delete Workspace… to its owner or Leave Workspace… to anyone
    /// else, neither until the role is known
    /// (`MainWindowModel.ownsWorkspace`). Right-click opens the same
    /// menu. Not selectable.
    private func workspaceRow(_ section: NavSection) -> some View {
        let folded = model.collapsed.contains(section.id)
        let name = section.workspace.home
            ? String(localized: "Personal")
            : section.workspace.name ?? section.workspace.docId
        // Hidden by opacity, not removed, so the row's layout holds
        // and an open menu keeps its anchor when the pointer leaves.
        let controls = hoveredWorkspace == section.id ? 1.0 : 0.0
        // One colour for both controls, as a solid system colour: a
        // borderless menu draws its label in its own tint and ignores
        // a hierarchical style, so the "…" and the chevron drifted.
        let muted = Color(nsColor: .tertiaryLabelColor)
        return HStack(spacing: 2) {
            Text(verbatim: name).foregroundStyle(.secondary).lineLimit(1)
            Spacer(minLength: 0)
            Menu {
                workspaceMenu(section, name: name)
            } label: {
                Image(systemName: "ellipsis")
                    .foregroundStyle(muted)
                    .frame(width: 20, height: 20)
                    .contentShape(Rectangle())
            }
            // A plain button's menu draws the label as given.
            .menuStyle(.button)
            .buttonStyle(.plain)
            .menuIndicator(.hidden)
            .fixedSize()
            .hoverCircle()
            .opacity(controls)
            .accessibilityLabel(Text("Workspace Actions"))
            .help(Text("Workspace Actions"))
            Image(systemName: "chevron.right")
                .font(.caption.weight(.semibold))
                .foregroundStyle(muted)
                .rotationEffect(.degrees(folded ? 0 : 90))
                .frame(width: 20, height: 20)
                .hoverCircle()
                .opacity(controls)
                .accessibilityHidden(true)
        }
        .font(.body)
        .imageScale(.medium)
        .frame(minHeight: 20)
        .contentShape(Rectangle())
        .onTapGesture {
            withAnimation(.easeInOut(duration: 0.15)) { model.setCollapsed(section.id, !folded) }
        }
        .onHover { inside in
            if inside {
                hoveredWorkspace = section.id
            } else if hoveredWorkspace == section.id {
                hoveredWorkspace = nil
            }
        }
        .contextMenu { workspaceMenu(section, name: name) }
        .selectionDisabled()
        .accessibilityAddTraits(.isHeader)
        .accessibilityAction(named: folded ? Text("Expand") : Text("Collapse")) {
            model.setCollapsed(section.id, !folded)
        }
    }

    @ViewBuilder
    private func workspaceMenu(_ section: NavSection, name: String) -> some View {
        let docId = section.id
        let target = section.target
        if !section.workspace.home {
            Button("Share…") { sharing = SharingTarget(docId: docId, name: name) }
            Button("Rename…") {
                // An unnamed workspace shows its id; the field starts empty.
                let window = NSApp.keyWindow
                WorkspaceConfirm.rename(name: section.workspace.name ?? "", on: window) { renamed in
                    model.renameWorkspace(docId, name: renamed) { error in
                        WorkspaceConfirm.failed(
                            String(localized: "Could not rename the workspace"), error, on: window)
                    }
                }
            }
            Divider()
        }
        Button("Export JSON…") {
            WorkspaceJSON.save(workspace: target, name: name, model: model, on: NSApp.keyWindow)
        }
        Button("Import JSON…") {
            WorkspaceJSON.load(workspace: target, model: model, on: NSApp.keyWindow)
        }
        switch section.workspace.home ? nil : model.ownsWorkspace[docId] {
        case true?:
            Divider()
            Button("Delete Workspace…") {
                WorkspaceConfirm.delete(name: name, on: NSApp.keyWindow) {
                    model.deleteWorkspace(docId) { error in
                        WorkspaceConfirm.failed(
                            String(localized: "Could not delete the workspace"), error, on: NSApp.keyWindow)
                    }
                }
            }
        case false?:
            Divider()
            Button("Leave Workspace…") {
                WorkspaceConfirm.leave(name: name, on: NSApp.keyWindow) {
                    model.leaveWorkspace(docId) { error in
                        WorkspaceConfirm.failed(
                            String(localized: "Could not leave the workspace"), error, on: NSApp.keyWindow)
                    }
                }
            }
        case nil:
            EmptyView()
        }
    }

    /// The Inbox's order, oldest first (the queue's own) or newest
    /// first: a preference of this Mac's, for every Inbox.
    private var inboxOrderPicker: some View {
        Picker("Order", selection: $inboxNewestFirst) {
            Text("Oldest First").tag(false)
            Text("Newest First").tag(true)
        }
        .pickerStyle(.inline)
    }

    /// A user list's right-click menu: Delete, which confirms and bins
    /// its open items (`ListConfirm`).
    @ViewBuilder
    private func listMenu(_ section: NavSection, _ list: ListInfo) -> some View {
        let target = section.target
        Button("Delete") {
            let shared = model.hasOtherMembers(section.id)
                ? section.workspace.name ?? section.workspace.docId : nil
            ListConfirm.delete(name: list.name, sharedWith: shared, on: NSApp.keyWindow) {
                model.deleteList(workspace: target, list: list.id, mode: .toBin)
            }
        }
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
            case .bin: bin
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
                // An Inbox is a queue in arrival order: shown newest
                // first when this Mac says so, and never reordered by
                // hand (no row drag, here or anywhere).
                let rows = model.shown.list == MainWindowModel.inbox && inboxNewestFirst
                    ? Array(model.items.reversed()) : model.items
                ForEach(Array(rows.enumerated()), id: \.offset) { _, item in
                    HStack(alignment: .center, spacing: 8) {
                        ItemCheckButton(state: item.state) { _ in model.tick(item) }
                        Text(verbatim: item.text)
                            .lineLimit(1)
                            .truncationMode(.tail)
                        Spacer()
                        if model.starredIds.contains(item.id) { starMark }
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

    /// The Calendar: the engine's agenda as it groups it. The days
    /// gone by that have rows (the events that happened on them; a
    /// task done today is at the end of Today, muted, and one done
    /// earlier is not here), then Overdue while something is,
    /// then Today, always there, then each non-empty day. It opens on
    /// Today's header (Overdue's, when it is there), the past above
    /// it, and the toolbar's "Today" comes back
    /// there: an AppKit table (`AgendaTable.swift`), which can be put
    /// on a row exactly. A row has its tick, its text with its star right after it,
    /// under that where it lives (the list, the workspace when it is
    /// not Personal's) and its place, then at the far edge the tone's
    /// note and its clock time when timed ("All day" for an event
    /// with none). A row with that second line
    /// is taller.
    private var agenda: some View {
        AgendaTable(
            groups: model.agenda,
            selected: model.selected?.id,
            todayJumps: model.todayJumps,
            heading: Self.heading(for:),
            rowHeight: { agendaRowDetailed($0) ? 42 : 24 },
            row: { row in AnyView(agendaRow(row)) },
            select: { row in model.selectItem(workspace: row?.docId, id: row?.id) },
            // The other lists' `itemMenu`, as the table's own menu.
            menu: { row in
                [
                    .init(title: String(localized: "Duplicate")) {
                        model.duplicate(workspace: row.docId, id: row.id)
                    },
                    nil,
                    .init(title: String(localized: "Move to Bin")) {
                        model.bin(workspace: row.docId, id: row.id)
                    },
                ]
            })
            // Under the toolbar, as a list is: the scroll view insets
            // its own content for it.
            .ignoresSafeArea()
    }

    /// Whether a Calendar row has its second line (where it lives, its
    /// place): such a row is taller.
    private func agendaRowDetailed(_ row: AgendaEntry) -> Bool {
        model.agendaListName(row) != nil || !(row.place?.label ?? "").isEmpty
    }

    /// Whether a Calendar row leads with a checkbox. An event's own
    /// row does not while it is open (spec/events-plan.md: nothing asks
    /// for the tick) or once its day has gone by (it is a record of
    /// what was on): the event pill holds the box's place. One marked
    /// done from today on shows the box, ticked, and a deadline's own
    /// row is owed, so it keeps it.
    static func agendaRowShowsCheck(_ row: AgendaEntry, today: String = DateOffers.today()) -> Bool {
        guard row.event, row.placedBy == .when else { return true }
        return row.state.isClosed && !agendaRowIsPast(row, today: today)
    }

    /// An event's own row on a day gone by: the only rows the past
    /// days hold. (A task whose day has passed is in Overdue, not
    /// there.)
    static func agendaRowIsPast(_ row: AgendaEntry, today: String = DateOffers.today()) -> Bool {
        guard row.event, row.placedBy == .when, let when = row.when, when.count >= 10 else { return false }
        return String(when.prefix(10)) < today
    }

    private func agendaRow(_ row: AgendaEntry) -> some View {
        // Muted: a done row, and every row of a day gone by, done or
        // not.
        let done = row.state == .done || Self.agendaRowIsPast(row)
        let check = Self.agendaRowShowsCheck(row)
        let listName = model.agendaListName(row)
        let place = row.place?.label ?? ""
        let detailed = agendaRowDetailed(row)
        // The date and time sit on the text's line too, not the middle
        // of a two-line row.
        return HStack(alignment: .rowTitle, spacing: 8) {
            // The tick sits on the text's line, not the row's middle.
            HStack(alignment: .rowTitle, spacing: 8) {
                // An event's row has the event pill for a checkbox
                // (`agendaRowShowsCheck`), drawn over the row below.
                if check {
                    ItemCheckButton(state: row.state) { model.setDone(workspace: row.docId, id: row.id, done: $0) }
                } else {
                    EventSlot()
                }
                VStack(alignment: .leading, spacing: 4) {
                    // The star follows the text, not the row's far edge.
                    HStack(alignment: .center, spacing: 6) {
                        Text(verbatim: row.text)
                            .lineLimit(1)
                            .truncationMode(.tail)
                            .foregroundStyle(done ? .secondary : .primary)
                        if model.starredIds.contains(row.id) { starMark }
                    }
                    .alignmentGuide(.rowTitle) { $0[VerticalAlignment.center] }
                    if detailed {
                        HStack(alignment: .center, spacing: 8) {
                            if let listName {
                                // The list's own icon, when it has one.
                                Text(verbatim: [row.listIcon, listName].compactMap { $0 }.joined(separator: " "))
                                    .lineLimit(1)
                            }
                            if !place.isEmpty {
                                HStack(alignment: .center, spacing: 2) {
                                    NavIcon(image: NavArt.pin, size: 12).accessibilityHidden(true)
                                    Text(verbatim: place).lineLimit(1)
                                }
                            }
                        }
                        .foregroundStyle(.tertiary)
                    }
                }
            }
            Spacer(minLength: 8)
            switch row.tone {
            case .overdue:
                // The day it was scheduled for, as the other stamps are
                // set; the Overdue header already says it is late.
                if let day = Self.overdueDay(row) {
                    Text(verbatim: day).foregroundStyle(.secondary).monospacedDigit().fixedSize()
                }
            case .warning: Text("Due today").font(.footnote).foregroundStyle(.orange).fixedSize()
            case .neutral: EmptyView()
            }
            if TodayMenu.isTimed(row), let when = row.when {
                Text(verbatim: String(when.dropFirst(11).prefix(5)))
                    .foregroundStyle(.secondary)
                    .monospacedDigit()
                    .fixedSize()
            } else if row.placedBy == .deadline {
                // A deadline's own row. One still ahead says so here;
                // today's and a late one are toned above.
                if row.tone == .neutral, let deadline = row.deadline {
                    Text("Due \(ItemPanel.dayText(deadline))").foregroundStyle(.secondary).fixedSize()
                }
            } else if row.event, row.when != nil {
                // An event with a day and no clock time.
                Text("All day").foregroundStyle(.secondary).fixedSize()
            }
        }
        // The pill runs the height of the row, so it spans the second
        // line too.
        .overlay(alignment: .leading) {
            if !check { EventPill() }
        }
        .itemRow(height: detailed ? 42 : 24)
    }

    /// Starred: Home's curated order over every workspace. A row has
    /// its tick, its text and where it lives (the list, and the
    /// workspace when it is not Personal's). No star on a row: every
    /// one here has it. The item's panel takes it off.
    private var starred: some View {
        List(selection: selection { id in model.starred.first { $0.id == id }?.docId }) {
            ForEach(model.starred, id: \.id) { row in
                let done = row.state == .done
                HStack(alignment: .center, spacing: 8) {
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

    /// Done: every workspace's newest Done items that the filter
    /// (`DoneFilter.swift`) admits, newest first. The
    /// tick puts a row back. As the web's Done rows: the tick, when it
    /// entered Done, the text, and the list it is on at the far edge.
    private var done: some View {
        List(selection: selection { id in model.done.first { $0.item.id == id }?.workspace }) {
            ForEach(model.done) { row in
                HStack(alignment: .center, spacing: 8) {
                    ItemCheckButton(state: row.item.state) {
                        model.setDone(workspace: row.workspace, id: row.item.id, done: $0)
                    }
                    // As wide as it reads, so the text follows it at
                    // the same gap on every row.
                    Text(verbatim: Self.stamp(row.item.lifecycleAt))
                        .foregroundStyle(.secondary)
                        .monospacedDigit()
                        .fixedSize()
                    Text(verbatim: row.item.text)
                        .lineLimit(1)
                        .truncationMode(.tail)
                        .foregroundStyle(.secondary)
                    Spacer()
                    Text(verbatim: model.doneListName(row))
                        .font(.footnote)
                        .foregroundStyle(.tertiary)
                        .lineLimit(1)
                }
                .itemRow()
                .itemMenu(model, workspace: row.workspace, id: row.item.id)
                .tag(row.item.id)
            }
            if model.done.isEmpty {
                if model.doneFilter.isEmpty {
                    Text("Nothing here").foregroundStyle(.secondary)
                } else {
                    Text("No finished items match the filter.").foregroundStyle(.secondary)
                }
            }
        }
        .listStyle(.inset)
        .scrollContentBackground(.hidden)
    }

    /// Empty, asked first: the sidebar row's menu and the Bin's bar.
    private var emptyBinButton: some View {
        Button("Empty") {
            BinConfirm.empty(count: model.binned.count, on: NSApp.keyWindow) { model.emptyBin() }
        }
        .disabled(model.binned.isEmpty)
    }

    /// The Bin: every workspace's binned items, newest binned first. A
    /// row has its text and, at its trailing edge, the list it would go
    /// back to (and the workspace when not Personal's); its menu
    /// restores or deletes it. A bar over the rows empties the lot, as
    /// Finder's Trash has it. Both deletes are for good, so both ask.
    /// On macOS 26 the bar is a safe-area bar, as Finder's is: no
    /// divider, the rows scroll under it behind the system's edge
    /// effect, so it reads as part of the toolbar. Before that, a
    /// plain bar over a divider.
    @ViewBuilder private var bin: some View {
        if #available(macOS 26, *) {
            // The hard style: a sharp edge under the bar, as for a
            // pinned header, not the soft fade.
            binList
                .safeAreaBar(edge: .top) { binBar }
                .scrollEdgeEffectStyle(.hard, for: .top)
        } else {
            VStack(spacing: 0) {
                binBar
                Divider()
                binList
            }
        }
    }

    /// The Bin's bar, as Finder's Trash has it: its name leading,
    /// Empty at its trailing edge, small.
    private var binBar: some View {
        HStack(spacing: 8) {
            Text("Bin").fontWeight(.semibold)
            Spacer()
            emptyBinButton.controlSize(.small)
        }
        .padding(.horizontal, 16)
        .padding(.vertical, 6)
    }

    private var binList: some View {
        List(selection: selection { id in model.binned.first { $0.id == id }?.docId }) {
            ForEach(model.binned, id: \.binKey) { row in
                HStack(alignment: .center, spacing: 8) {
                    Text(verbatim: row.text)
                        .lineLimit(1)
                        .truncationMode(.tail)
                    Spacer()
                    let place = [model.binnedListName(row), model.sharedWorkspaceName(row.docId)]
                        .compactMap { $0 }.joined(separator: " · ")
                    if !place.isEmpty {
                        Text(verbatim: place).font(.footnote).foregroundStyle(.tertiary).lineLimit(1)
                    }
                }
                .itemRow()
                .binnedMenu(model, workspace: row.docId, id: row.id)
                .tag(row.id)
            }
            if model.binned.isEmpty {
                Text("Nothing here").foregroundStyle(.secondary)
            }
        }
        .listStyle(.inset)
        .scrollContentBackground(.hidden)
    }

    /// An overdue Calendar row's scheduled day (of the field that placed
    /// it), with its year once that differs. Nil for one scheduled
    /// earlier today: its time alone says when.
    static func overdueDay(_ row: AgendaEntry, now: Date = Date(), calendar: Calendar = .current) -> String? {
        guard let scheduled = row.placedBy == .deadline ? row.deadline : row.when,
              let date = Agenda.start(of: scheduled, calendar: calendar),
              !calendar.isDate(date, inSameDayAs: now)
        else { return nil }
        if calendar.isDate(date, equalTo: now, toGranularity: .year) {
            return date.formatted(.dateTime.day().month(.abbreviated))
        }
        return date.formatted(.dateTime.day().month(.abbreviated).year())
    }

    /// A Done row's moment (unix ms): the time of day for today, the
    /// day of the week for the six days before it (further back a
    /// weekday would name the wrong week), then the date, with its year
    /// once that differs.
    static func stamp(_ millis: Int64, now: Date = Date(), calendar: Calendar = .current) -> String {
        let date = Date(timeIntervalSince1970: TimeInterval(millis) / 1000)
        let days = calendar.dateComponents(
            [.day], from: calendar.startOfDay(for: date), to: calendar.startOfDay(for: now)
        ).day ?? 0
        if days <= 0 { return date.formatted(date: .omitted, time: .shortened) }
        if days < 7 { return date.formatted(.dateTime.weekday(.abbreviated)) }
        if calendar.isDate(date, equalTo: now, toGranularity: .year) {
            return date.formatted(.dateTime.day().month(.abbreviated))
        }
        return date.formatted(.dateTime.day().month(.abbreviated).year())
    }

    /// "Today · 14 Nov" or the day spelled out ("Friday 20 Nov").
    static func heading(for group: AgendaGroup) -> String {
        if group.today {
            let date = Agenda.start(of: group.day) ?? Date()
            return String(localized: "Today · \(date.formatted(.dateTime.day().month(.abbreviated)))")
        }
        guard let date = Agenda.start(of: group.day) else { return group.day }
        return date.formatted(.dateTime.weekday(.wide).day().month(.abbreviated))
    }
}

private extension VerticalAlignment {
    /// The middle of a two-line row's first line: what sits beside the
    /// text (the tick) lines up with it, not with both lines.
    enum RowTitle: AlignmentID {
        static func defaultValue(in context: ViewDimensions) -> CGFloat { context[VerticalAlignment.center] }
    }

    static let rowTitle = VerticalAlignment(RowTitle.self)
}

private extension View {
    /// A sidebar row's size: see the sidebar's `sidebarRowSize`.
    func navRow() -> some View {
        font(.body).fontWeight(.regular)
            .imageScale(.medium).frame(minHeight: 20)
    }

    /// An item row's size, fixed: 32 pt, the web's row height
    /// (`ROW_HEIGHT` in `js/web/src/Workspace.tsx`). The list adds 4 pt
    /// above and below the content. The line between two rows is the
    /// web's too. A row with a second line (the Calendar's) passes its
    /// own height.
    func itemRow(height: CGFloat = 24) -> some View {
        frame(height: height)
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

    /// A binned row's right-click menu: the same actions as the
    /// panel's "…" button for a binned item.
    func binnedMenu(_ model: MainWindowModel, workspace: String?, id: String) -> some View {
        contextMenu {
            Button("Restore") { model.restore(workspace: workspace, id: id) }
            Divider()
            Button("Delete") {
                BinConfirm.delete(on: NSApp.keyWindow) { model.deleteBinned(workspace: workspace, id: id) }
            }
        }
    }
}

extension BinEntry {
    /// The row's identity across workspaces.
    var binKey: String { "\(docId ?? "")/\(id)" }
}

/// The Bin's two deletes are for good: each asks first, as a sheet on
/// the window (a bare alert without one).
enum BinConfirm {
    static func delete(on window: NSWindow?, then confirmed: @escaping () -> Void) {
        ask(
            String(localized: "Delete this item permanently?"),
            String(localized: "It is removed from every device and cannot be restored."),
            button: String(localized: "Delete"), on: window, then: confirmed)
    }

    static func empty(count: Int, on window: NSWindow?, then confirmed: @escaping () -> Void) {
        guard count > 0 else { return }
        ask(
            count == 1
                ? String(localized: "Delete 1 item permanently?")
                : String(localized: "Delete all \(count) items permanently?"),
            String(localized: "They are removed from every device and cannot be restored."),
            button: String(localized: "Empty Bin"), on: window, then: confirmed)
    }

    private static func ask(
        _ message: String, _ info: String, button: String, on window: NSWindow?,
        then confirmed: @escaping () -> Void
    ) {
        let alert = NSAlert()
        alert.alertStyle = .warning
        alert.messageText = message
        alert.informativeText = info
        alert.addButton(withTitle: button).hasDestructiveAction = true
        alert.addButton(withTitle: String(localized: "Cancel"))
        if let window {
            alert.beginSheetModal(for: window) { if $0 == .alertFirstButtonReturn { confirmed() } }
        } else if alert.runModal() == .alertFirstButtonReturn {
            confirmed()
        }
    }
}

/// A move across workspaces that changes who can see the item asks
/// first, as the web's dialog: into a workspace with other members it
/// says who will see it, out of one that it goes for everyone there.
enum MoveConfirm {
    /// `exposes` is set when the target has other members: how many
    /// members in all, nil inside when the count is not known.
    /// `removesFrom` names a source that has other members.
    static func across(
        to target: String, exposes: Int??, removesFrom source: String?, on window: NSWindow?,
        then confirmed: @escaping () -> Void
    ) {
        var lines: [String] = []
        if let exposes {
            lines.append(
                exposes.map { String(localized: "This item will be visible to all \($0) members of \(target).") }
                    ?? String(localized: "This item will be visible to everyone in \(target)."))
        }
        if let source {
            lines.append(String(localized: "It will be removed from \(source) for everyone there."))
        }
        let alert = NSAlert()
        alert.messageText = String(localized: "Move to \(target)?")
        alert.informativeText = lines.joined(separator: " ")
        alert.addButton(withTitle: String(localized: "Move"))
        alert.addButton(withTitle: String(localized: "Cancel"))
        let finish = { (response: NSApplication.ModalResponse) in
            if response == .alertFirstButtonReturn { confirmed() }
        }
        if let window {
            alert.beginSheetModal(for: window, completionHandler: finish)
        } else {
            finish(alert.runModal())
        }
    }
}

/// Delete on a user list confirms, saying its open items go to the Bin, as
/// the web's dialog. On a workspace with other members it says they
/// will all see the result.
enum ListConfirm {
    static func delete(
        name: String, sharedWith workspace: String?, on window: NSWindow?,
        then confirmed: @escaping () -> Void
    ) {
        let alert = NSAlert()
        alert.alertStyle = .warning
        alert.messageText = String(localized: "Delete “\(name)”?")
        alert.informativeText = workspace.map {
            String(localized: "Its open items go to the Bin. Everyone in \($0) will see them.")
        } ?? String(localized: "Its open items go to the Bin.")
        alert.addButton(withTitle: String(localized: "Delete")).hasDestructiveAction = true
        alert.addButton(withTitle: String(localized: "Cancel"))
        let finish = { (response: NSApplication.ModalResponse) in
            if response == .alertFirstButtonReturn { confirmed() }
        }
        if let window {
            alert.beginSheetModal(for: window, completionHandler: finish)
        } else {
            finish(alert.runModal())
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

/// A translucent disc behind a small sidebar control while the pointer
/// is over it, as a toolbar button's hover: the workspace row's "…"
/// and chevron.
private struct HoverCircle: ViewModifier {
    @State private var hovered = false

    func body(content: Content) -> some View {
        content
            .background(Circle().fill(Color.primary.opacity(hovered ? 0.1 : 0)))
            .onHover { hovered = $0 }
            .animation(.easeOut(duration: 0.1), value: hovered)
    }
}

extension View {
    fileprivate func hoverCircle() -> some View { modifier(HoverCircle()) }
}
