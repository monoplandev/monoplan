// A list's rows and Starred's, on `DndListView` (`apple/MonoplanDnd`,
// the web's list component on AppKit): several rows selected at once,
// dragged to reorder, moved with ⌘↑ / ⌘↓ (spec/mac-dnd-plan.md). The
// other lenses are still SwiftUI lists, and the Calendar a table.
//
// A row is AppKit throughout: the labels, the hairline, and the
// checkbox and star as AppKit drawings of the ones the rest of the app
// has (`ItemCheckArt.drawRowBox`, `StarArt.row`). They were hosted
// SwiftUI views until 2026-11-20; a jump down a list recycles a
// windowful of rows in a frame or two, and the hosting views made that
// drop frames. Only the checkbox takes its own clicks; a press anywhere
// else on the row selects or drags it.

import AppKit
import MonoplanCore
import MonoplanDnd
import MonoplanDndCore
import SwiftUI

/// One row as the list draws it.
struct ItemListRow: Equatable {
    let id: String
    /// The workspace the item lives in; nil is Home.
    let workspace: String?
    let text: String
    let state: ItemState
    /// The star at the row's far edge.
    var starred = false
    /// Small print at the row's far edge (where the item lives).
    var note: String?
    /// Drawn in the secondary colour (a Done row in Starred).
    var muted = false
}

struct ItemList: NSViewRepresentable {
    /// What is shown: a change of it is another list, not an edit of
    /// this one, so its rows do not slide and it starts at the top.
    let surface: Surface
    let rows: [ItemListRow]
    /// The item whose panel is open, if one is.
    let selected: String?
    /// Whether rows can be dragged and ⌘-moved (not in an Inbox).
    let reorder: Bool
    /// The selection came down to one row (its panel opens), or to
    /// none or several (it shuts).
    let select: (ItemListRow?) -> Void
    /// A row's checkbox, with the new "done".
    let toggle: (ItemListRow, Bool) -> Void
    /// `ids`, in order, now sit directly above `before` (nil: last).
    let move: ([String], String?) -> Void
    /// The right-click menu for the selected rows; nil is a separator.
    let menu: ([ItemListRow]) -> [AgendaTable.RowAction?]

    func makeCoordinator() -> Coordinator { Coordinator(self) }

    func makeNSView(context: Context) -> DndListView {
        var config = DndConfig()
        // The web's `ROW_HEIGHT`.
        config.itemHeight = 32
        // A click on no row shuts the panel, as it did in the list.
        config.clearOnBlankClick = true
        let list = DndListView(config: config)
        list.dataSource = context.coordinator
        list.delegate = context.coordinator
        // The Calendar's selection (`AgendaTable.RowView`).
        list.selectionInset = 10
        list.selectionRadius = 6
        list.selectionColor = PaneColor.rowSelected
        list.dormantSelectionColor = PaneColor.rowSelectedInactive
        list.placeholderColor = PaneColor.dropSlot
        list.dragBackgroundColor = PaneColor.list
        return list
    }

    func updateNSView(_ list: DndListView, context: Context) {
        context.coordinator.apply(self, to: list)
    }

    final class Coordinator: NSObject, DndListDataSource, DndListDelegate {
        var parent: ItemList
        private var rows: [String: ItemListRow] = [:]
        private var surface: Surface?
        /// The model's selection as last carried into the list.
        private var synced: String?
        /// Inside `apply`: the list's own notices are not the user's.
        private var applying = false

        init(_ parent: ItemList) {
            self.parent = parent
        }

        func apply(_ next: ItemList, to list: DndListView) {
            applying = true
            defer { applying = false }
            parent = next
            rows = Dictionary(next.rows.map { ($0.id, $0) }, uniquingKeysWith: { first, _ in first })
            list.model.config.reorder = next.reorder
            list.model.config.drag = next.reorder

            let moved = surface != next.surface
            surface = next.surface
            list.setOrder(next.rows.map(\.id), animated: !moved)
            list.reloadRows()
            if moved { list.scrollToTop() }

            // The model's selection moves the list's only when it has
            // itself changed (a search hit opened, the panel shut):
            // several rows selected is the list's own state, with no
            // panel, and is left alone.
            guard next.selected != synced || moved else { return }
            synced = next.selected
            if let id = next.selected {
                if list.selectedKeys != [id] {
                    list.model.selection.setSelectedKeys([id])
                    list.render()
                    list.scrollToKey(id)
                }
            } else if list.selectedKeys.count == 1 || moved {
                list.model.selection.clear()
                list.render()
            }
        }

        // MARK: DndListDataSource

        func dndListMakeRowContent(_ list: DndListView) -> NSView { ItemRowContent() }

        func dndList(_ list: DndListView, configure content: NSView, key: String, state: DndRowState) {
            guard let content = content as? ItemRowContent, let row = rows[key] else { return }
            content.show(row, selected: state.selected) { [weak self] done in
                guard let self, let row = self.rows[key] else { return }
                self.parent.toggle(row, done)
            }
        }

        // MARK: DndListDelegate

        func dndListSelectionDidChange(_ list: DndListView) {
            guard !applying else { return }
            let keys = list.selectedKeys
            let one = keys.count == 1 ? rows[keys[0]] : nil
            synced = one?.id
            parent.select(one)
        }

        func dndList(_ list: DndListView, didMove keys: [String], before: String?) {
            parent.move(keys, before)
        }

        func dndList(_ list: DndListView, menuFor keys: [String]) -> NSMenu? {
            let actions = parent.menu(keys.compactMap { rows[$0] })
            guard !actions.isEmpty else { return nil }
            let menu = NSMenu()
            for action in actions {
                menu.addItem(action.map { ClosureMenuItem($0) } ?? .separator())
            }
            return menu
        }
    }
}

