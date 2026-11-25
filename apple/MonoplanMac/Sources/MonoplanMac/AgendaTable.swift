// The Calendar's list, as an AppKit table (2026-11-17, after the iOS
// page's `AgendaCollection.swift`). The Calendar opens on Today with
// the days gone by above it, and a SwiftUI `List` cannot be opened
// part-way down reliably. Here every row has a fixed height, so the
// table knows exactly where Today's header is and the scroll view is
// put there, on each layout until the pane has settled, and again by
// the toolbar's "Today".
//
// What is overdue (the rows the engine leads Today with, in the overdue
// tone) has a section of its own right above Today, there only while
// something is overdue (`spec/calendar-plan.md` "Overdue"). When it is
// there it is what the table opens on and what "Today" comes back to,
// so what is owed is never left above the top of the pane.
//
// Every day from Today on has a header, empty or not, for two weeks
// ahead and through to the last day something is on: the days ahead
// read as a calendar, as the web's Upcoming does by default
// (`fillEmptyDays` in `js/web/src/dayGroups.ts`). Days gone by show
// only when something is on them. "Hide Empty Days" under the
// toolbar's filter button (`agendaHideEmptyDaysKey`, a device
// preference) leaves out every empty day but Today.
//
// The rows are the window's SwiftUI rows, hosted in the cells and
// rewritten in place; a day's header is a group row. No multiple
// selection: a click selects an item, which opens its panel, as in the
// SwiftUI lists. A right-click anywhere on an item's row
// selects it too and opens its menu: the menu is the table's, not the
// hosted content's, which would only take a click on what it draws.
// The selected row has a soft ground, not
// the accent colour: the web's, a pale blue in the key window and a
// grey in any other (`PaneColor.rowSelected`, `RowView`).
//
// Drag to reschedule (`spec/calendar-plan.md` "Reschedule"; the web's
// `Upcoming.tsx`): a press on an item's row that travels 4 pt picks the
// row up, the day section under the pointer is tinted, the pane's top
// and bottom edges scroll it, and letting go writes the row's date onto
// that day (`AgendaDrop`). Escape cancels. As on the web the agenda is
// not the dnd list, and takes its parts: the stack that follows the
// pointer is `DndDragOverlay` and the edge scroll `DndAutoscroll`
// (`apple/MonoplanDnd`). Nothing of the table's own drag is used
// (`spec/mac-dnd-plan.md` "Phase 2 attempt"): no `NSDraggingSession`,
// no pasteboard, no drop feedback of AppKit's. One row at a time, the
// table's selection being one row.
//
// A `when` covering several days has a row on each (`Engine.agenda`
// spreads it, each row with its `span`): "Day 2" under the title, the
// time lane what the day holds of it (`AgendaEntry.lane`), and only the
// first row shown picks up a drag, as the web's span tails.

import AppKit
import MonoplanCore
import MonoplanDnd
import MonoplanDndCore
import MonoplanMacKit
import SwiftUI

struct AgendaTable: NSViewRepresentable {
    let groups: [AgendaGroup]
    /// The selected item's id, if one is.
    let selected: String?
    /// The model's count of "Today" presses: a new value scrolls back
    /// to Today, or to Overdue above it when something is.
    let todayJumps: Int
    /// Every day from Today on gets a header (`AgendaGroup.filled`); false is only
    /// Today and the days something is on.
    let fillEmptyDays: Bool
    let heading: (AgendaGroup) -> String
    /// A row's content height; the table adds 4 pt above and below,
    /// as a SwiftUI list does.
    let rowHeight: (AgendaEntry) -> CGFloat
    let row: (AgendaEntry) -> AnyView
    /// A row was picked; nil for a click on no row.
    let select: (AgendaEntry?) -> Void
    /// A row's right-click menu, top to bottom; nil is a separator.
    let menu: (AgendaEntry) -> [RowAction?]
    /// A row was dragged onto a day (`YYYY-MM-DD`, never Overdue).
    /// True when that writes something: the row is then on its way to
    /// another day, and is not shown where it was in the meantime.
    var reschedule: (AgendaEntry, String) -> Bool = { _, _ in false }

    /// One line of a row's menu.
    struct RowAction {
        let title: String
        let run: () -> Void
    }

    func makeCoordinator() -> Coordinator { Coordinator(self) }

