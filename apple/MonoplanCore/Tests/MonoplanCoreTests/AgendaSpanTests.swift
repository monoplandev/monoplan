import MonoplanCore
import XCTest

/// `AgendaSpan.swift`: a row's day, its "Day N" and its time lane, read
/// off the row as the web's `Upcoming.tsx` reads a `DayRow`.
final class AgendaSpanTests: XCTestCase {
    private func row(
        when: String, duration: UInt32? = nil, event: Bool = false, span: (UInt32, UInt32)? = nil,
        tone: AgendaTone = .neutral
    ) -> AgendaEntry {
        AgendaEntry(
            id: "a", text: "a", listId: "inbox", listName: nil, listIcon: nil, state: .todo, event: event,
            when: when, duration: duration, place: nil, assignee: nil, tone: tone, docId: nil, docName: nil,
            span: span.map { AgendaSpan(day: $0.0, total: $0.1) })
    }

    func testRowDayFollowsTheSpan() {
        XCTAssertEqual(row(when: "2026-11-30").rowDay, "2026-11-30")
        XCTAssertEqual(row(when: "2026-11-30", duration: 3 * 1440, span: (3, 3)).rowDay, "2026-12-02")
        XCTAssertEqual(row(when: "2026-11-30T22:00", duration: 120, span: (2, 2)).rowDay, "2026-12-01")
    }

    func testDayLabel() {
        XCTAssertEqual(row(when: "2026-11-30", duration: 3 * 1440, span: (1, 3)).spanDayLabel, 1)
        XCTAssertEqual(row(when: "2026-11-30T09:00", duration: 26 * 60, span: (2, 2)).spanDayLabel, 2)
        // An evening past midnight is one occasion.
        XCTAssertNil(row(when: "2026-11-30T22:00", duration: 120, span: (2, 2)).spanDayLabel)
        XCTAssertNil(row(when: "2026-11-30").spanDayLabel)
    }

    func testLane() {
        XCTAssertEqual(row(when: "2026-11-30").lane, .none)
        XCTAssertEqual(row(when: "2026-11-30", event: true).lane, .allDay)
        XCTAssertEqual(row(when: "2026-11-30T09:15").lane, .start("09:15"))
        // A timed span: start, then all day, then until its end.
        let timed = { (day: UInt32) in
            self.row(when: "2026-11-30T22:00", duration: 2 * 1440 + 180, event: true, span: (day, 3))
        }
        XCTAssertEqual(timed(1).lane, .start("22:00"))
        XCTAssertEqual(timed(2).lane, .allDay)
        XCTAssertEqual(timed(3).lane, .until("01:00"))
        // A task's lane is empty where an event's says "All day".
        XCTAssertEqual(row(when: "2026-11-30T22:00", duration: 2 * 1440 + 180, span: (2, 3)).lane, .none)
    }

    func testUpcomingDropsTheDaysGoneBy() {
        let groups = [
            AgendaGroup(day: "2026-11-20", today: false, rows: []),
            AgendaGroup(day: "2026-11-25", today: true, rows: []),
            AgendaGroup(day: "2026-11-26", today: false, rows: []),
        ]
        XCTAssertEqual(AgendaGroup.upcoming(groups).map(\.day), ["2026-11-25", "2026-11-26"])
    }
}
