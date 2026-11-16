// The window's chrome, in AppKit as Xcode's or Sketch's is: a split
// view controller of three panes (the glass sidebar, the list, the
// item's panel), each one a SwiftUI view, under one toolbar that the
// split view divides. The toolbar has a separator tracking each
// divider, so what comes after the panel's separator sits over the
// panel wherever its edge is dragged: the status control and the
// star, the iOS item page's bar (`ItemView.bar`). The
// system draws each on its glass. Over the list the bar shows the
// name of what is listed: the bar's own title item, a button, since the
// window's title cannot be clicked; on a user list it opens a popover
// that renames the list.
//
// SwiftUI's own `NavigationSplitView` and `.inspector` were tried
// here first: in a window made by hand their toolbar items do not
// land over the panel.
//

import AppKit
import MonoplanCore
import SwiftUI

private extension NSToolbarItem.Identifier {
    static let itemStatus = NSToolbarItem.Identifier("MonoplanItemStatus")
    static let itemStar = NSToolbarItem.Identifier("MonoplanItemStar")
    /// The item's "…" menu, after the star with no space between, so
    /// the two share one glass. The rows' right-click menu holds the
    /// same actions.
    static let itemMore = NSToolbarItem.Identifier("MonoplanItemMore")
    /// The separator tracking the panel's edge (the system's own
    /// inspector separator wants an inspector item).
    static let panelSeparator = NSToolbarItem.Identifier("MonoplanPanelSeparator")
    /// The list's section, at its trailing edge: search, then new.
    static let search = NSToolbarItem.Identifier("MonoplanSearch")
    static let newItem = NSToolbarItem.Identifier("MonoplanNewItem")
    /// Over the sidebar, at its trailing edge, while there is no
    /// account. Nothing takes its place once there is one: the
    /// account lives in Settings, New Workspace… in the app menu.
    static let signIn = NSToolbarItem.Identifier("MonoplanSignIn")
    /// In Sign In's place once there is an account: the iOS Home's
    /// folder button, a popover that names a new list.
    static let newList = NSToolbarItem.Identifier("MonoplanNewList")
    /// The list's name, in the window title's place, after the
    /// sidebar's separator. On a user list a click renames it.
    static let title = NSToolbarItem.Identifier("MonoplanTitle")
}

final class MainSplitViewController: NSSplitViewController, NSToolbarDelegate {
    private let model: MainWindowModel
    private let sidebarItem: NSSplitViewItem
    private let listItem: NSSplitViewItem
    private let panelItem: NSSplitViewItem
    let toolbar = NSToolbar(identifier: "MonoplanMain")
    /// Set while `sync` moves the panel itself, so the resize it
    /// causes is not read as the user folding the panel away.
    private var syncing = false
    /// Whether the profile has an account: without one the bar leads
    /// with Sign In. The window's controller keeps it and calls `sync`.
    var signedIn = true
    /// Sign In was clicked.
    var onSignIn: () -> Void = {}
    /// The rename popover, while it is up.
    private var renamePopover: NSPopover?
    /// The new-list popover, while it is up.
    private var newListPopover: NSPopover?
    /// Search's overlay (`SearchPanel.swift`).
    private lazy var searchPanel = SearchPanelController(model: model)

    static let sidebarWidth = (min: 160.0, initial: 200.0, max: 400.0)
    static let panelWidth = (min: 240.0, max: 520.0)

    /// The toolbar with the panel shut: the title's section after the
    /// sidebar's separator, with search and new at its end. The
    /// panel's separator is there only with the panel (`panelItems`):
    /// with the panel shut it would draw at the window's edge.
    /// The space between those two keeps each on its own glass:
    /// adjacent buttons would share one.
    private static let fixedItems: [NSToolbarItem.Identifier] = [
        .sidebarTrackingSeparator, .title, .flexibleSpace, .search, .space, .newItem,
    ]

