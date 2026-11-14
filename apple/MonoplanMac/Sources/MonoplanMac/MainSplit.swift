// The window's chrome, in AppKit as Xcode's or Sketch's is: a split
// view controller of three panes (the glass sidebar, the list, the
// item's panel), each one a SwiftUI view, under one toolbar that the
// split view divides. The toolbar has a separator tracking each
// divider, so what comes after the panel's separator sits over the
// panel wherever its edge is dragged: the status control and the
// star, the iOS item page's bar (`ItemView.bar`). The
// system draws each on its glass. The window's title, which the bar
// shows over the list, is the name of what is listed
// (`MainWindowController`).
//
// SwiftUI's own `NavigationSplitView` and `.inspector` were tried
// here first: in a window made by hand their toolbar items do not
// land over the panel.
//
// The panel has no state menu and no more menu yet: it has no writes
// for them.

import AppKit
import MonoplanCore
import SwiftUI

private extension NSToolbarItem.Identifier {
    static let itemStatus = NSToolbarItem.Identifier("MonoplanItemStatus")
    static let itemStar = NSToolbarItem.Identifier("MonoplanItemStar")
    /// The separator tracking the panel's edge (the system's own
    /// inspector separator wants an inspector item).
    static let panelSeparator = NSToolbarItem.Identifier("MonoplanPanelSeparator")
    /// The list's section, at its trailing edge: search, then new.
    static let search = NSToolbarItem.Identifier("MonoplanSearch")
    static let newItem = NSToolbarItem.Identifier("MonoplanNewItem")
    /// Over the sidebar, at its trailing edge, while there is no
    /// account.
    static let signIn = NSToolbarItem.Identifier("MonoplanSignIn")
    /// In Sign In's place once there is one: the account's menu, as
    /// the avatar on the iOS Home page.
    static let account = NSToolbarItem.Identifier("MonoplanAccount")
}

final class MainSplitViewController: NSSplitViewController, NSToolbarDelegate, NSMenuDelegate {
    private let model: MainWindowModel
    private let sidebarItem: NSSplitViewItem
    private let listItem: NSSplitViewItem
    private let panelItem: NSSplitViewItem
    let toolbar = NSToolbar(identifier: "MonoplanMain")
    /// Set while `sync` moves the panel itself, so the resize it
    /// causes is not read as the user folding the panel away.
    private var syncing = false
    /// Whether the profile has an account: the bar leads with the
    /// account's menu with one, with Sign In without. The window's
    /// controller keeps it and calls `sync`.
    var signedIn = true
    /// The account's email, once the engine has said; the menu's
    /// heading.
    var email: String?
    /// Sign In was clicked.
    var onSignIn: () -> Void = {}
    /// The account menu's New Workspace… was picked.
    var onNewWorkspace: () -> Void = {}

    static let sidebarWidth = (min: 160.0, initial: 200.0, max: 400.0)
    static let panelWidth = (min: 240.0, max: 520.0)