    func makeNSView(context: Context) -> NSScrollView {
        let coordinator = context.coordinator
        let table = coordinator.table
        table.style = .inset
        table.headerView = nil
        let column = NSTableColumn(identifier: NSUserInterfaceItemIdentifier("row"))
        column.resizingMask = .autoresizingMask
        table.addTableColumn(column)
        table.columnAutoresizingStyle = .uniformColumnAutoresizingStyle
        table.backgroundColor = .clear
        table.usesAutomaticRowHeights = false
        table.intercellSpacing = NSSize(width: table.intercellSpacing.width, height: 0)
        table.allowsEmptySelection = true
        table.allowsMultipleSelection = false
        // A day's header scrolls away with its rows: one pinned under
        // the toolbar broke the page's flow for little.
        table.floatsGroupRows = false
        table.dataSource = coordinator
        table.delegate = coordinator
        table.rowMenu = { [weak coordinator] in coordinator?.menu(forRow: $0) }
        table.press = { [weak coordinator] in coordinator?.pressed($0) ?? false }

        let scroll = coordinator.scroll
        scroll.documentView = table
        scroll.hasVerticalScroller = true
        scroll.autohidesScrollers = true
        scroll.drawsBackground = false
        scroll.onLayout = { [weak coordinator] in coordinator?.laidOut() }
        NotificationCenter.default.addObserver(
            coordinator, selector: #selector(Coordinator.userScrolled(_:)),
            name: NSScrollView.willStartLiveScrollNotification, object: scroll)
        coordinator.show()
        return scroll
    }

    func updateNSView(_ scroll: NSScrollView, context: Context) {
        context.coordinator.parent = self
        context.coordinator.show()
    }

    /// A scroll view that says when it has been laid out: the pane's
    /// insets (the toolbar over it) settle there.
    final class LayoutScrollView: NSScrollView {
        var onLayout: (() -> Void)?

        override func layout() {
            super.layout()
            onLayout?()
        }
    }

    /// A table that asks for the menu of the row under a right-click,
    /// wherever on the row it lands.
    final class MenuTable: NSTableView {
        var rowMenu: ((Int) -> NSMenu?)?

        override func menu(for event: NSEvent) -> NSMenu? {
            let row = row(at: convert(event.locationInWindow, from: nil))
            return row >= 0 ? rowMenu?(row) : nil
        }

        /// A press the table is given; true when it was taken (and
        /// followed to the button's release), so the table's own
        /// tracking is not run for it.
        var press: ((NSEvent) -> Bool)?

        override func mouseDown(with event: NSEvent) {
            if press?(event) == true { return }
            super.mouseDown(with: event)
        }
    }

    /// A menu line that runs a closure.
    private final class ActionItem: NSMenuItem {
        private let run: () -> Void

        init(_ action: RowAction) {
            run = action.run
            super.init(title: action.title, action: #selector(fire(_:)), keyEquivalent: "")
            target = self
        }

        @available(*, unavailable)
        required init(coder: NSCoder) { fatalError("not built from a nib") }

        @objc private func fire(_ sender: Any?) { run() }
    }

    /// One table row.
    private enum Line {
        case header(day: String, title: String, today: Bool)
        /// `divided`: a line under it, as between two rows of a day.
        case item(AgendaEntry, day: String, height: CGFloat, divided: Bool)
        /// A day's line when nothing is on it.
        case nothing(day: String, today: Bool)

        /// The row's identity, and the shape the table laid out: two
        /// lines with the same key have the same kind and place.
        var key: String {
            switch self {
            case .header(let day, _, _): return "h/\(day)"
            case .item(let row, let day, _, _): return "i/\(day)/\(row.docId ?? "")/\(row.id)"
            case .nothing(let day, _): return "n/\(day)"
            }
        }

        var height: CGFloat {
            switch self {
            // Tight above the label, its old gap kept below it to the
            // line: a day reads close under the one before.
            case .header: return 22
            case .item(_, _, let height, _): return height + 8
            case .nothing: return 24
            }
        }

        var entry: AgendaEntry? {
            if case .item(let row, _, _, _) = self { return row }
            return nil
        }

        /// The section the line is in: a day, or Overdue's.
        var day: String {
            switch self {
            case .header(let day, _, _), .item(_, let day, _, _), .nothing(let day, _): return day
            }
        }
    }

    final class Coordinator: NSObject, NSTableViewDataSource, NSTableViewDelegate {
        var parent: AgendaTable
        let table = MenuTable()
        let scroll = LayoutScrollView()

        private var lines: [Line] = []
        /// The last "Today" press acted on.
        private var jumps: Int
        /// The table has been put on Today once: only the first rows
        /// do it, a reread leaves the scroll where it is.
        private var opened = false
        /// Still being put on Today: every layout puts Today's header
        /// back at the top, until the pane has settled or the user
        /// scrolls.
        private var anchoring = false
        /// The selection is being set from the model, not picked.
        private var syncing = false

        init(_ parent: AgendaTable) {
            self.parent = parent
            jumps = parent.todayJumps
        }

