// A list's rows, Starred's, Assigned's and Done's, on `DndListView` (`apple/MonoplanDnd`,
// the web's list component on AppKit): several rows selected at once,
// dragged to reorder, moved with ⌘↑ / ⌘↓ (spec/mac-dnd-plan.md). The
// other lenses are still SwiftUI lists, and the Calendar a table.
//
// A list that takes new items ends in a row to type one into
// (`ItemAddRow`, in the list's trailer slot: the web's capture row).
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
    var state: ItemState
    /// The star at the row's far edge, or after the text when the
    /// small print has the edge.
    var starred = false
    /// Small print at the row's far edge (where the item lives).
    var note: String?
    /// Drawn in the secondary colour (a Done row in Starred).
    var muted = false
    /// Small print before the text (when a Done row entered Done).
    var stamp: String?
}

/// What the row for a new item tells its owner. The item panel shows
/// the item being typed (`ItemCapture`), so the owner hears of the row
/// as it goes, not only of the item at the end.
struct ItemListCapture {
    /// The row took the keyboard.
    let began: () -> Void
    /// Its text, as typed.
    let typed: (String) -> Void
    /// The text is an item: by Return (`chained`, the row keeps the
    /// keyboard) or by the keyboard leaving, `click` being the press
    /// that took it (in the window's coordinates) if one did.
    let submitted: (String, _ chained: Bool, _ click: NSPoint?) -> Void
    /// The row was left with nothing typed, by Escape (`cancelled`) or
    /// the keyboard going elsewhere.
    let ended: (_ cancelled: Bool, _ click: NSPoint?) -> Void
    /// Counts the submitted items now in the rows: on a change, the
    /// row lets go of the text it was holding for one.
    let settled: Int
}

/// A drag of the list's rows over what lies outside it: the sidebar's
/// rows take them (`SidebarDrops`). Points are the window's.
/// An item closed (done or cancelled) or reopened from this window,
/// for `ItemList.closes`. `serial` counts up across the window.
struct ItemListClose: Equatable {
    let serial: Int
    let id: String
    let closed: Bool
}

struct ItemListDrop {
    enum Outcome {
        /// Nothing there takes rows: the list goes on to its own drop.
        case ignored
        /// Taken. The rows stay selected if they are still to be seen
        /// where they were dropped (the list shown).
        case taken(keepsSelection: Bool)
    }

    /// The drag is at this point.
    let moved: (NSPoint, NSWindow?) -> Void
    /// The rows, in the list's order, were let go at this point.
    let dropped: ([ItemListRow], NSPoint, NSWindow?) -> Outcome
    /// The drag is over, however it ended.
    let ended: () -> Void
}