    init(model: MainWindowModel) {
        self.model = model
        sidebarItem = NSSplitViewItem(
            sidebarWithViewController: Self.pane(MainWindowView(model: model, pane: .sidebar)))
        listItem = NSSplitViewItem(
            viewController: Self.pane(MainWindowView(model: model, pane: .list)))
        // A plain pane, not an inspector item: an inspector is glass,
        // and the bar's background shows through it.
        panelItem = NSSplitViewItem(
            viewController: Self.pane(ItemPanelPane(model: model)))
        super.init(nibName: nil, bundle: nil)
        // Before the panes go in: the split view with our dividers.
        let split = PaneSplitView()
        split.isVertical = true
        split.dividerStyle = .thin
        splitView = split
        sidebarItem.minimumThickness = Self.sidebarWidth.min
        sidebarItem.maximumThickness = Self.sidebarWidth.max
        sidebarItem.canCollapse = false
        listItem.minimumThickness = 260
        panelItem.minimumThickness = Self.panelWidth.min
        panelItem.maximumThickness = Self.panelWidth.max
        panelItem.canCollapse = true
        panelItem.canCollapseFromWindowResize = false
        // As a sidebar's: the window's resizing goes to the list.
        panelItem.holdingPriority = sidebarItem.holdingPriority
        panelItem.isCollapsed = true
        addSplitViewItem(sidebarItem)
        addSplitViewItem(listItem)
        addSplitViewItem(panelItem)
        toolbar.delegate = self
        toolbar.displayMode = .iconOnly
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }

    /// A pane's SwiftUI view, sized by the split view alone. Left to
    /// itself a hosting controller turns its content's smallest and
    /// largest sizes into constraints on the pane, and so on the
    /// window: content with nothing in it for a moment (the panel
    /// between two selections) then folds the window down to its bar,
    /// and content wider than its pane pushes the pane off the
    /// toolbar's sections.
    private static func pane<Content: View>(_ view: Content) -> NSViewController {
        let host = NSHostingController(rootView: view)
        host.sizingOptions = []
        return host
    }

    /// The sidebar at its default width, then whatever the dividers
    /// were last left at (the split view saves them under the name).
    /// Once the window has its size.
    func restoreDividers() {
        splitView.setPosition(Self.sidebarWidth.initial, ofDividerAt: 0)
        splitView.autosaveName = "MonoplanMainSplit"
        sync()
    }

    // MARK: The model's state, shown

    /// The panel open on a selection and shut without one, and the
    /// toolbar's item controls for the item it shows. Called on every
    /// change of the model.
    func sync() {
        let open = model.selected != nil
        if panelItem.isCollapsed == open {
            syncing = true
            panelItem.isCollapsed = !open
            syncing = false
        }
        // Over the sidebar, before its separator: Sign In without an
        // account, New List with one; the space before it pushes it to
        // the sidebar's trailing edge.
        let lead: [NSToolbarItem.Identifier] = [.flexibleSpace, signedIn ? .newList : .signIn]
        let leading = toolbar.items.firstIndex { $0.itemIdentifier == .sidebarTrackingSeparator } ?? 0
        if toolbar.items.prefix(leading).map(\.itemIdentifier) != lead {
            for _ in 0..<leading { toolbar.removeItem(at: 0) }
            for (index, identifier) in lead.enumerated() {
                toolbar.insertItem(withItemIdentifier: identifier, at: index)
            }
        }
        let fixed = Self.fixedItems.count + lead.count
        let wanted = panelItems()
        let have = toolbar.items.dropFirst(fixed).map(\.itemIdentifier)
        if have != wanted {
            while toolbar.items.count > fixed {
                toolbar.removeItem(at: toolbar.items.count - 1)
            }
            for identifier in wanted {
                toolbar.insertItem(withItemIdentifier: identifier, at: toolbar.items.count)
            }
        }
        for item in toolbar.items { update(item) }
    }

    /// The panel's part of the bar: nothing with the panel shut; else
    /// its separator, the status once the item is read, and the star
    /// and the "…" menu at the far edge. No close button: Escape, or
    /// its edge dragged shut. Only an Open, unbinned item can be
    /// starred (spec/focus.md); a binned item's "…" menu restores or
    /// deletes it.
    private func panelItems() -> [NSToolbarItem.Identifier] {
        guard model.selected != nil else { return [] }
        var items: [NSToolbarItem.Identifier] = [.panelSeparator]
        guard let item = model.detail else { return items }
        items.append(.itemStatus)
        items.append(.flexibleSpace)
        if item.item.binnedAt == nil, item.closedAt == nil { items.append(.itemStar) }
        items.append(.itemMore)
        return items
    }