        deinit { NotificationCenter.default.removeObserver(self) }

        // MARK: Rows

        /// Today always, the days ahead always when `fillEmptyDays`
        /// (`AgendaGroup.filled`), any other day when it has rows, and Overdue
        /// above Today when Today's group has overdue rows.
        private func build() -> [Line] {
            var lines: [Line] = []
            var days: Set<String> = []
            func items(_ rows: [AgendaEntry], day: String) {
                for (index, row) in rows.enumerated() {
                    lines.append(.item(
                        row, day: day, height: parent.rowHeight(row),
                        divided: index < rows.count - 1))
                }
            }
            let groups = parent.fillEmptyDays
                ? AgendaGroup.filled(parent.groups)
                : AgendaGroup.nonEmpty(parent.groups)
            for group in groups {
                guard days.insert(group.day).inserted else { continue }
                var rows = group.rows
                if group.today {
                    let overdue = rows.filter { $0.tone == .overdue }
                    rows.removeAll { $0.tone == .overdue }
                    if !overdue.isEmpty {
                        lines.append(.header(
                            day: Self.overdueDay, title: String(localized: "Overdue"), today: false))
                        items(overdue, day: Self.overdueDay)
                    }
                }
                lines.append(.header(day: group.day, title: parent.heading(group), today: group.today))
                if rows.isEmpty {
                    lines.append(.nothing(day: group.day, today: group.today))
                }
                items(rows, day: group.day)
            }
            return lines
        }

        /// The Overdue section's place in a line's key, where a day's
        /// date is for the others.
        private static let overdueDay = "overdue"

        /// Show the parent's days. The same rows in the same places
        /// are rewritten where they are (a hosted cell built again is
        /// the slow path: `spec/mac-dnd-plan.md`); anything else
        /// reloads, holding the row at the top of the pane in place.
        func show() {
            let lines = build()
            let same = lines.count == self.lines.count
                && zip(lines, self.lines).allSatisfy { $0.key == $1.key && $0.height == $1.height }
            if same {
                self.lines = lines
                for index in visibleRows() {
                    if let cell = table.view(atColumn: 0, row: index, makeIfNecessary: false) {
                        fill(cell, lines[index], selected: index == table.selectedRow)
                    }
                }
            } else {
                let held = anchoring ? nil : topRow()
                // The dropped row has landed (or the rows are others).
                landing = nil
                change(to: lines)
                if let held { restore(held) }
            }
            syncSelection()

            if !opened, anchorRow() != nil {
                opened = true
                anchoring = true
                scroll.needsLayout = true
            }
            if jumps != parent.todayJumps {
                jumps = parent.todayJumps
                anchoring = false
                putTodayAtTop(animated: true)
            }
        }

        /// Put `next` in the table by taking out the lines that left
        /// and putting in the ones that came, the rest staying as they
        /// are. `reloadData` builds every hosted row in view again,
        /// near 2 ms each (45 ms for a pane of 25, measured
        /// 2026-11-23): a row moved to another day was that long in
        /// arriving. Here only the lines that came are built.
        private func change(to next: [Line]) {
            let old = lines
            let difference = next.map(\.key).difference(from: old.map(\.key))
            // Nothing to keep (the first rows, another account's): a
            // reload is the same work.
            guard !old.isEmpty, difference.count < old.count else {
                lines = next
                table.reloadData()
                return
            }
            var removed = IndexSet(), inserted = IndexSet()
            for step in difference {
                switch step {
                case .remove(let offset, _, _): removed.insert(offset)
                case .insert(let offset, _, _): inserted.insert(offset)
                }
            }
            // A selected line that leaves takes the table's selection
            // with it; that is not the user's doing, and `show` puts
            // the selection back by id.
            syncing = true
            table.beginUpdates()
            lines = next
            // Removals count in the old lines, insertions in the new.
            table.removeRows(at: removed, withAnimation: [])
            table.insertRows(at: inserted, withAnimation: [])
            table.endUpdates()
            syncing = false

            // A line that stayed may be another height now (a row that
            // gained its second line of small print).
            var heights: [String: CGFloat] = [:]
            for line in old { heights[line.key] = line.height }
            var resized = IndexSet()
            for (index, line) in next.enumerated() where !inserted.contains(index) {
                if let was = heights[line.key], was != line.height { resized.insert(index) }
            }
            if !resized.isEmpty { table.noteHeightOfRows(withIndexesChanged: resized) }
            for index in visibleRows() {
                if let cell = table.view(atColumn: 0, row: index, makeIfNecessary: false) {
                    fill(cell, lines[index], selected: index == table.selectedRow)
                }
            }
            markRows()
        }