    /// The toolbar with the panel shut: the title's section after the
    /// sidebar's separator, with search and new at its end. The
    /// panel's separator is there only with the panel (`panelItems`):
    /// with the panel shut it would draw at the window's edge.
    /// The space between those two keeps each on its own glass:
    /// adjacent buttons would share one.
    private static let fixedItems: [NSToolbarItem.Identifier] = [
        .sidebarTrackingSeparator, .flexibleSpace, .search, .space, .newItem,
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
        // The account's menu, or Sign In without an account, leads
        // the bar, before the sidebar's separator, so it sits over the
        // sidebar; the space before it pushes it to the sidebar's
        // trailing edge.
        let lead: [NSToolbarItem.Identifier] = [.flexibleSpace, signedIn ? .account : .signIn]
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
    /// at the far edge. No close button: Escape, or its edge dragged
    /// shut. Only an Open item can be starred (spec/focus.md).
    private func panelItems() -> [NSToolbarItem.Identifier] {
        guard model.selected != nil else { return [] }
        var items: [NSToolbarItem.Identifier] = [.panelSeparator]
        guard let item = model.detail else { return items }
        items.append(.itemStatus)
        if item.item.binnedAt == nil, item.closedAt == nil {
            items += [.flexibleSpace, .itemStar]
        }
        return items
    }

    private func update(_ toolbarItem: NSToolbarItem) {
        guard let item = model.detail else { return }
        switch toolbarItem.itemIdentifier {
        case .itemStatus:
            guard let button = toolbarItem.view as? NSButton else { return }
            // A binned item just says so: the bin masks its state.
            if item.item.binnedAt != nil {
                button.title = String(localized: "Binned")
                button.image = nil
                button.isEnabled = false
                button.toolTip = nil
            } else {
                let closed = item.item.state.isClosed
                let hint = closed ? String(localized: "Mark as not done") : String(localized: "Mark as done")
                button.title = ItemPanel.statusLabel(item)
                button.image = ItemCheckArt.image(item.item.state, accessibilityDescription: hint)
                button.isEnabled = true
                button.toolTip = hint
            }
            button.sizeToFit()
        case .itemStar:
            let hint = item.starred ? String(localized: "Remove star") : String(localized: "Star")
            toolbarItem.image = StarArt.toolbarImage(starred: item.starred)
            toolbarItem.label = hint
            toolbarItem.toolTip = hint
        default:
            break
        }
    }

    // MARK: Toolbar

    func toolbarDefaultItemIdentifiers(_ toolbar: NSToolbar) -> [NSToolbarItem.Identifier] {
        Self.fixedItems
    }

    func toolbarAllowedItemIdentifiers(_ toolbar: NSToolbar) -> [NSToolbarItem.Identifier] {
        Self.fixedItems + [.panelSeparator, .signIn, .account, .itemStatus, .itemStar]
    }

    func toolbar(
        _ toolbar: NSToolbar, itemForItemIdentifier identifier: NSToolbarItem.Identifier,
        willBeInsertedIntoToolbar flag: Bool
    ) -> NSToolbarItem? {
        if identifier == .panelSeparator {
            return NSTrackingSeparatorToolbarItem(identifier: identifier, splitView: splitView, dividerIndex: 1)
        }
        if identifier == .account {
            // The menu is filled as it opens (`menuNeedsUpdate`).
            let item = NSMenuToolbarItem(itemIdentifier: identifier)
            let label = String(localized: "Account")
            item.image = NSImage(systemSymbolName: "person.crop.circle", accessibilityDescription: label)
            item.label = label
            item.toolTip = label
            item.showsIndicator = false
            item.menu.delegate = self
            item.menu.autoenablesItems = false
            return item
        }
        let item = NSToolbarItem(itemIdentifier: identifier)
        switch identifier {
        case .signIn:
            let label = String(localized: "Sign In")
            item.title = label
            item.label = label
            item.isBordered = true
            item.target = self
            item.action = #selector(signIn(_:))
        case .search:
            // Not wired yet: a later surface is search's. The action
            // does nothing (without one the button draws disabled).
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
            // The checkbox and the workflow state as one control: a
            // click ticks and reopens.
            let button = NSButton(title: "", target: self, action: #selector(toggleDone(_:)))
            button.bezelStyle = .toolbar
            button.imagePosition = .imageLeading
            item.view = button
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

    /// The account's menu, as on iOS: the email as its heading, then
    /// New Workspace…, which needs a running engine.
    func menuNeedsUpdate(_ menu: NSMenu) {
        menu.removeAllItems()
        if let email { menu.addItem(.sectionHeader(title: email)) }
        let new = NSMenuItem(
            title: String(localized: "New Workspace…"), action: #selector(newWorkspace(_:)), keyEquivalent: "")
        new.target = self
        if case .connected = model.state { new.isEnabled = true } else { new.isEnabled = false }
        menu.addItem(new)
    }

    @objc private func newWorkspace(_ sender: Any?) {
        onNewWorkspace()
    }

    @objc private func openSearch(_ sender: Any?) {}

    @objc private func newItem(_ sender: Any?) {}

    @objc private func toggleDone(_ sender: Any?) {
        guard let selected = model.selected, let item = model.detail, item.item.binnedAt == nil else { return }
        model.setDone(workspace: selected.workspace, id: selected.id, done: !item.item.state.isClosed)
    }

    @objc private func toggleStar(_ sender: Any?) {
        guard let selected = model.selected, let item = model.detail else { return }
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
            if let selected = model.selected {
                ItemPanel(item: model.detail, workspaceName: model.sharedWorkspaceName(selected.workspace)) {
                    model.setText(workspace: selected.workspace, item: $0, text: $1)
                }
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
