// A workspace's lists in the sidebar, on `DndListView` (`apple/MonoplanDnd`,
// the web's list component on AppKit): drag one to reorder, or ⌘↑ / ⌘↓,
// through `Engine.moveLists` (spec/mac-dnd-plan.md phase 3, the list
// reorder half). The web's nav does the same with its own component
// (`nav.tsx` `onReorder`).
//
// The rest of the sidebar stays a SwiftUI `List`: the lenses (the one
// Inbox among them) and each workspace's heading.
// This is one row of that list, as tall as its lists, set into the
// `List` with no row insets and `bleed` wider than the cell each side,
// so its rows and their selection line up with the system rows above
// and below (measured off-screen against a sidebar `List`, macOS 26,
// 2026-11-22: cell x 16–204 of a 220 pt sidebar, selection 10–210,
// radius 8, icon column centred 27.25, title at 42, 28 pt rows).
//
// Items dragged from the list on screen drop onto these rows too: the
// block is one `SidebarDropZone` (`SidebarDrop.swift`), lighting the
// row under the drag.
//
// Its selection is its own: the list shown, when it is one of these.
// The `List` has no selection either: its rows draw their own grey
// selection to match these (`MainWindowView.sidebar`).

import AppKit
import MonoplanCore
import MonoplanDnd
import MonoplanDndCore
import SwiftUI

struct SidebarLists: NSViewRepresentable {
    /// A sidebar row's height (`sidebarRowSize` small, with `navRow`'s
    /// 4 pt more).
    static let rowHeight: CGFloat = 28
    /// How far the rows reach past the `List`'s cell either side: the
    /// system's selection does.
    static let bleed: CGFloat = 6

    /// The workspace these are the lists of; nil is Home.
    let workspace: String?
    /// The workspace's active lists, in its order (`NavSection.userLists`).
    let lists: [ListInfo]
    /// The list shown, if it is one of these.
    let selected: String?
    /// Each list's open count at its trailing edge (the preference).
    let counts: Bool
    /// The window is key: the labels in `PaneColor.navText`, else the
    /// dimmer `navTextInactive`.
    let active: Bool
    let select: (String) -> Void
    /// `ids` now sit directly above `before` (nil: last).
    let move: ([String], String?) -> Void
    /// Delete, from the row's right-click menu (it confirms first).
    let delete: (ListInfo) -> Void
    /// The window's drop zones: each row here takes dropped items.
    let drops: SidebarDrops

    /// The block's height for `count` lists.
    static func height(_ count: Int) -> CGFloat { CGFloat(count) * rowHeight }

    func makeCoordinator() -> Coordinator { Coordinator(self) }

    func makeNSView(context: Context) -> DndListView {
        var config = DndConfig()
        config.itemHeight = Self.rowHeight
        // One list at a time is shown, and dragged.
        config.multi = false
        let list = DndListView(config: config)
        list.scrolls = false
        list.dataSource = context.coordinator
        list.delegate = context.coordinator
        list.selectionInset = 0
        list.selectionRadius = 8
        // The SwiftUI rows' selection (`PaneColor.navSelected`), focused
        // or not, as the web nav's.
        list.selectionColor = PaneColor.navSelected
        list.dormantSelectionColor = PaneColor.navSelected
        list.placeholderColor = PaneColor.dropSlot
        list.dragBackgroundColor = PaneColor.sidebar
        context.coordinator.attach(list)
        drops.add(context.coordinator)
        return list
    }

    func updateNSView(_ list: DndListView, context: Context) {
        context.coordinator.apply(self, to: list)
    }

    final class Coordinator: NSObject, DndListDataSource, DndListDelegate, SidebarDropZone {
        var parent: SidebarLists
        private var rows: [String: ListInfo] = [:]
        /// Inside `apply`: the list's own notices are not the user's.
        private var applying = false
        private weak var list: DndListView?
        /// On the row items would drop on, under the rows (which are
        /// clear), as the drop slot's shade is.
        private let highlight = SidebarDropHighlight()
        /// The list `highlight` is on.
        private var lit: String?

        init(_ parent: SidebarLists) {
            self.parent = parent
        }

        func attach(_ list: DndListView) {
            self.list = list
            highlight.isHidden = true
            list.addSubview(highlight, positioned: .below, relativeTo: list.scrollView)
        }

        // MARK: SidebarDropZone

        func dropTarget(at point: NSPoint, in window: NSWindow) -> SidebarDropTarget? {
            guard let list, SidebarDrops.shows(list.bounds, of: list, at: point, in: window),
                  let key = list.key(at: list.convert(point, from: nil))
            else { return nil }
            return .list(workspace: parent.workspace, list: key)
        }

        func showDrop(_ target: SidebarDropTarget?) {
            if case .list(_, let id)? = target { lit = id } else { lit = nil }
            placeHighlight()
        }

        /// The rows do not scroll and are all one height: a row's frame
        /// is its place in the order.
        private func placeHighlight() {
            guard let list, let lit, let index = list.order.firstIndex(of: lit) else {
                highlight.lit = false
                highlight.isHidden = true
                return
            }
            highlight.frame = NSRect(
                x: 0, y: CGFloat(index) * SidebarLists.rowHeight,
                width: list.bounds.width, height: SidebarLists.rowHeight)
            highlight.isHidden = false
            highlight.lit = true
        }

        func apply(_ next: SidebarLists, to list: DndListView) {
            applying = true
            defer { applying = false }
            parent = next
            rows = Dictionary(next.lists.map { ($0.id, $0) }, uniquingKeysWith: { first, _ in first })
            list.setOrder(next.lists.map(\.id))
            list.reloadRows()
            sync(list)
            // The lists changed under a drag.
            if lit != nil { placeHighlight() }
        }