        private func visibleRows() -> Range<Int> {
            let range = table.rows(in: table.visibleRect)
            return range.location..<min(range.location + range.length, lines.count)
        }

        func numberOfRows(in tableView: NSTableView) -> Int { lines.count }

        func tableView(_ tableView: NSTableView, heightOfRow row: Int) -> CGFloat { lines[row].height }

        func tableView(_ tableView: NSTableView, isGroupRow row: Int) -> Bool {
            if case .header = lines[row] { return true }
            return false
        }

        func tableView(_ tableView: NSTableView, viewFor tableColumn: NSTableColumn?, row: Int) -> NSView? {
            let line = lines[row]
            let cell: NSView
            if case .header = line {
                cell = tableView.makeView(withIdentifier: HeaderCell.identifier, owner: nil) ?? HeaderCell()
            } else {
                cell = tableView.makeView(withIdentifier: HostCell.identifier, owner: nil) ?? HostCell()
            }
            fill(cell, line, selected: row == tableView.selectedRow)
            return cell
        }

        func tableView(_ tableView: NSTableView, rowViewForRow row: Int) -> NSTableRowView? {
            tableView.makeView(withIdentifier: RowView.identifier, owner: nil) as? RowView ?? RowView()
        }

        /// A row view came into use (scrolled into view, a reload): it
        /// may be a recycled one, so its drag marks are set every time.
        func tableView(_ tableView: NSTableView, didAdd rowView: NSTableRowView, forRow row: Int) {
            mark(rowView, row: row)
        }

        /// `selected`: the row is the selected one, which has no line
        /// under it (it would cross the selection's ground).
        private func fill(_ cell: NSView, _ line: Line, selected: Bool) {
            switch line {
            case .header(_, let title, _):
                (cell as? HeaderCell)?.label.stringValue = title
            case .item(let row, _, _, let divided):
                (cell as? HostCell)?.show(parent.row(row), divided: divided && !selected)
            case .nothing(_, let today):
                (cell as? HostCell)?.show(
                    AnyView((today ? Text("Nothing today") : Text("Nothing scheduled")).italic().foregroundStyle(.tertiary)
                        .frame(maxWidth: .infinity, alignment: .leading)),
                    divided: false)
            }
        }

        // MARK: Selection

        func tableView(_ tableView: NSTableView, shouldSelectRow row: Int) -> Bool {
            lines[row].entry != nil
        }

        func tableViewSelectionDidChange(_ notification: Notification) {
            // The line under a row goes with its selection.
            for index in visibleRows() {
                if let cell = table.view(atColumn: 0, row: index, makeIfNecessary: false) {
                    fill(cell, lines[index], selected: index == table.selectedRow)
                }
            }
            guard !syncing else { return }
            let row = table.selectedRow
            parent.select(row >= 0 ? lines[row].entry : nil)
        }

        /// An item row's menu, for a right-click on it: the row is
        /// selected first, as a click would, so the menu is for the
        /// item whose panel is open.
        fileprivate func menu(forRow row: Int) -> NSMenu? {
            guard row < lines.count, let entry = lines[row].entry else { return nil }
            let actions = parent.menu(entry)
            guard !actions.isEmpty else { return nil }
            if table.selectedRow != row {
                table.selectRowIndexes([row], byExtendingSelection: false)
            }
            let menu = NSMenu()
            menu.autoenablesItems = false
            menu.items = actions.map { $0.map(ActionItem.init) ?? .separator() }
            return menu
        }

        /// Put the table's selection on the model's item. A reload
        /// keeps a selection by position, so it is set again by id.
        private func syncSelection() {
            let current = table.selectedRow
            let currentId = current >= 0 && current < lines.count ? lines[current].entry?.id : nil
            guard currentId != parent.selected || (current >= 0 && currentId == nil) else { return }
            syncing = true
            if let id = parent.selected, let row = lines.firstIndex(where: { $0.entry?.id == id }) {
                table.selectRowIndexes([row], byExtendingSelection: false)
            } else {
                table.deselectAll(nil)
            }
            syncing = false
        }

        // MARK: Drag

        /// The travel that turns a press into a drag: the web's
        /// `DRAG_START_PX`.
        private static let dragStart: CGFloat = 4

        /// A row in the air.
        private struct Drag {
            let entry: AgendaEntry
            /// The dragged line's key: its row is faint while it is up.
            let key: String
            let overlay: DndDragOverlay?
            /// The pointer, in the window's coordinates.
            var pointer: NSPoint
            /// The day the row would land on.
            var day: String?
        }

