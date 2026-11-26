// The window's chrome, in AppKit as Xcode's or Sketch's is: a split
// view controller of three panes (the opaque sidebar, the list, the
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
    /// The separator tracking the sidebar's edge (the system's own
    /// wants a sidebar item).
    static let sidebarSeparator = NSToolbarItem.Identifier("MonoplanSidebarSeparator")
    /// Over the sidebar, before New List, once there is an account;
    /// the list's section has new alone at its trailing edge.
    static let search = NSToolbarItem.Identifier("MonoplanSearch")
    static let newItem = NSToolbarItem.Identifier("MonoplanNewItem")
    /// Before search while Done is shown: its filter's popover
    /// (`DoneFilter.swift`).
    static let doneFilter = NSToolbarItem.Identifier("MonoplanDoneFilter")
    /// Before "Today" on the Calendar: its filter's popover
    /// (`CalendarFilter.swift`).
    static let agendaFilter = NSToolbarItem.Identifier("MonoplanAgendaFilter")
    /// Before new on the Inbox, while more than one workspace is open:
    /// its filter's popover (`InboxFilter.swift`).
    static let inboxFilter = NSToolbarItem.Identifier("MonoplanInboxFilter")
    /// The Calendar's "Today": scrolls back to Today's header.
    static let today = NSToolbarItem.Identifier("MonoplanToday")
    /// At the far end of the list's section, after new, while there
    /// is no account. Nothing takes its place once there is one: the
    /// account lives in Settings and the menus.
    static let signIn = NSToolbarItem.Identifier("MonoplanSignIn")
    /// Over the sidebar, at its trailing edge, after search: the iOS
    /// Home's folder button, a popover that names a new list.
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
    /// The rename popover, while it is up.
    private var renamePopover: NSPopover?
    /// The new-list popover, while it is up.
    private var newListPopover: NSPopover?
    /// The Done filter's popover, while it is up.
    private var doneFilterPopover: NSPopover?
    /// The Calendar filter's popover, while it is up.
    private var calendarFilterPopover: NSPopover?
    /// The Inbox filter's popover, while it is up.
    private var inboxFilterPopover: NSPopover?
    /// Search's overlay (`SearchPanel.swift`).
    private lazy var searchPanel = SearchPanelController(model: model)

    static let sidebarWidth = (min: 160.0, initial: 200.0, max: 400.0)
    static let panelWidth = (min: 240.0, max: 520.0)

    /// The toolbar before the first `sync`: the title's section after
    /// the sidebar's separator, with new at its end. `sync` adds the
    /// sidebar's items before it and the panel's (`panelItems`) after.
    private static let fixedItems = listItems(lens: nil)

    /// The list's section of the bar. On Done its filter leads the
    /// button at the end, on a glass of its own (the space between
    /// them: adjacent buttons would share one); on the Calendar
    /// "Today" does; on the Inbox its filter, while there are
    /// workspaces to pick from (`workspaces`). Without an account Sign In ends the section,
    /// likewise on its own glass.
    private static func listItems(
        lens: Lens?, workspaces: Int = 1, signedIn: Bool = true
    ) -> [NSToolbarItem.Identifier] {
        let own: [NSToolbarItem.Identifier]
        switch lens {
        case .done: own = [.doneFilter, .space]
        case .agenda: own = [.agendaFilter, .space, .today, .space]
        case .inbox where workspaces > 1: own = [.inboxFilter, .space]
        default: own = []
        }
        return [.sidebarSeparator, .title, .flexibleSpace] + own + [.newItem] + (signedIn ? [] : [.space, .signIn])
    }

    init(model: MainWindowModel) {
        self.model = model
        // A sidebar item: the system's Liquid Glass sidebar, the
        // SwiftUI pane clear over it (trial, 2026-11-23; it was a plain
        // pane, opaque in `PaneColor.sidebar`).
        sidebarItem = NSSplitViewItem(
            sidebarWithViewController: Self.pane(MainWindowView(model: model, pane: .sidebar)))
        // A sidebar item's own priority: the window's resizing goes
        // to the list.
        sidebarItem.holdingPriority = NSLayoutConstraint.Priority(260)
        listItem = NSSplitViewItem(
            viewController: Self.pane(MainWindowView(model: model, pane: .list)))
        // A plain pane, not an inspector item: an inspector is glass,
        // and the bar's background shows through it.
        panelItem = NSSplitViewItem(
            viewController: Self.pane(ItemPanelPane(model: model)))
        super.init(nibName: nil, bundle: nil)
        // A press that takes the keyboard from the new-item row: in
        // the panel, it is after the item being typed (`ItemCapture`).
        model.isInPanel = { [weak self] point in
            guard let self else { return false }
            let view = self.panelItem.viewController.view
            return view.window != nil && view.bounds.contains(view.convert(point, from: nil))
        }
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
        // Always there (2026-11-20): the selected item's page, a new
        // item's while one is typed, and empty otherwise.
        panelItem.canCollapse = false
        // As a sidebar's: the window's resizing goes to the list.
        panelItem.holdingPriority = sidebarItem.holdingPriority
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

    /// The toolbar's sections, with the item controls for the item
    /// the panel shows. Called on every change of the model.
    func sync() {
        // Over the sidebar, before its separator: search then New
        // List, account or none (side by side, on one glass: the
        // sidebar is too narrow at its least for a space between);
        // the space before pushes them to the sidebar's trailing
        // edge. Sign In is at the end of the list's section.
        let lead: [NSToolbarItem.Identifier] = [.flexibleSpace, .search, .newList]
        let leading = toolbar.items.firstIndex { $0.itemIdentifier == .sidebarSeparator } ?? 0
        if toolbar.items.prefix(leading).map(\.itemIdentifier) != lead {
            for _ in 0..<leading { toolbar.removeItem(at: 0) }
            for (index, identifier) in lead.enumerated() {
                toolbar.insertItem(withItemIdentifier: identifier, at: index)
            }
        }
        // The rest: the list's section, then the panel's. Whatever
        // already matches from the front stays; the tail is rebuilt.
        let wanted = lead
            + Self.listItems(lens: model.shown.lens, workspaces: model.sections.count, signedIn: model.signedIn)
            + Self.panelItems
        let have = toolbar.items.map(\.itemIdentifier)
        if have != wanted {
            let kept = zip(have, wanted).prefix { $0 == $1 }.count
            while toolbar.items.count > kept {
                toolbar.removeItem(at: toolbar.items.count - 1)
            }
            for identifier in wanted.dropFirst(kept) {
                toolbar.insertItem(withItemIdentifier: identifier, at: toolbar.items.count)
            }
        }
        for item in toolbar.items { update(item) }
    }

    /// The panel's part of the bar: its separator, the status, and
    /// the star and the "…" menu at the far edge. Always all of them:
    /// taking items out and putting them back as the selection comes
    /// and goes redraws the bar over the panel, so what does not apply
    /// is disabled instead (`update`).
    private static let panelItems: [NSToolbarItem.Identifier] =
        [.panelSeparator, .itemStatus, .flexibleSpace, .itemStar, .itemMore]

    /// The item the panel's controls act on; nil with the panel empty
    /// and during a capture, which is not an item yet: nothing to
    /// tick, star or bin.
    private var barItem: ItemInfo? { model.capture == nil ? model.detail : nil }

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
        if toolbarItem.itemIdentifier == .doneFilter {
            // The count beside the glyph while any check is in effect, as
            // the web's trigger; the hint says it too.
            let count = model.doneFilter.count
            let label = count > 0 ? String(localized: "Filter (\(count) active)") : String(localized: "Filter")
            toolbarItem.image = FilterArt.image(count: count, label: label)
            toolbarItem.label = label
            toolbarItem.toolTip = label
            return
        }
        if toolbarItem.itemIdentifier == .inboxFilter {
            // How many workspaces are picked; 0 is All Workspaces.
            let count = model.inboxPicked.count
            let label = count > 0 ? String(localized: "Filter (\(count) active)") : String(localized: "Filter")
            toolbarItem.image = FilterArt.image(count: count, label: label)
            toolbarItem.label = label
            toolbarItem.toolTip = label
            return
        }
        if toolbarItem.itemIdentifier == .agendaFilter {
            // The count of checked workspaces beside the glyph, as Done's.
            // "Hide Empty Days" is a view option, not counted (the web's).
            let count = model.calendarCheckedDocs.count
            let label = count > 0 ? String(localized: "Filter (\(count) active)") : String(localized: "Filter")
            toolbarItem.image = FilterArt.image(count: count, label: label)
            toolbarItem.label = label
            toolbarItem.toolTip = label
            return
        }
        let item = barItem
        switch toolbarItem.itemIdentifier {
        case .itemStatus:
            guard let control = toolbarItem.view as? NSSegmentedControl else { return }
            guard let item else {
                // No item: an empty box and the control's name, greyed.
                control.segmentCount = 2
                control.setLabel("", forSegment: 0)
                control.setImage(ItemCheckArt.image(.backlog, accessibilityDescription: nil), forSegment: 0)
                control.setToolTip(nil, forSegment: 0)
                control.setLabel(String(localized: "Status"), forSegment: 1)
                control.setToolTip(nil, forSegment: 1)
                control.isEnabled = false
                for segment in 0..<control.segmentCount { control.setWidth(0, forSegment: segment) }
                control.sizeToFit()
                return
            }
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
            let starred = item?.starred ?? false
            let hint = starred ? String(localized: "Remove star") : String(localized: "Star")
            toolbarItem.image = StarArt.toolbarImage(starred: starred)
            toolbarItem.label = hint
            toolbarItem.toolTip = hint
            // Only an Open, unbinned item can be starred (spec/focus.md).
            toolbarItem.isEnabled = item.map { $0.item.binnedAt == nil && $0.closedAt == nil } ?? false
        case .itemMore:
            toolbarItem.isEnabled = item != nil
            guard let item else { return }
            // The menu follows the item in and out of the Bin: a
            // binned item's restores or deletes it.
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
        Self.fixedItems + [.doneFilter, .agendaFilter, .inboxFilter, .today, .panelSeparator, .signIn, .search, .newList, .itemStatus, .itemStar, .itemMore]
    }

    func toolbar(
        _ toolbar: NSToolbar, itemForItemIdentifier identifier: NSToolbarItem.Identifier,
        willBeInsertedIntoToolbar flag: Bool
    ) -> NSToolbarItem? {
        if identifier == .sidebarSeparator {
            return NSTrackingSeparatorToolbarItem(identifier: identifier, splitView: splitView, dividerIndex: 0)
        }
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
            // Enabled by `update`, not by the toolbar's validation.
            item.autovalidates = false
            update(item)
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
        case .doneFilter:
            item.isBordered = true
            item.target = self
            item.action = #selector(filterDone(_:))
        case .agendaFilter:
            item.isBordered = true
            item.target = self
            item.action = #selector(filterCalendar(_:))
        case .inboxFilter:
            item.isBordered = true
            item.target = self
            item.action = #selector(filterInbox(_:))
        case .today:
            let label = String(localized: "Today")
            item.title = label
            item.label = label
            item.isBordered = true
            item.target = self
            item.action = #selector(jumpToToday(_:))
        case .newItem:
            // A new item's page on the panel, the caret in its title
            // (`MainWindowModel.newItem`).
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
            // Enabled by `update`, not by the toolbar's validation.
            item.autovalidates = false
        default:
            return nil
        }
        update(item)
        return item
    }

    @objc private func jumpToToday(_ sender: Any?) {
        model.jumpToToday()
    }

    @objc private func signIn(_ sender: Any?) {
        model.onSignIn()
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

    /// Done's filter: a popover of checkboxes under the button. Each
    /// check applies at once, so there is nothing to confirm; a click
    /// outside (or the button again) shuts it.
    @objc private func filterDone(_ sender: Any?) {
        if let doneFilterPopover, doneFilterPopover.isShown {
            doneFilterPopover.performClose(nil)
            return
        }
        guard let item = toolbar.items.first(where: { $0.itemIdentifier == .doneFilter }) else { return }
        let popover = NSPopover()
        popover.behavior = .transient
        let host = NSHostingController(rootView: DoneFilterForm(model: model))
        host.sizingOptions = .preferredContentSize
        popover.contentViewController = host
        doneFilterPopover = popover
        popover.show(relativeTo: item)
    }

    /// The Calendar's filter: a popover under the button, as Done's.
    @objc private func filterCalendar(_ sender: Any?) {
        if let calendarFilterPopover, calendarFilterPopover.isShown {
            calendarFilterPopover.performClose(nil)
            return
        }
        guard let item = toolbar.items.first(where: { $0.itemIdentifier == .agendaFilter }) else { return }
        let popover = NSPopover()
        popover.behavior = .transient
        let host = NSHostingController(rootView: CalendarFilterForm(model: model))
        host.sizingOptions = .preferredContentSize
        popover.contentViewController = host
        calendarFilterPopover = popover
        popover.show(relativeTo: item)
    }

    /// The Inbox's filter: a popover under the button, as the
    /// Calendar's. A pick applies at once.
    @objc private func filterInbox(_ sender: Any?) {
        if let inboxFilterPopover, inboxFilterPopover.isShown {
            inboxFilterPopover.performClose(nil)
            return
        }
        guard let item = toolbar.items.first(where: { $0.itemIdentifier == .inboxFilter }) else { return }
        let popover = NSPopover()
        popover.behavior = .transient
        let host = NSHostingController(rootView: InboxFilterForm(model: model))
        host.sizingOptions = .preferredContentSize
        popover.contentViewController = host
        inboxFilterPopover = popover
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

    @objc private func newItem(_ sender: Any?) {
        model.newItem()
    }

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
                title: String(localized: "Delete"), action: #selector(deleteItem(_:)), keyEquivalent: "")
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

    /// Escape, when nothing nearer wants it, empties the panel: the
    /// selection, or a capture, is let go.
    override func cancelOperation(_ sender: Any?) {
        if model.panelOpen { model.closeItem() }
    }

}

/// The inspector pane: the selected item's page; without one, how
/// many rows are selected when there are several, and otherwise empty.
struct ItemPanelPane: View {
    @ObservedObject var model: MainWindowModel

    var body: some View {
        Group {
            // The page shown, which on a new selection is the old one
            // until the new one is read: its fields, and its writes,
            // are for the item it shows, in that item's workspace.
            // One `ItemPanel` for an item and for a capture, so the
            // item a capture becomes takes over its views as they are.
            let capture = model.capture
            let shown = model.selected != nil ? model.detailOf : nil
            let workspace = capture.map(\.workspace) ?? shown?.workspace
            if let identity = model.panelIdentity, capture != nil || shown != nil {
                ItemPanel(
                    item: capture.map(model.captureInfo) ?? model.detail, identity: identity,
                    isNew: capture != nil,
                    focusTitle: capture?.focusTitle ?? false,
                    workspaceName: model.sharedWorkspaceName(workspace),
                    lists: model.moveLists(workspace: workspace),
                    otherLists: model.otherMoveLists(workspace: workspace),
                    retitle: { item, text in
                        if capture != nil { return model.captureTitled(text) }
                        model.setText(workspace: workspace, item: item, text: text)
                    },
                    move: { item, list in
                        if capture != nil { return model.editCapture { $0.list = list } }
                        model.moveItem(workspace: workspace, id: item.item.id, list: list)
                    },
                    moveAcross: { item, target, list in
                        if capture != nil {
                            return model.editCapture {
                                $0.workspace = target
                                $0.list = list
                            }
                        }
                        model.moveItemAcross(
                            from: workspace, id: item.item.id, to: target, list: list, on: NSApp.keyWindow)
                    },
                    knownPlaces: { model.knownPlaces(matching: $0, completion: $1) },
                    setPlace: { item, place, exact in
                        if capture != nil {
                            return model.editCapture {
                                $0.place = place
                                $0.placeExact = exact
                            }
                        }
                        model.setPlace(workspace: workspace, id: item.item.id, place: place, exact: exact)
                    },
                    setWhen: { item, when, duration in
                        if capture != nil {
                            return model.editCapture {
                                $0.setWhen(when)
                                switch duration {
                                case .keep: break
                                case .clear: $0.duration = nil
                                case .minutes(let minutes): $0.duration = Int(minutes)
                                }
                            }
                        }
                        model.setWhen(workspace: workspace, id: item.item.id, when: when, duration: duration)
                    },
                    setWhenZone: { item, zone in
                        if capture != nil { return model.editCapture { $0.setZone(zone) } }
                        model.setWhenZone(workspace: workspace, id: item.item.id, zone: zone)
                    },
                    setEvent: { item, event in
                        if capture != nil { return model.editCapture { $0.event = event } }
                        model.setEvent(workspace: workspace, id: item.item.id, event: event)
                    },
                    // Assigned once it is an item.
                    assignment: capture == nil ? model.assignment(workspace: workspace) : nil,
                    setAssignee: { model.setAssignee(workspace: workspace, id: $0.item.id, assignee: $1) },
                    setDuration: { item, minutes in
                        if capture != nil { return model.editCapture { $0.duration = minutes } }
                        model.setDuration(workspace: workspace, id: item.item.id, minutes: minutes)
                    },
                    addSubtask: { model.addSubtask(workspace: workspace, id: $0.item.id, text: $1) },
                    setSubtaskText: {
                        model.setSubtaskText(workspace: workspace, id: $0.item.id, subtask: $1, text: $2)
                    },
                    setSubtaskDone: {
                        model.setSubtaskDone(workspace: workspace, id: $0.item.id, subtask: $1, done: $2)
                    },
                    moveSubtask: {
                        model.moveSubtask(workspace: workspace, id: $0.item.id, subtask: $1, index: $2)
                    },
                    removeSubtask: { model.removeSubtask(workspace: workspace, id: $0.item.id, subtask: $1) },
                    uploads: model.fileUploads.filter {
                        $0.workspace == workspace && $0.item == (capture == nil ? shown?.id : nil)
                    },
                    canAttach: model.accountId?.isEmpty == false,
                    attachFiles: { model.attachFiles(workspace: workspace, id: $0.item.id, urls: $1) },
                    openFile: { model.openFile(workspace: workspace, file: $0, done: $1) },
                    removeFile: { model.removeFile(workspace: workspace, fileId: $0.id) },
                    dismissUpload: { model.dismissUpload($0) })
            } else if model.selectedCount > 1 {
                Text("\(model.selectedCount) items selected")
                    .foregroundStyle(.secondary)
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
        label.font = .systemFont(ofSize: NSFont.systemFontSize, weight: .medium)
        label.textColor = .labelColor
        label.lineBreakMode = .byTruncatingTail
        label.setContentCompressionResistancePriority(.defaultLow, for: .horizontal)
        iconView.contentTintColor = .labelColor
        iconView.imageScaling = .scaleProportionallyUpOrDown
        iconView.isHidden = true
        countLabel.font = .systemFont(ofSize: NSFont.systemFontSize, weight: .regular)
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

/// A filter button's image: the funnel glyph, and while checks are in
/// effect their count beside it, the web's trigger
/// (`.done-filter-trigger-count`). One template image, so the toolbar
/// tints it and gives the item its own glass as for any other glyph.
enum FilterArt {
    private static let symbol = "line.3.horizontal.decrease"

    static func image(count: Int, label: String) -> NSImage? {
        let config = NSImage.SymbolConfiguration(pointSize: 13, weight: .regular)
        guard let glyph = NSImage(systemSymbolName: symbol, accessibilityDescription: label)?
            .withSymbolConfiguration(config)
        else { return nil }
        guard count > 0 else { return glyph }
        let text = NSAttributedString(string: "\(count)", attributes: [
            .font: NSFont.monospacedDigitSystemFont(ofSize: 12, weight: .medium),
            .foregroundColor: NSColor.black,
        ])
        let gap: CGFloat = 4
        let textSize = text.size()
        let height = max(glyph.size.height, ceil(textSize.height))
        let size = NSSize(width: ceil(glyph.size.width + gap + textSize.width), height: height)
        let image = NSImage(size: size, flipped: false) { _ in
            glyph.draw(in: NSRect(
                x: 0, y: (height - glyph.size.height) / 2,
                width: glyph.size.width, height: glyph.size.height))
            text.draw(at: NSPoint(x: glyph.size.width + gap, y: (height - textSize.height) / 2))
            return true
        }
        image.isTemplate = true
        image.accessibilityDescription = label
        return image
    }
}