struct ItemList: NSViewRepresentable {
    /// What is shown: a change of it is another list, not an edit of
    /// this one, so its rows do not slide and it starts at the top.
    let surface: Surface
    let rows: [ItemListRow]
    /// The item whose panel is open, if one is.
    let selected: String?
    /// Whether rows can be dragged to another place in the list and
    /// ⌘-moved (not in an Inbox or Done).
    let reorder: Bool
    /// The selection came down to one row (its panel opens), or to
    /// none or several (it shuts).
    let select: (ItemListRow?) -> Void
    /// How many rows are selected, on each change of it: the panel
    /// says so when there are several.
    var selectedCount: (Int) -> Void = { _ in }
    /// A row's checkbox, with the new "done".
    let toggle: (ItemListRow, Bool) -> Void
    /// `ids`, in order, now sit directly above `before` (nil: last).
    let move: ([String], String?) -> Void
    /// The right-click menu for the selected rows; nil is a separator.
    let menu: ([ItemListRow]) -> [AgendaTable.RowAction?]
    /// The row after the last one, to type a new item into for the end
    /// of this list; nil is a list with no such row. It is this
    /// surface's own: text still in the row when another list is shown
    /// goes to the list it was typed under.
    var capture: ItemListCapture?
    /// Where rows dragged out of the list can go; nil is nowhere, and
    /// then a list that does not reorder does not drag either.
    var drop: ItemListDrop?
    /// The last rows are in sight or nearly: a list read a page at a
    /// time (Done) reads on. Asked again on each scroll while they are.
    var nearEnd: (() -> Void)?
    /// A row closed while shown here stays where it was, ticked, for a
    /// beat before it leaves (`Linger`): a list's rows, Starred's and
    /// Assigned's, which drop a row once it is done.
    var lingers = false
    /// The items this window closed or reopened lately, newest last,
    /// from whichever control: the checkbox, the panel's tick, the
    /// state menu. Those new since the last update start or join the
    /// linger.
    var closes: [ItemListClose] = []
    /// An untick of a lingering row, with the row now below it that
    /// is not lingering (nil: none), in place of `toggle`: Starred's,
    /// whose entry went when the row was ticked and comes back there.
    var reopenLingering: ((ItemListRow, _ before: String?) -> Void)?

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
        // The list posts its clip view's bounds changes already.
        NotificationCenter.default.addObserver(
            context.coordinator, selector: #selector(Coordinator.scrolled(_:)),
            name: NSView.boundsDidChangeNotification, object: list.scrollView.contentView)
        return list
    }

    func updateNSView(_ list: DndListView, context: Context) {
        context.coordinator.apply(self, to: list)
    }

    final class Coordinator: NSObject, DndListDataSource, DndListDelegate {
        var parent: ItemList
        private var rows: [String: ItemListRow] = [:]
        /// What the list shows: `parent.rows` with the lingering rows
        /// put back where they were.
        private var shown: [ItemListRow] = []
        private var linger = Linger()
        private var flush: DispatchWorkItem?
        /// The newest of `closes` already taken.
        private var seenClose = 0
        private var surface: Surface?
        /// The model's selection as last carried into the list.
        private var synced: String?
        /// Inside `apply`: the list's own notices are not the user's.
        private var applying = false
        /// The count last given to `selectedCount`.
        private var counted = 0
        private let addRow = ItemAddRow()
        private weak var list: DndListView?

        init(_ parent: ItemList) {
            self.parent = parent
            super.init()
            addRow.began = { [weak self] in self?.parent.capture?.began() }
            addRow.typed = { [weak self] in self?.parent.capture?.typed($0) }
            addRow.commit = { [weak self] in self?.parent.capture?.submitted($0, $1, $2) }
            addRow.ended = { [weak self] in self?.parent.capture?.ended($0, $1) }
            addRow.leave = { [weak self] toLastRow in
                guard let list = self?.list else { return }
                list.window?.makeFirstResponder(list)
                if toLastRow { list.perform(.selectOnlyLast) }
            }
        }

        func apply(_ next: ItemList, to list: DndListView) {
            applying = true
            defer {
                applying = false
                // Rows that left, or another list: the selection may
                // have changed with no notice. Told after this view
                // update, not inside it.
                if list.selectedKeys.count != counted {
                    DispatchQueue.main.async { [weak self, weak list] in
                        guard let self, let list else { return }
                        self.count(list)
                    }
                }
            }
            self.list = list
            let moved = surface != next.surface
            // Text typed under the last list is that list's item.
            if moved { addRow.flush() }
            // Ticked rows linger on the list they were ticked on only.
            if moved || !next.lingers { endLinger() }
            let fresh = next.closes.filter { $0.serial > seenClose }
            seenClose = next.closes.last?.serial ?? seenClose
            if !moved, next.lingers { take(fresh) }
            let display = linger.overlay(next.rows)
            let grew = !moved && display.count > shown.count
            // The item Return sent is in these rows, or wherever else
            // the panel sent it.
            let settled = !moved && next.capture?.settled != parent.capture?.settled
            parent = next
            shown = display
            rows = Dictionary(display.map { ($0.id, $0) }, uniquingKeysWith: { first, _ in first })
            list.model.config.reorder = next.reorder
            // An Inbox's rows have nowhere to go in it, and still drag
            // out to the sidebar.
            list.model.config.drag = next.reorder || next.drop != nil

            surface = next.surface
            list.setOrder(display.map(\.id), animated: !moved)
            list.setTrailer(next.capture == nil ? nil : addRow)
            if settled { addRow.settle() }
            list.reloadRows()
            if moved { list.scrollToTop() }
            // Typing one item after another: the row stays in view as
            // the list grows above it.
            if grew, addRow.isEditing { list.scrollToTrailer() }
            // A page that does not reach the end of the window.
            if next.nearEnd != nil {
                DispatchQueue.main.async { [weak self] in self?.checkEnd() }
            }

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

        @objc func scrolled(_ note: Notification) { checkEnd() }

        /// Within a windowful of the last row.
        private func checkEnd() {
            guard let nearEnd = parent.nearEnd, let list, !shown.isEmpty,
                  let content = list.scrollView.documentView else { return }
            let clip = list.scrollView.contentView.bounds
            if content.frame.maxY - clip.maxY < clip.height { nearEnd() }
        }

        // MARK: DndListDataSource

        func dndListMakeRowContent(_ list: DndListView) -> NSView { ItemRowContent() }

        func dndList(_ list: DndListView, configure content: NSView, key: String, state: DndRowState) {
            guard let content = content as? ItemRowContent, let row = rows[key] else { return }
            content.show(row, selected: state.selected) { [weak self] done in
                guard let self, let row = self.rows[key] else { return }
                if !done, let reopen = self.parent.reopenLingering, self.linger.has(row.id) {
                    reopen(row, self.linger.below(row.id, in: self.shown))
                } else {
                    self.parent.toggle(row, done)
                }
            }
        }

        // MARK: Linger

        /// A close of a row on screen holds it in place, ticked, and
        /// pushes the whole group's exit to a beat after this close;
        /// a reopen of a lingering row leaves it where it is, open.
        /// Taken from the rows as they were before this update, which
        /// may already lack the row if the write's read came first.
        private func take(_ closes: [ItemListClose]) {
            var closed = false
            for close in closes {
                if close.closed {
                    guard let row = shown.first(where: { $0.id == close.id }) else { continue }
                    linger.close(row, in: shown)
                    closed = true
                } else {
                    linger.reopen(close.id)
                }
            }
            if closed { armFlush() }
        }

        private func armFlush() {
            flush?.cancel()
            let work = DispatchWorkItem { [weak self] in
                guard let self else { return }
                self.endLinger()
                self.refresh()
            }
            flush = work
            DispatchQueue.main.asyncAfter(deadline: .now() + Linger.beat, execute: work)
        }

        private func endLinger() {
            flush?.cancel()
            flush = nil
            linger = Linger()
        }

        /// The same rows again, with the linger as it now is.
        private func refresh() {
            guard let list else { return }
            apply(parent, to: list)
        }

        // MARK: DndListDelegate

        func dndListSelectionDidChange(_ list: DndListView) {
            guard !applying else { return }
            let keys = list.selectedKeys
            let one = keys.count == 1 ? rows[keys[0]] : nil
            synced = one?.id
            parent.select(one)
            count(list)
        }

        private func count(_ list: DndListView) {
            let count = list.selectedKeys.count
            guard count != counted else { return }
            counted = count
            parent.selectedCount(count)
        }

        func dndList(_ list: DndListView, didMove keys: [String], before: String?) {
            parent.move(keys, before)
        }

        func dndList(_ list: DndListView, dragDidMoveTo windowPoint: NSPoint, keys: [String]) {
            parent.drop?.moved(windowPoint, list.window)
        }

        func dndList(_ list: DndListView, dragWillEndAt windowPoint: NSPoint, keys: [String]) -> Bool {
            guard let drop = parent.drop else { return false }
            // In the order shown, whatever order they were picked in.
            let dragged = Set(keys)
            let rows = shown.filter { dragged.contains($0.id) }
            switch drop.dropped(rows, windowPoint, list.window) {
            case .ignored:
                return false
            case .taken(let keepsSelection):
                // Rows that went elsewhere, or were starred or binned,
                // are done with, as on the web.
                if !keepsSelection {
                    list.model.selection.clear()
                    list.render()
                }
                return true
            }
        }

        func dndListDragDidEnd(_ list: DndListView) {
            parent.drop?.ended()
        }

        /// ↓ on the last row (or in an empty list) goes on to the row
        /// that takes a new item.
        func dndListNavigatePastEnd(_ list: DndListView) -> Bool {
            guard parent.capture != nil else { return false }
            addRow.focus()
            list.scrollToTrailer()
            return true
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

/// The web's done linger (`lingerChain` in `js/web/src/Workspace.tsx`):
/// a row ticked off a list that drops done rows stays where it was,
/// ticked, for `beat` before it goes. Each tick inside the beat of the
/// one before joins the group and holds it all another beat from
/// itself, so a run of ticks leaves together once the user stops,
/// rather than the rows under the pointer sliding up between clicks.
/// Purely a display tail: the item is Done the moment it is ticked.
/// Only ticks made in this list linger; a row closed elsewhere (another
/// window, another device) leaves at the next read, as it did before.
struct Linger {
    /// The web's `DONE_LINGER_MS`.
    static let beat: TimeInterval = 2

    private struct Entry {
        var row: ItemListRow
        /// Where the row was among the rows that do not linger, when
        /// it was ticked: each one after the rows ticked before it had
        /// gone, as the web's capture.
        let index: Int
        /// Unticked while lingering: the row stays, open, until a read
        /// has it back in its own place.
        var reopened = false
    }

    private var entries: [Entry] = []

    /// `row` was ticked in `shown`, the rows on screen.
    mutating func close(_ row: ItemListRow, in shown: [ItemListRow]) {
        entries.removeAll { $0.row.id == row.id }
        let lingering = Set(entries.map(\.row.id))
        let rest = shown.filter { !lingering.contains($0.id) }
        let index = rest.firstIndex { $0.id == row.id } ?? rest.count
        var ticked = row
        ticked.state = .done
        entries.append(Entry(row: ticked, index: index))
    }

    /// `id` is lingering, ticked or reopened.
    func has(_ id: String) -> Bool { entries.contains { $0.row.id == id } }

    /// The first row under `id` in `shown` that is not lingering.
    func below(_ id: String, in shown: [ItemListRow]) -> String? {
        guard let at = shown.firstIndex(where: { $0.id == id }) else { return nil }
        return shown[(at + 1)...].first { !has($0.id) }?.id
    }

    mutating func reopen(_ id: String) {
        guard let i = entries.firstIndex(where: { $0.row.id == id }) else { return }
        entries[i].reopened = true
        entries[i].row.state = .todo
    }

    /// `rows` as read, with the lingering ones where they were. A row
    /// still in the read (its write not yet read back) is shown ticked
    /// in its own place; one gone from it is put back at its index,
    /// replayed newest first (`restoreCapturedPositions` in
    /// `js/web/src/linger.ts`). A reopened row the read has back again
    /// is the read's.
    mutating func overlay(_ rows: [ItemListRow]) -> [ItemListRow] {
        guard !entries.isEmpty else { return rows }
        let present = Set(rows.map(\.id))
        entries.removeAll { $0.reopened && present.contains($0.row.id) }
        let lingering = Set(entries.map(\.row.id))
        var out = rows.filter { !lingering.contains($0.id) }
        for entry in entries.reversed() {
            out.insert(entry.row, at: min(max(entry.index, 0), out.count))
        }
        return out
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

/// A row's checkbox: the box `ItemCheckbox` draws, as a plain view that
/// takes its own clicks. Not a hosted SwiftUI button, which costs a
/// couple of milliseconds a row each time the list recycles one.
/// Held down, the box swells a touch and springs back on release, as
/// the web's does (`input.task-check[data-pressed]` in `styles.css`).
private final class RowCheck: NSView, DndInteractiveView {
    var state: ItemState = .todo {
        didSet { if state != oldValue { box.state = state } }
    }
    var toggle: (Bool) -> Void = { _ in }
    private let box = RowCheckBox()
    private var pressed = false {
        didSet { if pressed != oldValue { swell(pressed) } }
    }

    override init(frame: NSRect) {
        super.init(frame: frame)
        wantsLayer = true
        box.wantsLayer = true
        addSubview(box)
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError("not built from a nib") }

    override func layout() {
        super.layout()
        box.frame = bounds
    }

    /// The box is drawing only: every click is the control's.
    override func hitTest(_ point: NSPoint) -> NSView? {
        frame.contains(point) ? self : nil
    }

    /// The web's timings: out quickly while held (0.08 s ease-out), back
    /// over 0.22 s on a curve that overshoots a little before settling.
    private func swell(_ on: Bool) {
        guard let layer = box.layer else { return }
        let target = on ? Self.swollen(layer.bounds) : CATransform3DIdentity
        let from = layer.presentation()?.transform ?? layer.transform
        CATransaction.begin()
        CATransaction.setDisableActions(true)
        layer.transform = target
        CATransaction.commit()
        guard !NSWorkspace.shared.accessibilityDisplayShouldReduceMotion else {
            layer.removeAnimation(forKey: "swell")
            layer.transform = CATransform3DIdentity
            return
        }
        let animation = CABasicAnimation(keyPath: "transform")
        animation.fromValue = NSValue(caTransform3D: from)
        animation.toValue = NSValue(caTransform3D: target)
        animation.duration = on ? 0.08 : 0.22
        animation.timingFunction =
            on
            ? CAMediaTimingFunction(name: .easeOut)
            : CAMediaTimingFunction(controlPoints: 0.34, 1.56, 0.64, 1)
        layer.add(animation, forKey: "swell")
    }

    /// 1.15 about the middle: AppKit pins a view's layer at its corner,
    /// so the scale is wrapped in a move to the centre and back.
    private static func swollen(_ bounds: CGRect) -> CATransform3D {
        let scale: CGFloat = 1.15
        var t = CATransform3DMakeTranslation(bounds.midX, bounds.midY, 0)
        t = CATransform3DScale(t, scale, scale, 1)
        return CATransform3DTranslate(t, -bounds.midX, -bounds.midY, 0)
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

/// The drawing half of `RowCheck`, on its own layer so the swell can
/// scale it without the control's frame moving.
private final class RowCheckBox: NSView {
    var state: ItemState = .todo {
        didSet { needsDisplay = true }
    }

    override func draw(_ dirtyRect: NSRect) {
        ItemCheckArt.drawRowBox(state, in: bounds)
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
    private let stamp = NSTextField(labelWithString: "")
    private let star = NSImageView(image: StarArt.row)
    private var selected = false

    override var isFlipped: Bool { true }

    init() {
        super.init(frame: .zero)
        star.setAccessibilityLabel(String(localized: "Starred"))
        title.font = .systemFont(ofSize: NSFont.systemFontSize)
        title.lineBreakMode = .byTruncatingTail
        title.maximumNumberOfLines = 1
        note.font = .systemFont(ofSize: NSFont.systemFontSize)
        note.textColor = .tertiaryLabelColor
        note.lineBreakMode = .byTruncatingTail
        note.alignment = .right
        stamp.font = .monospacedDigitSystemFont(ofSize: NSFont.systemFontSize, weight: .regular)
        stamp.textColor = .secondaryLabelColor
        for view in [check, stamp, title, note, star] { addSubview(view) }
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError("not built from a nib") }

    func show(_ row: ItemListRow, selected: Bool, toggle: @escaping (Bool) -> Void) {
        check.state = row.state
        check.toggle = toggle
        title.stringValue = row.text
        title.textColor = row.muted ? .secondaryLabelColor : PaneColor.rowText
        note.stringValue = row.note ?? ""
        note.isHidden = (row.note ?? "").isEmpty
        stamp.stringValue = row.stamp ?? ""
        stamp.isHidden = (row.stamp ?? "").isEmpty
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
        // With small print at the edge, the star follows the text.
        let trailingStar = !star.isHidden && !note.isHidden
        if !star.isHidden, !trailingStar {
            star.frame = NSRect(x: right - 13, y: mid - 6.5, width: 13, height: 13)
            right = star.frame.minX - 8
        }
        var left = Self.edge + 15 + 8
        // The cell's size, not the intrinsic one: that leaves out
        // the cell's 2 pt of padding either side, and a frame that
        // narrow truncates every label. A frame overhangs its edge
        // by the padding so the glyphs still meet it.
        let pad: CGFloat = 2
        if !stamp.isHidden {
            // As wide as it reads: the text follows at the same gap.
            let width = ceil(stamp.cell?.cellSize.width ?? stamp.intrinsicContentSize.width)
            let height = ceil(stamp.intrinsicContentSize.height)
            stamp.frame = NSRect(x: left - pad, y: mid - height / 2, width: width, height: height)
            left = stamp.frame.maxX - pad + 8
        }
        if !note.isHidden {
            // The text keeps at least half the row.
            let wanted = ceil(note.cell?.cellSize.width ?? note.intrinsicContentSize.width)
            let width = min(wanted, max(0, (right - left) / 2))
            let height = ceil(note.intrinsicContentSize.height)
            note.frame = NSRect(x: right + pad - width, y: mid - height / 2, width: width, height: height)
            right = note.frame.minX - 8
        }
        let height = ceil(title.intrinsicContentSize.height)
        var width = max(0, right - left)
        if trailingStar {
            width = min(max(0, width - 13 - 6), ceil(title.cell?.cellSize.width ?? width))
            star.frame = NSRect(x: left + width + 6 - pad, y: mid - 6.5, width: 13, height: 13)
        }
        title.frame = NSRect(x: left, y: mid - height / 2, width: width, height: height)
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

/// The row after a list's last: a plus where a row has its checkbox,
/// and a field to type the next item into (the web's `.row-new`).
/// Return adds the item and keeps the keyboard for the one after;
/// Return on nothing and Escape give the keyboard back to the list, and
/// ↑ in the empty field goes to the last row. Text left in it when the
/// keyboard goes elsewhere is added too.
///
/// Return leaves the text in the field until the item is a row
/// (`settle`): the write is off the main thread, and a field emptied at
/// once shows its placeholder for a frame or two before the row draws
/// in its place. Held, the text stands still and the row appears under
/// it as the field empties and steps down.
private final class ItemAddRow: NSView, NSTextFieldDelegate {
    private static let edge: CGFloat = 20

    /// The field took the keyboard.
    var began: () -> Void = {}
    /// Its text as typed, less what Return has already sent.
    var typed: (String) -> Void = { _ in }
    /// The text is an item: by Return (true), or by the keyboard
    /// leaving, with the press that took it.
    var commit: (String, Bool, NSPoint?) -> Void = { _, _, _ in }
    /// Left with nothing typed: by Escape (true), and the press that
    /// took the keyboard.
    var ended: (Bool, NSPoint?) -> Void = { _, _ in }
    /// The keyboard goes back to the list, onto its last row if so.
    var leave: (Bool) -> Void = { _ in }

    private let plus = NSImageView()
    private let field = FocusField()
    /// Sent by Return and not yet a row; still in the field.
    private var pending: String?
    /// Escape is why the keyboard is leaving.
    private var cancelling = false

    override var isFlipped: Bool { true }

    init() {
        super.init(frame: .zero)
        plus.image = NSImage(systemSymbolName: "plus", accessibilityDescription: nil)
        plus.symbolConfiguration = .init(pointSize: 12, weight: .regular)
        plus.contentTintColor = .tertiaryLabelColor
        plus.setAccessibilityElement(false)
        field.font = .systemFont(ofSize: NSFont.systemFontSize)
        field.isBordered = false
        field.drawsBackground = false
        field.focusRingType = .none
        field.lineBreakMode = .byTruncatingTail
        field.cell?.usesSingleLineMode = true
        field.cell?.isScrollable = true
        // In the plus's ink, not the system's placeholder colour.
        field.placeholderAttributedString = NSAttributedString(
            string: String(localized: "Add an item…"),
            attributes: [.foregroundColor: NSColor.tertiaryLabelColor, .font: field.font as Any])
        field.delegate = self
        field.focused = { [weak self] in self?.began() }
        addSubview(plus)
        addSubview(field)
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError("not built from a nib") }

    var isEditing: Bool { field.currentEditor() != nil }

    func focus() {
        window?.makeFirstResponder(field)
    }

    /// The field's text less what Return has sent, trimmed.
    private var typedText: String {
        var text = Substring(field.stringValue)
        if let pending {
            let rest = text.drop { $0.isWhitespace || $0.isNewline }
            text = rest.hasPrefix(pending) ? rest.dropFirst(pending.count) : ""
        }
        return text.trimmingCharacters(in: .whitespacesAndNewlines)
    }

    /// Return: send what has been typed and hold it in the field until
    /// it lands. False when there is nothing to send.
    private func submit() -> Bool {
        // One on its way: Return again is not a second item.
        guard pending == nil else { return true }
        let text = typedText
        guard !text.isEmpty else { return false }
        pending = text
        commit(text, true, nil)
        // A write that failed lands nothing: the text stays, to send again.
        DispatchQueue.main.asyncAfter(deadline: .now() + 1) { [weak self] in
            if self?.pending == text { self?.pending = nil }
        }
        return true
    }

    /// The item Return sent is in the rows: the field lets go of it,
    /// and keeps anything typed after it.
    func settle() {
        guard pending != nil else { return }
        let rest = typedText
        pending = nil
        field.stringValue = rest
    }

    /// The row is done with: add what has been typed and not yet sent,
    /// if anything, and empty the field. `click` is the press that took
    /// the keyboard away.
    func flush(click: NSPoint? = nil) {
        let text = typedText
        pending = nil
        field.stringValue = ""
        let cancelled = cancelling
        cancelling = false
        if text.isEmpty {
            ended(cancelled, click)
        } else {
            commit(text, false, click)
        }
    }

    override func layout() {
        super.layout()
        let mid = bounds.midY
        // The checkbox's 15 pt slot, and the text column of the rows.
        plus.frame = NSRect(x: Self.edge, y: mid - 7.5, width: 15, height: 15)
        let left = Self.edge + 15 + 8
        let height = ceil(field.intrinsicContentSize.height)
        field.frame = NSRect(
            x: left, y: mid - height / 2, width: max(0, bounds.maxX - Self.edge - left), height: height)
    }

    /// Anywhere on the row is the field.
    override func mouseDown(with event: NSEvent) {
        focus()
    }

    override func resetCursorRects() {
        addCursorRect(bounds, cursor: .iBeam)
    }

    // MARK: NSTextFieldDelegate

    func control(_ control: NSControl, textView: NSTextView, doCommandBy selector: Selector) -> Bool {
        switch selector {
        case #selector(NSResponder.insertNewline(_:)):
            if !submit() { leave(false) }
            return true
        case #selector(NSResponder.cancelOperation(_:)):
            pending = nil
            field.stringValue = ""
            cancelling = true
            leave(false)
            cancelling = false
            return true
        case #selector(NSResponder.moveUp(_:)):
            guard field.stringValue.isEmpty else { return false }
            leave(true)
            return true
        default:
            return false
        }
    }

    func controlTextDidChange(_ obj: Notification) {
        typed(typedText)
    }

    func controlTextDidEndEditing(_ obj: Notification) {
        // A press that is taking the keyboard: where it landed says
        // whether the item being typed is wanted in the panel.
        var click: NSPoint?
        if let event = NSApp.currentEvent, event.type == .leftMouseDown || event.type == .leftMouseUp,
           event.window === window {
            click = event.locationInWindow
        }
        flush(click: click)
    }
}

/// A text field that says when it takes the keyboard.
private final class FocusField: NSTextField {
    var focused: () -> Void = {}

    override func becomeFirstResponder() -> Bool {
        let took = super.becomeFirstResponder()
        if took { focused() }
        return took
    }
}