        private var drag: Drag?
        /// The key of a line dropped on another day whose write has
        /// not come back as rows yet. It is hidden until then: the
        /// write is off the main thread, and the row shown again where
        /// it was reads as the drop undone for a frame or two.
        private var landing: String?
        private var autoscroll = DndAutoscroll(buffer: 48)
        private var link: CADisplayLink?
        private var lastTick: CFTimeInterval = 0

        /// A press on the table. One on an item's row with no modifier
        /// is taken here: the row is selected at once, as a click
        /// does, and the pointer is followed until the button comes up,
        /// as a drag once it has travelled `dragStart`. The events are
        /// read here, not left to `mouseDragged` / `mouseUp`: a press
        /// can reach the table through a hosted SwiftUI row, and where
        /// the rest of it would be delivered then is not the table's
        /// to say.
        fileprivate func pressed(_ event: NSEvent) -> Bool {
            guard event.clickCount == 1,
                  event.modifierFlags.intersection([.shift, .command, .control, .option]).isEmpty,
                  let window = table.window else { return false }
            let row = table.row(at: table.convert(event.locationInWindow, from: nil))
            guard row >= 0, row < lines.count, let entry = lines[row].entry else { return false }
            // A multi-day span drags by its first row shown only (the
            // web's `data-span-tail`): a later day's row is clicked as
            // the table clicks it, and picks up nothing.
            if entry.span != nil, lines.firstIndex(where: {
                $0.entry.map { $0.id == entry.id && $0.docId == entry.docId } ?? false
            }) != row {
                return false
            }
            let key = lines[row].key
            window.makeFirstResponder(table)
            if table.selectedRow != row {
                table.selectRowIndexes([row], byExtendingSelection: false)
            }

            let start = event.locationInWindow
            var cancelled = false
            while let next = window.nextEvent(matching: [.leftMouseDragged, .leftMouseUp, .keyDown]) {
                switch next.type {
                case .leftMouseDragged:
                    let point = next.locationInWindow
                    if drag == nil {
                        guard !cancelled, hypot(point.x - start.x, point.y - start.y) >= Self.dragStart else { continue }
                        beginDrag(entry, key: key, from: start, in: window)
                    }
                    moveDrag(to: point)
                case .leftMouseUp:
                    endDrag(commit: true)
                    return true
                case .keyDown:
                    // Escape puts the row back; the button is still
                    // down, so the press is followed to its release.
                    // Any other key is not for a row in the air.
                    if next.keyCode == 53, drag != nil {
                        cancelled = true
                        endDrag(commit: false)
                    }
                default:
                    break
                }
            }
            endDrag(commit: false)
            return true
        }