    private func update(_ toolbarItem: NSToolbarItem) {
        if toolbarItem.itemIdentifier == .title {
            guard let view = toolbarItem.view as? ToolbarTitleView else { return }
            view.title = model.listTitle
            view.count = model.titleCount
            view.icon = model.titleIcon
            toolbarItem.label = model.windowTitle
            view.renamable = model.renamableList != nil
            return
        }
        guard let item = model.detail else { return }
        switch toolbarItem.itemIdentifier {
        case .itemStatus:
            guard let control = toolbarItem.view as? NSSegmentedControl else { return }
            // A binned item just says so: the bin masks its state.
            if item.item.binnedAt != nil {
                control.segmentCount = 1
                control.setImage(nil, forSegment: 0)
                control.setLabel(String(localized: "Binned"), forSegment: 0)
                control.setToolTip(nil, forSegment: 0)
                control.isEnabled = false
            } else {
                let closed = item.item.state.isClosed
                let hint = closed ? String(localized: "Mark as not done") : String(localized: "Mark as done")
                control.segmentCount = 2
                control.setLabel("", forSegment: 0)
                control.setImage(
                    ItemCheckArt.image(item.item.state, accessibilityDescription: hint), forSegment: 0)
                control.setToolTip(hint, forSegment: 0)
                control.setLabel(ItemPanel.statusLabel(item), forSegment: 1)
                control.setToolTip(String(localized: "Status"), forSegment: 1)
                control.isEnabled = true
            }
            for segment in 0..<control.segmentCount { control.setWidth(0, forSegment: segment) }
            control.sizeToFit()
        case .itemStar:
            let hint = item.starred ? String(localized: "Remove star") : String(localized: "Star")
            toolbarItem.image = StarArt.toolbarImage(starred: item.starred)
            toolbarItem.label = hint
            toolbarItem.toolTip = hint
        case .itemMore:
            // The menu follows the item in and out of the Bin.
            let binned = item.item.binnedAt != nil
            if let menuItem = toolbarItem as? NSMenuToolbarItem,
               (menuItem.menu.items.first?.action == #selector(restoreItem(_:))) != binned {
                menuItem.menu = itemMenu(binned: binned)
            }
        default:
            break
        }
    }

    // MARK: Toolbar

    func toolbarDefaultItemIdentifiers(_ toolbar: NSToolbar) -> [NSToolbarItem.Identifier] {
        Self.fixedItems
    }

    func toolbarAllowedItemIdentifiers(_ toolbar: NSToolbar) -> [NSToolbarItem.Identifier] {
        Self.fixedItems + [.panelSeparator, .signIn, .newList, .itemStatus, .itemStar, .itemMore]
    }

    func toolbar(
        _ toolbar: NSToolbar, itemForItemIdentifier identifier: NSToolbarItem.Identifier,
        willBeInsertedIntoToolbar flag: Bool
    ) -> NSToolbarItem? {
        if identifier == .panelSeparator {
            return NSTrackingSeparatorToolbarItem(identifier: identifier, splitView: splitView, dividerIndex: 1)
        }
        if identifier == .itemMore {
            let item = NSMenuToolbarItem(itemIdentifier: identifier)
            let label = String(localized: "More")
            item.image = NSImage(systemSymbolName: "ellipsis", accessibilityDescription: label)
            item.label = label
            item.toolTip = label
            item.showsIndicator = false
            item.menu = itemMenu(binned: model.detail?.item.binnedAt != nil)
            return item
        }
        let item = NSToolbarItem(itemIdentifier: identifier)
        switch identifier {
        case .title:
            // Text with a caret on hover, not a bordered control: no
            // glass. The click does nothing off a user list.
            let view = ToolbarTitleView()
            view.title = model.listTitle
            view.count = model.titleCount
            view.icon = model.titleIcon
            view.onClick = { [weak self, weak view] in
                guard let self, let view else { return }
                self.titleClicked(view)
            }
            item.view = view
            item.isBordered = false
        case .signIn:
            let label = String(localized: "Sign In")
            item.title = label
            item.label = label
            item.isBordered = true
            item.target = self
            item.action = #selector(signIn(_:))
        case .newList:
            let label = String(localized: "New List")
            item.image = NSImage(systemSymbolName: "folder.badge.plus", accessibilityDescription: label)
            item.label = label
            item.toolTip = label
            item.isBordered = true
            item.target = self
            item.action = #selector(newList(_:))
        case .search:
            // Search's overlay, as ⌘F (Edit › Find…).
            let label = String(localized: "Search")
            item.image = NSImage(systemSymbolName: "magnifyingglass", accessibilityDescription: label)
            item.label = label
            item.toolTip = label
            item.isBordered = true
            item.target = self
            item.action = #selector(openSearch(_:))
        case .newItem:
            // Not wired yet: the action does nothing (without one the
            // toolbar would draw the button disabled).
            let label = String(localized: "New Item")
            item.image = NSImage(systemSymbolName: "square.and.pencil", accessibilityDescription: label)
            item.label = label
            item.toolTip = label
            item.isBordered = true
            item.target = self
            item.action = #selector(newItem(_:))
        case .itemStatus:
            // The checkbox and the workflow state as one control with
            // two halves, as the iOS item page's: the checkbox ticks
            // and reopens, the label opens a menu of the states.
            let control = NSSegmentedControl()
            control.trackingMode = .momentary
            control.target = self
            control.action = #selector(statusClicked(_:))
            item.view = control
        case .itemStar:
            item.isBordered = true
            item.target = self
            item.action = #selector(toggleStar(_:))
        default:
            return nil
        }
        update(item)
        return item
    }

