import XCTest
import MonoplanCore
@testable import MonoplanMac

final class TrayMenuTests: XCTestCase {
    func entry(
        _ id: String, _ text: String, list: String = "inbox", state: ItemState = .backlog,
        event: Bool = false, when: String? = nil, duration: UInt32? = nil, deadline: String? = nil,
        tone: AgendaTone = .neutral, docId: String? = nil, placedBy: AgendaPlacedBy? = nil
    ) -> AgendaEntry {
        AgendaEntry(
            id: id, text: text, listId: list, listName: nil, listIcon: nil, state: state, event: event,
            when: when,
            duration: duration,
            deadline: deadline, place: nil, assignee: nil, placedBy: placedBy ?? (when == nil ? .deadline : .when),
            tone: tone,
            docId: docId, docName: nil)
    }

    func testAnEventHasNoCheckboxWhileOpenOrOnceItsDayHasGoneBy() {
        let today = "2026-10-26"
        let shows = { MainWindowView.agendaRowShowsCheck($0, today: today) }
        XCTAssertFalse(shows(entry("a", "Gig", event: true, when: "2026-10-26T20:00")))
        XCTAssertTrue(shows(entry("b", "Gig", state: .done, event: true, when: "2026-10-26T20:00")))
        XCTAssertFalse(shows(entry("c", "Gig", state: .done, event: true, when: "2026-10-20T20:00")))
        XCTAssertTrue(shows(entry("d", "Task", when: "2026-10-26")))
        XCTAssertTrue(shows(entry("e", "Tickets", event: true, when: "2026-10-30", deadline: "2026-10-26", placedBy: .deadline)))
    }

    /// What `Engine.agenda(days: 0)` returns on a day with all five
    /// kinds of row, plus a group a tray does not show.
    var sample: [AgendaGroup] {
        [
            AgendaGroup(day: "2026-10-26", today: true, rows: [
                entry("a", "Overdue report", list: "work", state: .todo, deadline: "2026-10-20", tone: .overdue),
                entry("b", "Standup", event: true, when: "2026-10-26T09:00", duration: 15),
                entry("c", "Dentist", list: "errands", event: true, when: "2026-10-26T14:30", duration: 60, docId: "doc-2"),
                entry("d", "Groceries", state: .done, when: "2026-10-26"),
                entry("e", "Pay rent", state: .todo, deadline: "2026-10-26", tone: .warning),
            ]),
            AgendaGroup(day: "2026-10-27", today: false, rows: [
                entry("u", "Tomorrow's thing", when: "2026-10-27"),
            ]),
        ]
    }

    func date(_ h: Int, _ m: Int) -> Date {
        Calendar.current.date(from: DateComponents(year: 2026, month: 10, day: 26, hour: h, minute: m))!
    }

    func testOnlyTodayBecomesLinesWithTimesAndNotes() {
        let lines = TodayMenu.lines(from: sample, now: date(9, 5))
        XCTAssertEqual(lines.map(\.title), [
            "Overdue report", "09:00  Standup", "14:30  Dentist", "Groceries", "Pay rent",
        ])
        // An overdue row's note is the day it was due.
        let due = Calendar.current.date(from: DateComponents(year: 2026, month: 10, day: 20))!
        XCTAssertEqual(lines.map(\.note), [
            due.formatted(.dateTime.day().month(.abbreviated)), "now", "in 5h 25m", nil, "due today",
        ])
        XCTAssertEqual(lines.map(\.done), [false, false, false, true, false])
        XCTAssertEqual(lines.map(\.overdue), [true, false, false, false, false])
    }

    func testLinesCarryWhereTheItemLives() {
        let lines = TodayMenu.lines(from: sample, now: date(9, 5))
        XCTAssertEqual(lines.map(\.listId), ["work", "inbox", "errands", "inbox", "inbox"])
        XCTAssertEqual(lines.map(\.docId), [nil, nil, "doc-2", nil, nil])
    }

    func testTimedNotesFollowTheClock() {
        XCTAssertEqual(TodayMenu.lines(from: sample, now: date(8, 59))[1].note, "in 1m")
        XCTAssertEqual(TodayMenu.lines(from: sample, now: date(9, 20))[1].note, "started 20m ago")
        XCTAssertEqual(TodayMenu.lines(from: sample, now: date(15, 0))[2].note, "now")
    }

    func testNoTodayGroupIsNoLines() {
        XCTAssertTrue(TodayMenu.lines(from: [], now: date(9, 0)).isEmpty)
    }

    func testSpanFormatting() {
        XCTAssertEqual(TodayMenu.span(30), "<1m")
        XCTAssertEqual(TodayMenu.span(3 * 60), "3m")
        XCTAssertEqual(TodayMenu.span(2 * 3600 + 15 * 60), "2h 15m")
        XCTAssertEqual(TodayMenu.span(3 * 3600), "3h")
        XCTAssertEqual(TodayMenu.span(28 * 3600), "1d 4h")
    }

    func testLongTitlesAreClippedToThirtyTwoWithEllipsis() {
        let exact = String(repeating: "a", count: 32)
        XCTAssertEqual(TodayMenu.clipped(exact), exact)
        let long = String(repeating: "b", count: 33)
        let clipped = TodayMenu.clipped(long)
        XCTAssertEqual(clipped, String(repeating: "b", count: 31) + "…")
        // The time prefix is outside the cap.
        let group = AgendaGroup(day: "2026-10-26", today: true, rows: [
            entry("x", long, when: "2026-10-26T09:00"),
        ])
        XCTAssertEqual(TodayMenu.lines(from: [group], now: date(8, 0))[0].title, "09:00  " + clipped)
    }