        private func beginDrag(_ entry: AgendaEntry, key: String, from start: NSPoint, in window: NSWindow) {
            var overlay: DndDragOverlay?
            if let index = lines.firstIndex(where: { $0.key == key }),
               let rowView = table.rowView(atRow: index, makeIfNecessary: false) {
                // The part of the row the selection covers.
                let rect = rowView.bounds.insetBy(dx: RowView.inset, dy: 0)
                var snapshots: [CGImage] = []
                if rect.width > 0, rect.height > 0, let rep = rowView.bitmapImageRepForCachingDisplay(in: rect) {
                    rowView.cacheDisplay(in: rect, to: rep)
                    if let image = rep.cgImage { snapshots.append(image) }
                }
                var background = NSColor.windowBackgroundColor.cgColor
                var accent = NSColor.controlAccentColor.cgColor
                table.effectiveAppearance.performAsCurrentDrawingAppearance {
                    background = PaneColor.list.cgColor
                    accent = NSColor.controlAccentColor.cgColor
                }
                overlay = DndDragOverlay(
                    parent: window,
                    snapshots: snapshots,
                    itemSize: rect.size,
                    totalCount: 1,
                    cursor: window.convertPoint(toScreen: start),
                    grabRect: window.convertToScreen(rowView.convert(rect, to: nil)),
                    radius: RowView.radius,
                    background: background,
                    badgeColor: accent,
                    badgeText: nil)
            }
            drag = Drag(entry: entry, key: key, overlay: overlay, pointer: start, day: nil)
            // The table is the user's now, not the opening scroll's.
            anchoring = false
            markRows()
            lastTick = 0
            let link = table.displayLink(target: self, selector: #selector(tick(_:)))
            link.add(to: .main, forMode: .common)
            self.link = link
        }

        private func moveDrag(to point: NSPoint) {
            guard drag != nil, let window = table.window else { return }
            drag?.pointer = point
            drag?.overlay?.update(cursor: window.convertPoint(toScreen: point))
            autoscroll.update(pointer: viewportPoint(point), viewport: viewport())
            retarget()
        }

        /// The drag is over: let go on a day (`commit`), or put back.
        private func endDrag(commit: Bool) {
            guard let drag else { return }
            self.drag = nil
            drag.overlay?.stop()
            autoscroll.stop()
            link?.invalidate()
            link = nil
            if commit, let day = drag.day, parent.reschedule(drag.entry, day) {
                let key = drag.key
                landing = key
                // A write that failed lands nothing: the row comes back.
                DispatchQueue.main.asyncAfter(deadline: .now() + 1) { [weak self] in
                    guard let self, landing == key else { return }
                    landing = nil
                    markRows()
                }
            }
            markRows()
        }

        /// One frame of a drag: the edge scroll, the stack closing on
        /// the pointer, and the day under a pointer that may not have
        /// moved while the days did.
        @objc private func tick(_ link: CADisplayLink) {
            let dt = lastTick == 0 ? 1.0 / 60 : link.timestamp - lastTick
            lastTick = link.timestamp
            let step = min(max(dt, 0), 0.05)
            let delta = autoscroll.step(dt: step)
            if delta != 0 {
                scrollTo(y: scroll.contentView.bounds.origin.y + delta, animated: false)
            }
            drag?.overlay?.step(dt: step)
            retarget()
        }

        /// `point` (the window's) as the dnd parts take one: from the
        /// pane's top left, y down, not scrolled.
        private func viewportPoint(_ point: NSPoint) -> CGPoint {
            let p = scroll.convert(point, from: nil)
            return CGPoint(x: p.x, y: scroll.isFlipped ? p.y : scroll.bounds.height - p.y)
        }

        private func viewport() -> DndViewport {
            DndViewport(
                scrollTop: scroll.contentView.bounds.origin.y,
                width: scroll.bounds.width, height: scroll.bounds.height,
                topInset: scroll.contentInsets.top)
        }

        /// The day whose section is under `point` (the window's): its
        /// header, its rows or its empty line. Any day takes a drop,
        /// one gone by included; Overdue is not a day. Nil off the
        /// pane, under the toolbar and past the last row.
        private func day(at point: NSPoint) -> String? {
            let p = viewportPoint(point)
            guard p.x >= 0, p.x <= scroll.bounds.width,
                  p.y >= scroll.contentInsets.top, p.y <= scroll.bounds.height else { return nil }
            let row = table.row(at: table.convert(point, from: nil))
            guard row >= 0, row < lines.count else { return nil }
            let day = lines[row].day
            return day == Self.overdueDay ? nil : day
        }

        private func retarget() {
            guard let drag else { return }
            let day = self.day(at: drag.pointer)
            guard day != drag.day else { return }
            self.drag?.day = day
            markRows()
        }

        /// Every row view in use takes its drag marks.
        private func markRows() {
            table.enumerateAvailableRowViews { rowView, row in self.mark(rowView, row: row) }
        }

        /// The row's part in the drag: faint when it is the one in the
        /// air, tinted when its day is where the drop would land (the
        /// tint's corners are the section's, so the day reads as one
        /// block).
        private func mark(_ rowView: NSTableRowView, row: Int) {
            guard let rowView = rowView as? RowView else { return }
            guard let drag, row >= 0, row < lines.count else {
                let hidden = landing != nil && row >= 0 && row < lines.count && lines[row].key == landing
                rowView.alphaValue = hidden ? 0 : 1
                rowView.drop = nil
                return
            }
            let line = lines[row]
            rowView.alphaValue = line.key == drag.key ? 0.4 : 1
            if line.day == drag.day {
                let first = row == 0 || lines[row - 1].day != line.day
                let last = row == lines.count - 1 || lines[row + 1].day != line.day
                rowView.drop = RowView.Drop(first: first, last: last)
            } else {
                rowView.drop = nil
            }
        }

        // MARK: Scroll

        /// The header the table opens on and "Today" comes back to:
        /// Overdue's when it is there (it sits right above Today's),
        /// else Today's.
        private func anchorRow() -> Int? {
            lines.firstIndex {
                if case .header(let day, _, let today) = $0 { return today || day == Self.overdueDay }
                return false
            }
        }

        /// The pane was laid out: while the table is still opening,
        /// Today's header goes back to the top. Half a second after
        /// the first layout in a window the scroll is the user's.
        fileprivate func laidOut() {
            guard anchoring else { return }
            putTodayAtTop(animated: false)
            guard scroll.window != nil, !settling else { return }
            settling = true
            DispatchQueue.main.asyncAfter(deadline: .now() + 0.5) { [weak self] in
                guard let self, anchoring else { return }
                putTodayAtTop(animated: false)
                anchoring = false
            }
        }

        private var settling = false

        @objc fileprivate func userScrolled(_ notification: Notification) {
            anchoring = false
        }

        /// Put the top of Today's header (Overdue's, when it is
        /// there) at the top of the pane, under the toolbar: nothing
        /// of the day before shows. Rows have fixed heights, so the
        /// place is exact.
        private func putTodayAtTop(animated: Bool) {
            guard let row = anchorRow() else { return }
            scrollTo(y: table.rect(ofRow: row).minY - scroll.contentInsets.top, animated: animated)
        }

        private func scrollTo(y: CGFloat, animated: Bool) {
            let clip = scroll.contentView
            var bounds = clip.bounds
            bounds.origin.y = y
            // Not past the end: a table shorter than the pane from
            // Today on rests on its last row.
            let origin = clip.constrainBoundsRect(bounds).origin
            guard abs(origin.y - clip.bounds.origin.y) > 0.5 else { return }
            if animated {
                NSAnimationContext.runAnimationGroup({ context in
                    context.duration = 0.3
                    context.allowsImplicitAnimation = true
                    clip.animator().setBoundsOrigin(origin)
                }, completionHandler: { [weak self] in
                    guard let self else { return }
                    scroll.reflectScrolledClipView(scroll.contentView)
                })
            } else {
                clip.scroll(to: origin)
                scroll.reflectScrolledClipView(clip)
            }
        }

        /// The row at the top of the pane and how far it is scrolled
        /// past, to put it back after a reload: rows arriving above it
        /// (a sync into a past day) then do not move what is on screen.
        private func topRow() -> (key: String, past: CGFloat)? {
            let top = scroll.contentView.bounds.minY + scroll.contentInsets.top
            let row = table.row(at: NSPoint(x: 1, y: top + 1))
            guard row >= 0, row < lines.count else { return nil }
            return (lines[row].key, top - table.rect(ofRow: row).minY)
        }

        private func restore(_ held: (key: String, past: CGFloat)) {
            guard let row = lines.firstIndex(where: { $0.key == held.key }) else { return }
            scrollTo(y: table.rect(ofRow: row).minY + held.past - scroll.contentInsets.top, animated: false)
        }
    }

