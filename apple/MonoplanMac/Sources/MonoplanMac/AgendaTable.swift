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
// rewritten in place; a day's header is a group row. No drag, no
// multiple selection: a click selects an item, which opens its panel,
// as in the SwiftUI lists. A right-click anywhere on an item's row
// selects it too and opens its menu: the menu is the table's, not the
// hosted content's, which would only take a click on what it draws.
// The selected row has a soft ground, not
// the accent colour: the web's, a pale blue in the key window and a
// grey in any other (`PaneColor.rowSelected`, `RowView`).

import AppKit
import MonoplanCore
import SwiftUI

struct AgendaTable: NSViewRepresentable {
    let groups: [AgendaGroup]
    /// The selected item's id, if one is.
    let selected: String?
    /// The model's count of "Today" presses: a new value scrolls back
    /// to Today, or to Overdue above it when something is.
    let todayJumps: Int
    /// Every day from Today on gets a header (`filled`); false is only
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
    }

    /// How many days past Today are shown when nothing is on them, if
    /// nothing is placed later: two weeks of days in all, the web's
    /// `AGENDA_ALL_DAYS_AHEAD`.
    static let daysAhead = 13

    /// The groups with every day from Today on: an empty group for
    /// each day missing from Today through the later of the last day
    /// with rows and `daysAhead` past Today. Days gone by are kept only
    /// when they have rows, as before. Without a Today group (no read
    /// yet) the groups are the non-empty ones as they are.
    static func filled(_ groups: [AgendaGroup], ahead: Int = daysAhead) -> [AgendaGroup] {
        var out: [AgendaGroup] = []
        var byDay: [String: AgendaGroup] = [:]
        var today: String?
        for group in groups {
            if group.today { today = group.day }
            if let today, group.day >= today {
                byDay[group.day] = byDay[group.day] ?? group
            } else if group.today || !group.rows.isEmpty {
                out.append(group)
            }
        }
        guard let today else { return out }
        var last = addDays(today, ahead)
        if let latest = byDay.filter({ !$0.value.rows.isEmpty }).keys.max(), latest > last {
            last = latest
        }
        var day = today
        while day <= last {
            out.append(byDay[day] ?? AgendaGroup(day: day, today: day == today, rows: []))
            day = addDays(day, 1)
        }
        return out
    }

    /// `stamp` (`YYYY-MM-DD`) moved by `days`, on the Gregorian
    /// calendar in UTC, so no clock change skips or repeats a day.
    static func addDays(_ stamp: String, _ days: Int) -> String {
        var calendar = Calendar(identifier: .gregorian)
        calendar.timeZone = TimeZone(identifier: "UTC")!
        let parts = stamp.split(separator: "-").compactMap { Int($0) }
        guard parts.count == 3,
              let date = calendar.date(from: DateComponents(year: parts[0], month: parts[1], day: parts[2])),
              let moved = calendar.date(byAdding: .day, value: days, to: date)
        else { return stamp }
        let c = calendar.dateComponents([.year, .month, .day], from: moved)
        return String(format: "%04d-%02d-%02d", c.year!, c.month!, c.day!)
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
        /// (`filled`), any other day when it has rows, and Overdue
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
                ? AgendaTable.filled(parent.groups)
                : parent.groups.filter { $0.today || !$0.rows.isEmpty }
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
                self.lines = lines
                table.reloadData()
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
            NSBezierPath(roundedRect: bounds.insetBy(dx: 10, dy: 0), xRadius: 6, yRadius: 6).fill()
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
