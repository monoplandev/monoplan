import AppKit
import MonoplanCore
import SwiftUI
import XCTest
@testable import MonoplanMac

/// The Calendar table's drag to reschedule (`AgendaTable.swift`), driven
/// with mouse events in an off-screen window: the press is given to the
/// view under it (a row's hosting view, which hands it up to the table),
/// and what follows it is queued first, since the table reads the rest
/// of a press itself. The rows are hosted SwiftUI, as the app's are.
@MainActor
final class AgendaDragTests: XCTestCase {
    private func entry(_ id: String, when: String) -> AgendaEntry {
        AgendaEntry(
            id: id, text: id, listId: "inbox", listName: nil, listIcon: nil, state: .todo, event: false,
            when: when, duration: nil, place: nil, assignee: nil,
            tone: .neutral, docId: nil, docName: nil)
    }

    /// Today with rows a and b, tomorrow with c, then empty days:
    /// lines 0 header, 1 a, 2 b, 3 header, 4 c, 5 header (the 25th),
    /// 6 its empty line.
    private final class Rig {
        var dropped: [String] = []
        var selected: [String?] = []
        var pressed = 0
        let window: NSWindow
        let table: NSTableView

        init(groups: [AgendaGroup]) {
            _ = NSApplication.shared
            window = NSWindow(
                contentRect: NSRect(x: 0, y: 0, width: 400, height: 600), styleMask: [.titled],
                backing: .buffered, defer: false)
            window.isReleasedWhenClosed = false
            var rig: Rig?
            let view = AgendaTable(
                groups: groups, selected: nil, todayJumps: 0, fillEmptyDays: true,
                heading: { $0.day }, rowHeight: { _ in 24 },
                row: { row in
                    AnyView(HStack {
                        Button("tick") { rig?.pressed += 1 }
                        Text(verbatim: row.text)
                        Spacer()
                    })
                },
                select: { rig?.selected.append($0?.id) },
                menu: { _ in [] },
                reschedule: {
                    rig?.dropped.append("\($0.id) -> \($1)")
                    return true
                })
            let host = NSHostingView(rootView: view)
            window.contentView = host
            host.layoutSubtreeIfNeeded()
            table = Rig.find(NSTableView.self, in: host)!
            table.layoutSubtreeIfNeeded()
            rig = self
        }

        static func find<T: NSView>(_ type: T.Type, in view: NSView) -> T? {
            if let view = view as? T { return view }
            for sub in view.subviews {
                if let found = find(type, in: sub) { return found }
            }
            return nil
        }

        /// A point of line `row`, in the window's coordinates: its
        /// middle, `x` in from the row's left edge.
        func point(row: Int, x: CGFloat = 200) -> NSPoint {
            let rect = table.rect(ofRow: row)
            return table.convert(NSPoint(x: rect.minX + x, y: rect.midY), to: nil)
        }

        func mouse(_ type: NSEvent.EventType, at point: NSPoint) -> NSEvent {
            NSEvent.mouseEvent(
                with: type, location: point, modifierFlags: [], timestamp: ProcessInfo.processInfo.systemUptime,
                windowNumber: window.windowNumber, context: nil, eventNumber: 0, clickCount: 1, pressure: 1)!
        }

        func escape() -> NSEvent {
            NSEvent.keyEvent(
                with: .keyDown, location: .zero, modifierFlags: [], timestamp: ProcessInfo.processInfo.systemUptime,
                windowNumber: window.windowNumber, context: nil, characters: "\u{1b}",
                charactersIgnoringModifiers: "\u{1b}", isARepeat: false, keyCode: 53)!
        }

        /// Press at `start` with `rest` already waiting to be read.
        func press(at start: NSPoint, then rest: [NSEvent]) {
            for event in rest { NSApp.postEvent(event, atStart: false) }
            window.contentView?.hitTest(start)?.mouseDown(with: mouse(.leftMouseDown, at: start))
        }
    }

    private var groups: [AgendaGroup] {
        [
            AgendaGroup(day: "2026-11-23", today: true, rows: [
                entry("a", when: "2026-11-23"), entry("b", when: "2026-11-23T09:00"),
            ]),
            AgendaGroup(day: "2026-11-24", today: false, rows: [entry("c", when: "2026-11-24")]),
        ]
    }