    @objc private func signIn(_ sender: Any?) {
        onSignIn()
    }

    /// The toolbar's magnifying glass and Edit › Find…: Spotlight's bar
    /// over the window (`SearchPanel.swift`). Again shuts it.
    @objc func openSearch(_ sender: Any?) {
        guard let window = view.window else { return }
        searchPanel.toggle(over: window)
    }

    /// New List: a popover under the button with the name and, with
    /// more than one workspace open, which one (Personal unless
    /// changed), as on iOS. Create (or Return) makes the list and shows
    /// it; a refusal is said in an alert on the window.
    @objc private func newList(_ sender: Any?) {
        if let newListPopover, newListPopover.isShown {
            newListPopover.performClose(nil)
            return
        }
        guard let item = toolbar.items.first(where: { $0.itemIdentifier == .newList }) else { return }
        let workspaces = model.sections.map {
            NewListForm.Workspace(
                target: $0.target,
                name: $0.workspace.home
                    ? String(localized: "Personal") : $0.workspace.name ?? $0.workspace.docId)
        }
        guard !workspaces.isEmpty else { return }
        let popover = NSPopover()
        popover.behavior = .transient
        let form = NewListForm(workspaces: workspaces) { [weak self, weak popover] name, workspace in
            popover?.performClose(nil)
            guard let self else { return }
            self.model.addList(workspace: workspace, name: name) { [weak self] error in
                guard let window = self?.view.window else { return }
                let alert = NSAlert()
                alert.alertStyle = .warning
                alert.messageText = String(localized: "Could not create the list")
                alert.informativeText = error
                alert.beginSheetModal(for: window)
            }
        }
        let host = NSHostingController(rootView: form)
        host.sizingOptions = .preferredContentSize
        popover.contentViewController = host
        newListPopover = popover
        popover.show(relativeTo: item)
    }

    /// The title on a user list: a popover under it with the list's
    /// name in a field and nothing else. The name is written when the
    /// popover shuts (Return, a click outside, the title again) unless
    /// it is blank or unchanged; Escape shuts it without writing.
    private func titleClicked(_ sender: NSView) {
        if let renamePopover, renamePopover.isShown {
            renamePopover.performClose(nil)
            return
        }
        guard let target = model.renamableList else { return }
        let popover = NSPopover()
        popover.behavior = .transient
        let draft = RenameDraft(name: target.name)
        draft.close = { [weak popover] in popover?.performClose(nil) }
        draft.commit = { [model] name in
            model.renameList(workspace: target.workspace, list: target.list, name: name)
        }
        popover.delegate = draft
        let host = NSHostingController(rootView: RenameListForm(draft: draft))
        host.sizingOptions = .preferredContentSize
        popover.contentViewController = host
        renamePopover = popover
        popover.show(relativeTo: sender.bounds, of: sender, preferredEdge: sender.isFlipped ? .maxY : .minY)
    }

    @objc private func newItem(_ sender: Any?) {}

    /// The status control's halves: the checkbox, then the label.
    @objc private func statusClicked(_ sender: NSSegmentedControl) {
        guard let (selected, item) = model.current, item.item.binnedAt == nil else { return }
        if sender.selectedSegment == 0 {
            model.setDone(workspace: selected.workspace, id: selected.id, done: !item.item.state.isClosed)
        } else {
            popStateMenu(item, under: sender)
        }
    }