        /// The list's selection to the model's: the list shown, or none.
        private func sync(_ list: DndListView) {
            let wanted = parent.selected.flatMap { rows[$0] == nil ? nil : [$0] } ?? []
            guard list.selectedKeys != wanted else { return }
            if wanted.isEmpty {
                list.model.selection.clear()
            } else {
                list.model.selection.setSelectedKeys(wanted)
            }
            list.render()
        }

        // MARK: DndListDataSource

        func dndListMakeRowContent(_ list: DndListView) -> NSView { SidebarListRow() }

        func dndList(_ list: DndListView, configure content: NSView, key: String, state: DndRowState) {
            guard let content = content as? SidebarListRow, let info = rows[key] else { return }
            content.show(info, count: parent.counts, active: parent.active)
        }

        // MARK: DndListDelegate

        func dndListSelectionDidChange(_ list: DndListView) {
            guard !applying else { return }
            if let key = list.selectedKeys.first {
                if key != parent.selected { parent.select(key) }
            } else {
                // Escape, or a ⌘-click on the shown row: a list is still
                // shown, so it stays selected. After this notice, not
                // inside it.
                DispatchQueue.main.async { [weak self, weak list] in
                    guard let self, let list else { return }
                    self.sync(list)
                }
            }
        }

        func dndList(_ list: DndListView, didMove keys: [String], before: String?) {
            parent.move(keys, before)
        }

        func dndList(_ list: DndListView, menuFor keys: [String]) -> NSMenu? {
            guard let info = keys.first.flatMap({ rows[$0] }) else { return nil }
            let menu = NSMenu()
            menu.addItem(SidebarMenuItem(String(localized: "Delete")) { [weak self] in
                self?.parent.delete(info)
            })
            return menu
        }
    }
}

private final class SidebarMenuItem: NSMenuItem {
    private let run: () -> Void

    init(_ title: String, run: @escaping () -> Void) {
        self.run = run
        super.init(title: title, action: #selector(fire(_:)), keyEquivalent: "")
        target = self
    }

    @available(*, unavailable)
    required init(coder: NSCoder) { fatalError("not built from a nib") }

    @objc private func fire(_ sender: Any?) { run() }
}

/// One list's row: its emoji, or the document glyph without one, then
/// the name, then the open count if the preference shows it. The
/// SwiftUI rows' layout (`navRow` with a `Label`), in AppKit. In the
/// row's coordinates (it starts `bleed` left of the cell).
private final class SidebarListRow: NSView {
    /// The centre of the `Label`'s icon column.
    private static let iconCentre: CGFloat = 17.25
    /// The title's text, less the label's 2 pt padding.
    private static let titleX: CGFloat = 30
    /// The count's trailing edge, padding included.
    private static let countTrail: CGFloat = 7
    /// The baseline, from the row's top.
    private static let baseline: CGFloat = 18.5
    private static let font = NSFont.systemFont(ofSize: NSFont.systemFontSize)
    private static let countFont = NSFont.monospacedDigitSystemFont(ofSize: NSFont.systemFontSize, weight: .regular)

    private let glyph = NSImageView(image: NavArt.list)
    private let emoji = NSTextField(labelWithString: "")
    private let title = NSTextField(labelWithString: "")
    private let count = NSTextField(labelWithString: "")

    override var isFlipped: Bool { true }

    init() {
        super.init(frame: .zero)
        glyph.imageScaling = .scaleProportionallyUpOrDown
        emoji.font = Self.font
        emoji.alignment = .center
        title.font = Self.font
        title.lineBreakMode = .byTruncatingTail
        title.cell?.truncatesLastVisibleLine = true
        count.font = Self.countFont
        count.alignment = .right
        for view in [glyph, emoji, title, count] { addSubview(view) }
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError("not built from a nib") }

    func show(_ list: ListInfo, count shown: Bool, active: Bool) {
        let icon = list.icon.flatMap { $0.isEmpty ? nil : $0 }
        emoji.stringValue = icon ?? ""
        emoji.isHidden = icon == nil
        glyph.isHidden = icon != nil
        title.stringValue = list.name
        let open = Int(list.open)
        count.stringValue = shown && open > 0 ? "\(open)" : ""
        count.isHidden = count.stringValue.isEmpty
        // The same on the grey selection as off it, as the web nav; the
        // SwiftUI rows' colours (`MainWindowView.sidebar`).
        let ink = active ? PaneColor.navText : PaneColor.navTextInactive
        title.textColor = ink
        emoji.textColor = ink
        glyph.contentTintColor = ink
        count.textColor = active ? PaneColor.navCount : PaneColor.navCountInactive
        needsLayout = true
    }

    override func layout() {
        super.layout()
        let width = bounds.width
        let textTop = (Self.baseline - Self.font.ascender).rounded()
        let textHeight = ceil(Self.font.ascender - Self.font.descender + Self.font.leading)
        glyph.frame = NSRect(x: Self.iconCentre - 8, y: (bounds.height - 16) / 2, width: 16, height: 16)
        emoji.frame = NSRect(x: Self.iconCentre - 12, y: textTop, width: 24, height: textHeight)
        var trailing = width - Self.countTrail
        if !count.isHidden {
            let w = ceil(count.intrinsicContentSize.width)
            count.frame = NSRect(x: trailing - w, y: textTop, width: w, height: textHeight)
            trailing -= w + 4
        }
        title.frame = NSRect(x: Self.titleX, y: textTop, width: max(0, trailing - Self.titleX), height: textHeight)
    }

    override func isAccessibilityElement() -> Bool { true }
    override func accessibilityRole() -> NSAccessibility.Role? { .row }
    override func accessibilityLabel() -> String? {
        count.isHidden ? title.stringValue : "\(title.stringValue), \(count.stringValue)"
    }
}