    func testARowLetGoOnAnotherDayIsRescheduledThere() {
        let rig = Rig(groups: groups)
        let from = rig.point(row: 1)
        let to = rig.point(row: 6)
        rig.press(at: from, then: [
            rig.mouse(.leftMouseDragged, at: NSPoint(x: from.x, y: from.y - 10)),
            rig.mouse(.leftMouseDragged, at: to),
            rig.mouse(.leftMouseUp, at: to),
        ])
        XCTAssertEqual(rig.selected, ["a"])
        XCTAssertEqual(rig.dropped, ["a -> 2026-11-25"])
        // Until its write comes back as rows, the row is not shown
        // where it was.
        XCTAssertEqual(rig.table.rowView(atRow: 1, makeIfNecessary: false)?.alphaValue, 0)
        XCTAssertEqual(rig.table.rowView(atRow: 2, makeIfNecessary: false)?.alphaValue, 1)
    }

    /// New rows are put in by taking out the lines that left and
    /// putting in the ones that came (`change(to:)`), not by a reload:
    /// the table ends with the new lines, and the selection on its item.
    func testAMovedRowChangesTheTableInPlace() {
        _ = NSApplication.shared
        func table(_ groups: [AgendaGroup]) -> AgendaTable {
            AgendaTable(
                groups: groups, selected: "a", todayJumps: 0, fillEmptyDays: true,
                heading: { $0.day }, rowHeight: { _ in 24 },
                row: { AnyView(Text(verbatim: $0.text)) }, select: { _ in }, menu: { _ in [] })
        }
        let moved = [
            AgendaGroup(day: "2026-11-23", today: true, rows: [entry("b", when: "2026-11-23T09:00")]),
            AgendaGroup(day: "2026-11-24", today: false, rows: [entry("c", when: "2026-11-24")]),
            AgendaGroup(day: "2026-11-26", today: false, rows: [entry("a", when: "2026-11-26")]),
        ]
        let window = NSWindow(
            contentRect: NSRect(x: 0, y: 0, width: 400, height: 600), styleMask: [.titled],
            backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false
        let host = NSHostingView(rootView: table(groups))
        window.contentView = host
        host.layoutSubtreeIfNeeded()
        let view = Rig.find(NSTableView.self, in: host)!
        let rows = view.numberOfRows
        XCTAssertEqual(view.selectedRow, 1)

        // a leaves today and takes the 26th's empty line's place:
        // header, b, header, c, header, empty, header, a.
        host.rootView = table(moved)
        host.layoutSubtreeIfNeeded()
        XCTAssertEqual(view.numberOfRows, rows - 1)
        XCTAssertEqual(view.selectedRow, 7)
        XCTAssertEqual(view.rect(ofRow: 7).minY, view.rect(ofRow: 6).maxY)

        host.rootView = table(groups)
        host.layoutSubtreeIfNeeded()
        XCTAssertEqual(view.numberOfRows, rows)
        XCTAssertEqual(view.selectedRow, 1)
    }

    func testADaysHeaderTakesTheDropToo() {
        let rig = Rig(groups: groups)
        let from = rig.point(row: 2)
        let to = rig.point(row: 3)
        rig.press(at: from, then: [
            rig.mouse(.leftMouseDragged, at: to),
            rig.mouse(.leftMouseUp, at: to),
        ])
        XCTAssertEqual(rig.dropped, ["b -> 2026-11-24"])
    }

    func testAPressThatDoesNotTravelIsAClick() {
        let rig = Rig(groups: groups)
        let from = rig.point(row: 4)
        rig.press(at: from, then: [
            rig.mouse(.leftMouseDragged, at: NSPoint(x: from.x + 2, y: from.y + 2)),
            rig.mouse(.leftMouseUp, at: rig.point(row: 6)),
        ])
        XCTAssertEqual(rig.selected, ["c"])
        XCTAssertEqual(rig.dropped, [])
    }

    func testEscapePutsTheRowBack() {
        let rig = Rig(groups: groups)
        let from = rig.point(row: 1)
        let to = rig.point(row: 6)
        rig.press(at: from, then: [
            rig.mouse(.leftMouseDragged, at: to),
            rig.escape(),
            rig.mouse(.leftMouseDragged, at: to),
            rig.mouse(.leftMouseUp, at: to),
        ])
        XCTAssertEqual(rig.dropped, [])
    }

    func testOffThePaneNothingIsWritten() {
        let rig = Rig(groups: groups)
        let from = rig.point(row: 1)
        let off = NSPoint(x: -50, y: from.y)
        rig.press(at: from, then: [
            rig.mouse(.leftMouseDragged, at: off),
            rig.mouse(.leftMouseUp, at: off),
        ])
        XCTAssertEqual(rig.dropped, [])
    }
}