    func starred(_ pos: UInt32, _ id: String, _ text: String, list: String, listName: String?, docId: String? = nil) -> StarredItem {
        StarredItem(
            pos: pos, id: id, text: text, listId: list, state: .todo, place: nil,
            docId: docId, docName: nil, listName: listName)
    }

    func testStarredShowsTheFirstThreeClippedWithTheirLists() {
        let rows = [
            starred(1, "a", "Ship the tray", list: "work", listName: "Work", docId: "doc-2"),
            starred(2, "b", "Book flights", list: "inbox", listName: "Inbox"),
            starred(3, "c", String(repeating: "c", count: 40), list: "gone", listName: nil),
            starred(4, "d", "Fourth", list: "inbox", listName: "Inbox"),
        ]
        let lines = StarredMenu.lines(from: rows)
        XCTAssertEqual(lines.map(\.id), ["a", "b", "c"])
        XCTAssertEqual(lines[2].title, String(repeating: "c", count: 31) + "…")
        // A list the engine could not name shows nothing, not an id.
        XCTAssertEqual(lines.map(\.note), ["Work", "Inbox", nil])
        XCTAssertEqual(lines.map(\.listId), ["work", "inbox", "gone"])
        XCTAssertEqual(lines.map(\.docId), ["doc-2", nil, nil])
        XCTAssertTrue(StarredMenu.lines(from: []).isEmpty)
    }

    func sync(online: Bool, lastSyncedAt: Int64? = nil, reason: String? = nil, localOnly: Bool = false, pending: Bool = false) -> SyncState {
        SyncState(online: online, lastSyncedAt: lastSyncedAt, offlineReason: reason, localOnly: localOnly, pending: pending)
    }

    func testSyncLines() {
        let now = Date(timeIntervalSince1970: 1_000_000)
        let threeMinutesAgo: Int64 = (1_000_000 - 180) * 1000
        XCTAssertEqual(sync(online: true, lastSyncedAt: threeMinutesAgo).line(now: now), "Synced · 3m ago")
        XCTAssertEqual(sync(online: true, pending: true).line(now: now), "Syncing…")
        XCTAssertEqual(sync(online: false, localOnly: true).line(now: now), "Local workspace · not synced")
        XCTAssertNil(sync(online: false, localOnly: true).offlineNotice(now: now))
        XCTAssertNil(sync(online: true).offlineNotice(now: now))
        XCTAssertEqual(sync(online: false).offlineNotice(now: now), "Offline · never synced")
        XCTAssertEqual(
            sync(online: false, lastSyncedAt: threeMinutesAgo, reason: "connect failed").line(now: now),
            "Offline · last synced 3m ago · connect failed")
    }
}

final class AgendaFillTests: XCTestCase {
    private func entry(_ id: String) -> AgendaEntry {
        AgendaEntry(
            id: id, text: id, listId: "inbox", listName: nil, listIcon: nil, state: .backlog, event: false,
            when: nil, duration: nil, deadline: nil, place: nil, assignee: nil, placedBy: .deadline,
            tone: .neutral, docId: nil, docName: nil)
    }

    func testFillsTwoWeeksFromTodayKeepingPastNonEmpty() {
        let groups = [
            AgendaGroup(day: "2026-11-20", today: false, rows: [entry("a")]),
            AgendaGroup(day: "2026-11-23", today: true, rows: []),
            AgendaGroup(day: "2026-11-25", today: false, rows: [entry("b")]),
        ]
        let out = AgendaTable.filled(groups)
        XCTAssertEqual(out.first?.day, "2026-11-20")
        XCTAssertEqual(out.dropFirst().map(\.day).first, "2026-11-23")
        XCTAssertEqual(out.count, 1 + 14)
        XCTAssertEqual(out.last?.day, "2026-12-06")
        XCTAssertEqual(out.first { $0.day == "2026-11-25" }?.rows.count, 1)
        XCTAssertEqual(out.filter(\.today).count, 1)
    }

    func testReachesTheLastDayWithRows() {
        let groups = [
            AgendaGroup(day: "2026-11-23", today: true, rows: []),
            AgendaGroup(day: "2027-01-02", today: false, rows: [entry("c")]),
        ]
        let out = AgendaTable.filled(groups)
        XCTAssertEqual(out.last?.day, "2027-01-02")
        XCTAssertEqual(out.count, 41)
    }
}

final class SurfaceStoreTests: XCTestCase {
    func testRoundTrips() throws {
        let defaults = try XCTUnwrap(UserDefaults(suiteName: "SurfaceStoreTests"))
        defer { defaults.removePersistentDomain(forName: "SurfaceStoreTests") }
        XCTAssertNil(SurfaceStore.load(from: defaults))
        let list = Surface(lens: nil, workspace: "doc1", list: "abc")
        SurfaceStore.save(list, to: defaults)
        XCTAssertEqual(SurfaceStore.load(from: defaults), list)
        let lens = Surface(lens: .agenda, workspace: nil, list: "inbox")
        SurfaceStore.save(lens, to: defaults)
        XCTAssertEqual(SurfaceStore.load(from: defaults), lens)
    }
}