    /// The states as a menu under the status label, the current one
    /// ticked; a pick moves the item there in one write. On an Open
    /// event the single open choice is ticked whatever open state the
    /// item holds underneath, and picking it changes nothing, so a
    /// state the item brought in from a list is kept.
    private func popStateMenu(_ item: ItemInfo, under control: NSSegmentedControl) {
        let event = item.item.event
        let picked: ItemState = event && !item.item.state.isClosed ? .backlog : item.item.state
        let menu = NSMenu()
        for state in event ? ItemPanel.eventStates : ItemPanel.taskStates {
            let row = NSMenuItem(
                title: ItemPanel.statusLabel(state, event: event), action: #selector(pickState(_:)),
                keyEquivalent: "")
            row.target = self
            row.representedObject = state
            row.state = state == picked ? .on : .off
            menu.addItem(row)
        }
        // Under the label's half, its left edge on the half's.
        let x = control.width(forSegment: 0) > 0 ? control.width(forSegment: 0) : control.bounds.width / 3
        let y = control.isFlipped ? control.bounds.maxY + 6 : control.bounds.minY - 6
        menu.popUp(positioning: nil, at: NSPoint(x: x, y: y), in: control)
    }

    @objc private func pickState(_ sender: NSMenuItem) {
        guard let state = sender.representedObject as? ItemState, sender.state != .on,
            let selected = model.current?.selection
        else { return }
        model.setState(workspace: selected.workspace, id: selected.id, state: state)
    }

