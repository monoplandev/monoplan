// The window: native SwiftUI over the engine's typed calls
// (spec/swift-bindings-plan.md "The macOS window"), replacing the
// webview. Deliberately a debug screen for now: a session line (who,
// which server, sync), a sidebar with one group per open workspace
// stacked as on the web (Personal first, each a collapsible heading over
// its Inbox, Calendar and lists), and the open items of the selected
// list with a tick and a field to add one. No board, no drag and drop,
// no item editor; those come from sharing views with iOS.
//
// It holds no doc and no state of its own beyond the selection: every
// read is a typed call, made again whenever the engine says a doc
// changed and after each of its own writes. Nothing is optimistic.
//
// While the window is open the app is a regular app (Dock icon, main
// menu); closed, it goes back to a bare status item.

import AppKit
import SwiftUI
import MonoplanCore

final class MainWindowController: NSWindowController, NSWindowDelegate {
    private let model: MainWindowModel
    private let onVisibility: (Bool) -> Void
    private var newWorkspacePrompt: NewWorkspacePrompt?

    init(engine: EngineController, onVisibility: @escaping (Bool) -> Void) {
        model = MainWindowModel(engine: engine)
        self.onVisibility = onVisibility
        let window = NSWindow(contentViewController: NSHostingController(rootView: MainWindowView(model: model)))
        window.title = "Monoplan"
        window.styleMask = [.titled, .closable, .miniaturizable, .resizable]
        window.isReleasedWhenClosed = false
        window.minSize = NSSize(width: 520, height: 360)
        window.setContentSize(NSSize(width: 760, height: 520))
        window.center()
        // After the default frame, so a saved one wins.
        window.setFrameAutosaveName("MonoplanWindow")
        super.init(window: window)
        window.delegate = self
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }

    /// Bring the window forward, on a list when one is given (a tray
    /// row: the item's list in its workspace, nil being Home's).
    func show(list: String? = nil, workspace: String? = nil) {
        onVisibility(true)
        model.visible = true
        if let list {
            model.select(workspace: workspace, list: list)
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

    /// The lists shown under Inbox and Calendar: active ones, in the
    /// doc's order, without the reserved ids (which have no `ListMeta`
    /// row of their own).
    var userLists: [ListInfo] {
        lists.filter {
            $0.archivedAt == nil && $0.id != MainWindowModel.inbox && $0.id != MainWindowModel.events
        }
    }
}

/// A sidebar row: a list in a workspace (nil being Home).
struct NavTarget: Hashable {
    let workspace: String?
    let list: String
}

/// What the window shows. Main queue throughout; the reads run on the
/// engine's call queue and come back here.
final class MainWindowModel: ObservableObject {
    @Published private(set) var state: EngineState
    @Published private(set) var account: AccountInfo?
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

    init(engine: EngineController) {
        self.engine = engine
        state = engine.state
        collapsed = Set(UserDefaults.standard.stringArray(forKey: Self.collapsedKey) ?? [])
    }

    /// The first line of the window: who is signed in and where, then
    /// what the engine is doing.
    var sessionLine: String {
        let who = account.map { "\($0.email) · \($0.serverUrl)" } ?? String(localized: "Local workspace (no account)")
        switch state {
        case .connecting(let reason): return "\(who) · " + String(localized: "Not running · \(reason)")
        case .connected(let sync): return "\(who) · \(sync.line())"
        }
    }

    /// The selected list's name, for the items' heading.
    var listTitle: String {
        switch list {
        case Self.inbox: return String(localized: "Inbox")
        case Self.events: return String(localized: "Calendar")
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

    func select(workspace: String?, list: String) {
        reveal = true
        if workspace == self.workspace, list == self.list {
            reload()
            return
        }
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

    private func clear() {
        account = nil
        sections = []
        items = []
        generation += 1
    }

    private struct Snapshot {
        var account: AccountInfo?
        var sections: [NavSection]
        var workspace: String?
        var list: String
        var items: [Item]
    }

    /// Read everything shown, in one turn of the call queue so the
    /// parts agree with each other.
    func reload() {
        guard visible, let calls = engine.calls else { return }
        let workspace = workspace, list = list, generation = generation
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
            let known = list == Self.inbox || list == Self.events || lists.contains { $0.id == list }
            let shown = !gone && known ? list : Self.inbox
            return Snapshot(
                account: try engine.account(), sections: sections, workspace: selected,
                list: shown,
                items: try engine.items(workspace: selected, list: shown, includeDone: false))
        }) { [weak self] result in
            guard let self, generation == self.generation else { return }
            switch result {
            case .success(let snapshot):
                self.account = snapshot.account
                self.sections = snapshot.sections
                self.workspace = snapshot.workspace
                self.list = snapshot.list
                self.items = snapshot.items
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

    var body: some View {
        VStack(spacing: 0) {
            HStack {
                Text(verbatim: model.sessionLine)
                    .font(.footnote)
                    .foregroundStyle(.secondary)
                    .lineLimit(1)
                    .truncationMode(.middle)
                    .textSelection(.enabled)
                Spacer()
            }
            .padding(.horizontal, 12)
            .padding(.vertical, 6)
            Divider()
            HSplitView {
                sidebar
                    .frame(minWidth: 160, idealWidth: 200, maxWidth: 320)
                items
                    .frame(minWidth: 300, maxWidth: .infinity, maxHeight: .infinity)
            }
        }
    }

    /// One group per open workspace, stacked, Home ("Personal") first:
    /// a heading that folds the group, then its Inbox, Calendar and
    /// lists. The selection is a list in a workspace, so picking a row
    /// in another group moves both at once.
    private var sidebar: some View {
        List(selection: Binding<NavTarget?>(
            get: { NavTarget(workspace: model.workspace, list: model.list) },
            set: { if let target = $0 { model.select(workspace: target.workspace, list: target.list) } }
        )) {
            ForEach(model.sections) { section in
                Section(isExpanded: Binding(
                    get: { !model.collapsed.contains(section.id) },
                    set: { model.setCollapsed(section.id, !$0) }
                )) {
                    Text("Inbox").tag(NavTarget(workspace: section.target, list: MainWindowModel.inbox))
                    Text("Calendar").tag(NavTarget(workspace: section.target, list: MainWindowModel.events))
                    ForEach(section.userLists) { list in
                        HStack {
                            Text(verbatim: [list.icon, list.name].compactMap { $0 }.joined(separator: " "))
                                .lineLimit(1)
                            Spacer()
                            if list.open > 0 {
                                Text(verbatim: "\(list.open)").foregroundStyle(.secondary).monospacedDigit()
                            }
                        }
                        .tag(NavTarget(workspace: section.target, list: list.id))
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
            Text(verbatim: model.listTitle)
                .font(.headline)
                .padding(.horizontal, 12)
                .padding(.vertical, 8)
            if let error = model.error {
                Text(verbatim: error)
                    .font(.footnote)
                    .foregroundStyle(.red)
                    .padding(.horizontal, 12)
                    .padding(.bottom, 6)
            }
            List {
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
                            .textSelection(.enabled)
                        Spacer()
                        if let when = item.when ?? item.deadline {
                            Text(verbatim: when).foregroundStyle(.secondary).monospacedDigit()
                        }
                    }
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