private final class ClosureMenuItem: NSMenuItem {
    private let run: () -> Void

    init(_ action: AgendaTable.RowAction) {
        run = action.run
        super.init(title: action.title, action: #selector(fire(_:)), keyEquivalent: "")
        target = self
    }

    @available(*, unavailable)
    required init(coder: NSCoder) { fatalError("not built from a nib") }

    @objc private func fire(_ sender: Any?) { run() }
}

/// The checkbox, hosted: the one part of a row that takes its own
/// clicks.
/// A row's checkbox: the box `ItemCheckbox` draws, as a plain view that
/// takes its own clicks. Not a hosted SwiftUI button, which costs a
/// couple of milliseconds a row each time the list recycles one.
private final class RowCheck: NSView, DndInteractiveView {
    var state: ItemState = .todo {
        didSet { if state != oldValue { needsDisplay = true } }
    }
    var toggle: (Bool) -> Void = { _ in }
    private var pressed = false {
        didSet { if pressed != oldValue { needsDisplay = true } }
    }

    override func draw(_ dirtyRect: NSRect) {
        guard let context = NSGraphicsContext.current?.cgContext else { return }
        context.setAlpha(pressed ? 0.5 : 1)
        ItemCheckArt.drawRowBox(state, in: bounds)
    }

    override func acceptsFirstMouse(for event: NSEvent?) -> Bool { true }

    override func mouseDown(with event: NSEvent) { pressed = true }

    override func mouseDragged(with event: NSEvent) {
        pressed = bounds.contains(convert(event.locationInWindow, from: nil))
    }

    override func mouseUp(with event: NSEvent) {
        guard pressed else { return }
        pressed = false
        toggle(!state.isClosed)
    }

    override func isAccessibilityElement() -> Bool { true }
    override func accessibilityRole() -> NSAccessibility.Role? { .button }
    override func accessibilityLabel() -> String? {
        state == .cancelled
            ? String(localized: "Reopen")
            : state.isClosed ? String(localized: "Mark as not done") : String(localized: "Mark as done")
    }
    override func accessibilityPerformPress() -> Bool {
        toggle(!state.isClosed)
        return true
    }
}

/// A row's content: the checkbox, the text, then at the far edge the
/// star or the small print, over the hairline to the next row.
private final class ItemRowContent: NSView {
    private static let edge: CGFloat = 20
    private static let checkSide: CGFloat = 22

    private let check = RowCheck()
    private let title = NSTextField(labelWithString: "")
    private let note = NSTextField(labelWithString: "")
    private let star = NSImageView(image: StarArt.row)
    private var selected = false

    override var isFlipped: Bool { true }

    init() {
        super.init(frame: .zero)
        star.setAccessibilityLabel(String(localized: "Starred"))
        title.font = .systemFont(ofSize: NSFont.systemFontSize)
        title.lineBreakMode = .byTruncatingTail
        title.maximumNumberOfLines = 1
        note.font = .preferredFont(forTextStyle: .footnote)
        note.textColor = .tertiaryLabelColor
        note.lineBreakMode = .byTruncatingTail
        note.alignment = .right
        for view in [check, title, note, star] { addSubview(view) }
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError("not built from a nib") }

    func show(_ row: ItemListRow, selected: Bool, toggle: @escaping (Bool) -> Void) {
        check.state = row.state
        check.toggle = toggle
        title.stringValue = row.text
        title.textColor = row.muted ? .secondaryLabelColor : .labelColor
        note.stringValue = row.note ?? ""
        note.isHidden = (row.note ?? "").isEmpty
        star.isHidden = !row.starred
        if selected != self.selected {
            self.selected = selected
            needsDisplay = true
        }
        needsLayout = true
    }

    override func layout() {
        super.layout()
        let mid = bounds.midY
        let side = Self.checkSide
        check.frame = NSRect(x: Self.edge - (side - 15) / 2, y: mid - side / 2, width: side, height: side)
        var right = bounds.maxX - Self.edge
        if !star.isHidden {
            star.frame = NSRect(x: right - 13, y: mid - 6.5, width: 13, height: 13)
            right = star.frame.minX - 8
        }
        let left = Self.edge + 15 + 8
        if !note.isHidden {
            // The text keeps at least half the row.
            // The cell's size, not the intrinsic one: that leaves out
            // the cell's 2 pt of padding either side, and a frame that
            // narrow truncates every label. The frame overhangs `right`
            // by the padding so the glyphs still end on it.
            let pad: CGFloat = 2
            let wanted = ceil(note.cell?.cellSize.width ?? note.intrinsicContentSize.width)
            let width = min(wanted, max(0, (right - left) / 2))
            let height = ceil(note.intrinsicContentSize.height)
            note.frame = NSRect(x: right + pad - width, y: mid - height / 2, width: width, height: height)
            right = note.frame.minX - 8
        }
        let height = ceil(title.intrinsicContentSize.height)
        title.frame = NSRect(x: left, y: mid - height / 2, width: max(0, right - left), height: height)
    }

    /// The hairline under the row. A selected row has none: the
    /// selection's ground is the edge, and a line across a run of
    /// selected rows flickers as they slide.
    override func draw(_ dirtyRect: NSRect) {
        guard !selected else { return }
        PaneColor.rowDivider.setFill()
        NSRect(x: Self.edge, y: bounds.maxY - 1, width: max(0, bounds.width - Self.edge * 2), height: 1).fill()
    }
}
