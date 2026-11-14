// The window's chrome, in AppKit as Xcode's or Sketch's is: a split
// view controller of three panes (the glass sidebar, the list, the
// item's panel), each one a SwiftUI view, under one toolbar that the
// split view divides. The toolbar has a separator tracking each
// divider, so what comes after the panel's separator sits over the
// panel wherever its edge is dragged: the close button, the status
// control and the star, the iOS item page's bar (`ItemView.bar`). The
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
    static let closePanel = NSToolbarItem.Identifier("MonoplanClosePanel")
    static let itemStatus = NSToolbarItem.Identifier("MonoplanItemStatus")
    static let itemStar = NSToolbarItem.Identifier("MonoplanItemStar")
    /// The separator tracking the panel's edge (the system's own
    /// inspector separator wants an inspector item).
    static let panelSeparator = NSToolbarItem.Identifier("MonoplanPanelSeparator")
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

    static let sidebarWidth = (min: 160.0, initial: 200.0, max: 400.0)
    static let panelWidth = (min: 240.0, max: 520.0)

    /// The toolbar with the panel shut: the title's section, between
    /// the two tracking separators.
    private static let fixedItems: [NSToolbarItem.Identifier] = [
        .sidebarTrackingSeparator, .flexibleSpace, .panelSeparator,
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
        let wanted = panelItems()
        let have = toolbar.items.dropFirst(Self.fixedItems.count).map(\.itemIdentifier)
        if have != wanted {
            while toolbar.items.count > Self.fixedItems.count {
                toolbar.removeItem(at: toolbar.items.count - 1)
            }
            for identifier in wanted {
                toolbar.insertItem(withItemIdentifier: identifier, at: toolbar.items.count)
            }
        }
        for item in toolbar.items { update(item) }
    }

    /// What follows the panel's separator: nothing with the panel
    /// shut; else the close button, the status once the item is read,
    /// and the star at the far edge. Only an Open item can be starred
    /// (spec/focus.md).
    private func panelItems() -> [NSToolbarItem.Identifier] {
        guard model.selected != nil else { return [] }
        var items: [NSToolbarItem.Identifier] = [.closePanel]
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
        Self.fixedItems + [.closePanel, .itemStatus, .itemStar]
    }

    func toolbar(
        _ toolbar: NSToolbar, itemForItemIdentifier identifier: NSToolbarItem.Identifier,
        willBeInsertedIntoToolbar flag: Bool
    ) -> NSToolbarItem? {
        if identifier == .panelSeparator {
            return NSTrackingSeparatorToolbarItem(identifier: identifier, splitView: splitView, dividerIndex: 1)
        }
        let item = NSToolbarItem(itemIdentifier: identifier)
        switch identifier {
        case .closePanel:
            let label = String(localized: "Close")
            item.image = NSImage(systemSymbolName: "xmark", accessibilityDescription: label)
            item.label = label
            item.toolTip = label
            item.isBordered = true
            item.target = self
            item.action = #selector(closePanel(_:))
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

    @objc private func closePanel(_ sender: Any?) {
        model.closeItem()
    }

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
                ItemPanel(item: model.detail, workspaceName: model.sharedWorkspaceName(selected.workspace))
            }
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
    }
}