    /// A day's header: a group row. The label is not the cell's
    /// `textField`: the table restyles that one itself, and not for
    /// every header at once (new ones came in small, and a selection
    /// made the ones on screen larger). The font (the body's size and
    /// weight) and the colour are set here, once. A line runs under it,
    /// the rows' divider (`PaneColor.rowDivider`), across the cell.
    private final class HeaderCell: NSTableCellView {
        static let identifier = NSUserInterfaceItemIdentifier("AgendaHeader")

        let label = NSTextField(labelWithString: "")

        init() {
            super.init(frame: .zero)
            identifier = Self.identifier
            label.font = .systemFont(ofSize: NSFont.systemFontSize)
            label.textColor = .secondaryLabelColor
            label.lineBreakMode = .byTruncatingTail
            label.translatesAutoresizingMaskIntoConstraints = false
            addSubview(label)
            let divider = Divider()
            divider.translatesAutoresizingMaskIntoConstraints = false
            addSubview(divider)
            NSLayoutConstraint.activate([
                label.leadingAnchor.constraint(equalTo: leadingAnchor),
                label.trailingAnchor.constraint(lessThanOrEqualTo: trailingAnchor),
                label.bottomAnchor.constraint(equalTo: bottomAnchor, constant: -5),
                divider.leadingAnchor.constraint(equalTo: leadingAnchor),
                divider.trailingAnchor.constraint(equalTo: trailingAnchor),
                divider.bottomAnchor.constraint(equalTo: bottomAnchor),
                divider.heightAnchor.constraint(equalToConstant: 1),
            ])
        }

        @available(*, unavailable)
        required init?(coder: NSCoder) { fatalError("not built from a nib") }

        /// The line, drawn rather than a layer's colour so it follows
        /// the appearance (light, dark) on its own.
        private final class Divider: NSView {
            override func draw(_ dirtyRect: NSRect) {
                PaneColor.rowDivider.setFill()
                bounds.fill(using: .sourceOver)
            }
        }
    }

    /// A row that draws its own selection in the inset style's rounded
    /// shape, a pale blue in the key window and a grey in any other
    /// (`PaneColor.rowSelected`, `rowSelectedInactive`), and is never
    /// emphasized, so its cells are not told to draw as on the accent
    /// colour.
    private final class RowView: NSTableRowView {
        static let identifier = NSUserInterfaceItemIdentifier("AgendaRowView")
        /// The selection's margin either side, and its corners.
        static let inset: CGFloat = 10
        static let radius: CGFloat = 6

        /// The row's place in the day a dragged row would land on.
        struct Drop: Equatable {
            /// The section's first line (its header), and its last.
            let first: Bool
            let last: Bool
        }