    /// The "…" menu: the rows' right-click menu (`itemMenu` in
    /// `MainWindow.swift`), for the panel's item; a binned item's is
    /// the Bin's rows' (`binnedMenu`).
    private func itemMenu(binned: Bool) -> NSMenu {
        let menu = NSMenu()
        menu.autoenablesItems = false
        if binned {
            let restore = NSMenuItem(
                title: String(localized: "Restore"), action: #selector(restoreItem(_:)), keyEquivalent: "")
            let delete = NSMenuItem(
                title: String(localized: "Delete…"), action: #selector(deleteItem(_:)), keyEquivalent: "")
            for row in [restore, delete] { row.target = self }
            menu.items = [restore, .separator(), delete]
            return menu
        }
        let duplicate = NSMenuItem(
            title: String(localized: "Duplicate"), action: #selector(duplicateItem(_:)), keyEquivalent: "")
        let bin = NSMenuItem(
            title: String(localized: "Move to Bin"), action: #selector(binItem(_:)), keyEquivalent: "")
        for row in [duplicate, bin] { row.target = self }
        menu.items = [duplicate, .separator(), bin]
        return menu
    }

    @objc private func duplicateItem(_ sender: Any?) {
        guard let selected = model.current?.selection else { return }
        model.duplicate(workspace: selected.workspace, id: selected.id)
    }

    @objc private func binItem(_ sender: Any?) {
        guard let selected = model.current?.selection else { return }
        model.bin(workspace: selected.workspace, id: selected.id)
    }

    @objc private func restoreItem(_ sender: Any?) {
        guard let selected = model.current?.selection else { return }
        model.restore(workspace: selected.workspace, id: selected.id)
    }

    @objc private func deleteItem(_ sender: Any?) {
        guard let selected = model.current?.selection else { return }
        BinConfirm.delete(on: view.window) { [model] in
            model.deleteBinned(workspace: selected.workspace, id: selected.id)
        }
    }

    @objc private func toggleStar(_ sender: Any?) {
        guard let (selected, item) = model.current else { return }
        model.setStarred(workspace: selected.workspace, id: selected.id, starred: !item.starred)
    }

    /// Escape, when nothing nearer wants it, shuts the panel.
    override func cancelOperation(_ sender: Any?) {
        if model.selected != nil { model.closeItem() }
    }

    // MARK: Split view

    /// The panel's edge dragged until it folds away is the panel
    /// closed: the selection goes with it.
    override func splitViewDidResizeSubviews(_ notification: Notification) {
        super.splitViewDidResizeSubviews(notification)
        if !syncing, panelItem.isCollapsed, model.selected != nil { model.closeItem() }
    }
}

/// The inspector pane: the selected item's page, empty without one.
struct ItemPanelPane: View {
    @ObservedObject var model: MainWindowModel

    var body: some View {
        Group {
            // The page shown, which on a new selection is the old one
            // until the new one is read: its fields, and its writes,
            // are for the item it shows, in that item's workspace.
            if model.selected != nil, let shown = model.detailOf {
                ItemPanel(
                    item: model.detail, workspaceName: model.sharedWorkspaceName(shown.workspace),
                    lists: model.moveLists(workspace: shown.workspace),
                    otherLists: model.otherMoveLists(workspace: shown.workspace),
                    retitle: { model.setText(workspace: shown.workspace, item: $0, text: $1) },
                    move: { model.moveItem(workspace: shown.workspace, id: $0.item.id, list: $1) },
                    moveAcross: {
                        model.moveItemAcross(
                            from: shown.workspace, id: $0.item.id, to: $1, list: $2, on: NSApp.keyWindow)
                    },
                    knownPlaces: { model.knownPlaces(matching: $0, completion: $1) },
                    setPlace: { model.setPlace(workspace: shown.workspace, id: $0.item.id, place: $1, exact: $2) })
            }
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .background(Color(nsColor: PaneColor.panel).ignoresSafeArea())
    }
}

/// The window's split view, for its dividers' colour: the panes'
/// borders are near-black in dark, the system's in light.
private final class PaneSplitView: NSSplitView {
    override var dividerColor: NSColor { PaneColor.divider(super.dividerColor) }
}

/// The new-list popover: the name, focused as it opens, the workspace
/// it goes in (only while more than one is open; the first, Personal,
/// unless changed), and Create. Return creates too. `create` gets the
/// trimmed name and the workspace's `target`.
private struct NewListForm: View {
    struct Workspace: Hashable {
        let target: String?
        let name: String
    }

    let workspaces: [Workspace]
    let create: (String, String?) -> Void

    @State private var name = ""
    @State private var picked = 0
    @FocusState private var focused: Bool

    private var trimmed: String { name.trimmingCharacters(in: .whitespacesAndNewlines) }

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            TextField("New List", text: $name)
                .textFieldStyle(.roundedBorder)
                .focused($focused)
                .onSubmit(submit)
            if workspaces.count > 1 {
                Picker("Workspace", selection: $picked) {
                    ForEach(workspaces.indices, id: \.self) { index in
                        Text(verbatim: workspaces[index].name).tag(index)
                    }
                }
            }
            HStack {
                Spacer()
                Button("Create", action: submit)
                    .keyboardShortcut(.defaultAction)
                    .disabled(trimmed.isEmpty)
            }
        }
        .padding(12)
        .frame(width: 280)
        .onAppear { focused = true }
    }

    private func submit() {
        guard !trimmed.isEmpty, workspaces.indices.contains(picked) else { return }
        create(trimmed, workspaces[picked].target)
    }
}

/// The rename popover's state, and its delegate: whatever is in the
/// field when it shuts is written, unless Escape discarded it. The
/// model drops a blank or unchanged name.
private final class RenameDraft: NSObject, ObservableObject, NSPopoverDelegate {
    @Published var name: String
    var discarded = false
    var close: () -> Void = {}
    var commit: (String) -> Void = { _ in }

    init(name: String) { self.name = name }

    func popoverDidClose(_ notification: Notification) {
        if !discarded { commit(name) }
    }
}

/// The title's rename popover: the list's name in a field, labelled.
private struct RenameListForm: View {
    @ObservedObject var draft: RenameDraft
    @FocusState private var focused: Bool

    var body: some View {
        HStack(spacing: 8) {
            Text("Name")
            TextField("Name", text: $draft.name)
                .labelsHidden()
                .textFieldStyle(.roundedBorder)
                .frame(width: 240)
                .focused($focused)
                .onSubmit { draft.close() }
                .onExitCommand {
                    draft.discarded = true
                    draft.close()
                }
        }
        .padding(10)
        .onAppear { focused = true }
    }
}

/// The bar's title: the list's name, inset from the sidebar's edge,
/// in the label colour, with a down caret beside it while the pointer
/// is over a renamable one: the caret a document window's title shows
/// (an `NSDocument` gets that for free; a list is not a document, so
/// this draws its own). The caret's room is always kept, so the title
/// does not shift when it appears.
final class ToolbarTitleView: NSView {
    var title = "" {
        didSet { if label.stringValue != title { label.stringValue = title } }
    }
    /// The sidebar row's glyph, before the name, if it has one.
    var icon: NSImage? {
        didSet {
            guard icon !== oldValue else { return }
            iconView.image = icon
            iconView.isHidden = icon == nil
            labelAfterIcon.isActive = icon != nil
            labelAtInset.isActive = icon == nil
        }
    }
    /// Shown muted after the name; none for zero.
    var count = 0 {
        didSet {
            let text = count > 0 ? String(count) : ""
            if countLabel.stringValue != text { countLabel.stringValue = text }
            countLabel.isHidden = count == 0
        }
    }
    /// Whether a click renames: the caret, tooltip and click follow it.
    var renamable = false {
        didSet {
            toolTip = renamable ? String(localized: "Rename List") : nil
            setAccessibilityRole(renamable ? .button : .staticText)
            syncCaret()
        }
    }
    var onClick: () -> Void = {}

    private let label = NSTextField(labelWithString: "")
    private let countLabel = NSTextField(labelWithString: "")
    private let iconView = NSImageView()
    private lazy var labelAtInset = label.leadingAnchor.constraint(equalTo: leadingAnchor, constant: Self.inset)
    private lazy var labelAfterIcon = label.leadingAnchor.constraint(equalTo: iconView.trailingAnchor, constant: 6)
    private let caret = NSImageView()
    private var hovering = false {
        didSet { syncCaret() }
    }
    /// From the sidebar's separator to the title.
    static let inset: CGFloat = 10

    init() {
        super.init(frame: .zero)
        label.font = .systemFont(ofSize: NSFont.systemFontSize + 2, weight: .bold)
        label.textColor = .labelColor
        label.lineBreakMode = .byTruncatingTail
        label.setContentCompressionResistancePriority(.defaultLow, for: .horizontal)
        iconView.contentTintColor = .labelColor
        iconView.imageScaling = .scaleProportionallyUpOrDown
        iconView.isHidden = true
        countLabel.font = .systemFont(ofSize: NSFont.systemFontSize + 2, weight: .regular)
        countLabel.textColor = .secondaryLabelColor
        countLabel.setContentCompressionResistancePriority(.required, for: .horizontal)
        countLabel.isHidden = true
        caret.image = NSImage(systemSymbolName: "chevron.down", accessibilityDescription: nil)?
            .withSymbolConfiguration(.init(pointSize: 10, weight: .semibold))
        caret.contentTintColor = .secondaryLabelColor
        caret.isHidden = true
        for view in [iconView, label, countLabel, caret] {
            view.translatesAutoresizingMaskIntoConstraints = false
            addSubview(view)
        }
        NSLayoutConstraint.activate([
            labelAtInset,
            iconView.leadingAnchor.constraint(equalTo: leadingAnchor, constant: Self.inset),
            iconView.centerYAnchor.constraint(equalTo: label.centerYAnchor),
            iconView.widthAnchor.constraint(equalToConstant: 16),
            iconView.heightAnchor.constraint(equalToConstant: 16),
            label.centerYAnchor.constraint(equalTo: centerYAnchor),
            countLabel.leadingAnchor.constraint(equalTo: label.trailingAnchor, constant: 6),
            countLabel.firstBaselineAnchor.constraint(equalTo: label.firstBaselineAnchor),
            caret.leadingAnchor.constraint(equalTo: countLabel.trailingAnchor, constant: 4),
            caret.centerYAnchor.constraint(equalTo: label.centerYAnchor, constant: 1),
            caret.trailingAnchor.constraint(equalTo: trailingAnchor),
            heightAnchor.constraint(greaterThanOrEqualTo: label.heightAnchor),
        ])
        setAccessibilityElement(true)
        setAccessibilityRole(.staticText)
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }

    override func accessibilityLabel() -> String? { count > 0 ? "\(title), \(count)" : title }

    override func accessibilityPerformPress() -> Bool {
        guard renamable else { return false }
        onClick()
        return true
    }

    private func syncCaret() {
        caret.isHidden = !(renamable && hovering)
    }

    override func updateTrackingAreas() {
        super.updateTrackingAreas()
        for area in trackingAreas { removeTrackingArea(area) }
        addTrackingArea(NSTrackingArea(
            rect: .zero, options: [.mouseEnteredAndExited, .activeAlways, .inVisibleRect],
            owner: self))
    }

    override func mouseEntered(with event: NSEvent) { hovering = true }
    override func mouseExited(with event: NSEvent) { hovering = false }

    /// A click on an inactive window renames at once, as a toolbar
    /// button would.
    override func acceptsFirstMouse(for event: NSEvent?) -> Bool { true }

    override func mouseDown(with event: NSEvent) {
        guard renamable else { return super.mouseDown(with: event) }
        onClick()
    }
}