        /// Set while a drag is over the row's day: the row has the
        /// day's tint (the web's `.upcoming-day[data-drop-target]`).
        var drop: Drop? {
            didSet { if drop != oldValue { needsDisplay = true } }
        }

        init() {
            super.init(frame: .zero)
            identifier = Self.identifier
        }

        @available(*, unavailable)
        required init?(coder: NSCoder) { fatalError("not built from a nib") }

        override var isEmphasized: Bool {
            get { false }
            set {}
        }

        override func drawSelection(in dirtyRect: NSRect) {
            let key = window?.isKeyWindow ?? false
            (key ? PaneColor.rowSelected : PaneColor.rowSelectedInactive).setFill()
            NSBezierPath(
                roundedRect: bounds.insetBy(dx: Self.inset, dy: 0), xRadius: Self.radius, yRadius: Self.radius
            ).fill()
        }

        /// The drop day's tint, under the selection: square where the
        /// row meets the next of its day, round at the section's ends.
        override func drawBackground(in dirtyRect: NSRect) {
            super.drawBackground(in: dirtyRect)
            guard let drop else { return }
            let rect = bounds.insetBy(dx: Self.inset, dy: 0)
            let r = min(Self.radius, rect.height / 2)
            // `top` is the radius at `minY`: the row's top edge in a
            // flipped view, its bottom edge otherwise.
            let top = (isFlipped ? drop.first : drop.last) ? r : 0
            let bottom = (isFlipped ? drop.last : drop.first) ? r : 0
            let path = NSBezierPath()
            path.move(to: NSPoint(x: rect.minX + top, y: rect.minY))
            path.line(to: NSPoint(x: rect.maxX - top, y: rect.minY))
            path.appendArc(
                from: NSPoint(x: rect.maxX, y: rect.minY), to: NSPoint(x: rect.maxX, y: rect.minY + top), radius: top)
            path.line(to: NSPoint(x: rect.maxX, y: rect.maxY - bottom))
            path.appendArc(
                from: NSPoint(x: rect.maxX, y: rect.maxY), to: NSPoint(x: rect.maxX - bottom, y: rect.maxY),
                radius: bottom)
            path.line(to: NSPoint(x: rect.minX + bottom, y: rect.maxY))
            path.appendArc(
                from: NSPoint(x: rect.minX, y: rect.maxY), to: NSPoint(x: rect.minX, y: rect.maxY - bottom),
                radius: bottom)
            path.line(to: NSPoint(x: rect.minX, y: rect.minY + top))
            path.appendArc(
                from: NSPoint(x: rect.minX, y: rect.minY), to: NSPoint(x: rect.minX + top, y: rect.minY), radius: top)
            path.close()
            PaneColor.dropDay.setFill()
            path.fill()
        }

        /// The selection is drawn again when the window gains or
        /// loses key.
        override func viewDidMoveToWindow() {
            super.viewDidMoveToWindow()
            let center = NotificationCenter.default
            center.removeObserver(self)
            guard let window else { return }
            for name in [NSWindow.didBecomeKeyNotification, NSWindow.didResignKeyNotification] {
                center.addObserver(self, selector: #selector(keyChanged(_:)), name: name, object: window)
            }
        }

        @objc private func keyChanged(_ notification: Notification) {
            if isSelected { needsDisplay = true }
        }

        deinit { NotificationCenter.default.removeObserver(self) }
    }

    /// A row's SwiftUI content, hosted. The selection's ground is a
    /// soft grey (`RowView`), so the content is drawn the same on it.
    private final class HostCell: NSTableCellView {
        static let identifier = NSUserInterfaceItemIdentifier("AgendaRow")

        private let host = NSHostingView(rootView: AnyView(EmptyView()))
        private var content = AnyView(EmptyView())
        private var divided = false

        init() {
            super.init(frame: .zero)
            identifier = Self.identifier
            // The cell sizes the host, not the other way round.
            host.sizingOptions = []
            host.frame = bounds
            host.autoresizingMask = [.width, .height]
            addSubview(host)
        }

        @available(*, unavailable)
        required init?(coder: NSCoder) { fatalError("not built from a nib") }

        func show(_ content: AnyView, divided: Bool) {
            self.content = content
            self.divided = divided
            draw()
        }

        private func draw() {
            let divided = divided
            host.rootView = AnyView(content
                .frame(maxWidth: .infinity, maxHeight: .infinity)
                .overlay(alignment: .bottom) {
                    if divided {
                        Rectangle()
                            .fill(Color(nsColor: PaneColor.rowDivider))
                            .frame(height: 1)
                    }
                })
        }
    }
}
