import XCTest
import MonoplanCore
@testable import MonoplanMac

final class TrayMenuTests: XCTestCase {
    func entry(
        _ id: String, _ text: String, list: String = "inbox", state: ItemState = .backlog,
        event: Bool = false, when: String? = nil, duration: UInt32? = nil,
        tone: AgendaTone = .neutral, docId: String? = nil
    ) -> AgendaEntry {
        AgendaEntry(
            id: id, text: text, listId: list, listName: nil, listIcon: nil, state: state, event: event,
            when: when,
            duration: duration,
            place: nil, assignee: nil,
            tone: tone,
            docId: docId, docName: nil,
            day: String(when?.prefix(10) ?? ""), past: false, showsCheck: !event, spanLabel: nil, lane: .empty)
    }

    /// What `Engine.agenda(days: 0)` returns on a day with an
    /// overdue task, two timed events, a done task and an untimed task,
    /// plus a group a tray does not show.
    var sample: [AgendaGroup] {
        [
            AgendaGroup(day: "2026-10-26", today: true, rows: [
                entry("a", "Overdue report", list: "work", state: .todo, when: "2026-10-20", tone: .overdue),
                entry("b", "Standup", event: true, when: "2026-10-26T09:00", duration: 15),
                entry("c", "Dentist", list: "errands", event: true, when: "2026-10-26T14:30", duration: 60, docId: "doc-2"),
                entry("d", "Groceries", state: .done, when: "2026-10-26"),
                entry("e", "Pay rent", state: .todo, when: "2026-10-26"),
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
        // An overdue row's note is the day it was planned for.
        let due = Calendar.current.date(from: DateComponents(year: 2026, month: 10, day: 20))!
        XCTAssertEqual(lines.map(\.note), [
            due.formatted(.dateTime.day().month(.abbreviated)), "now", "in 5h 25m", nil, nil,
        ])
        XCTAssertEqual(lines.map(\.done), [false, false, false, true, false])
        XCTAssertEqual(lines.map(\.overdue), [true, false, false, false, false])
        // The two events lead with the pill.
        XCTAssertEqual(lines.map(\.event), [false, true, true, false, false])
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

    func testMenuBarNamesTheEventUnderWayOrAboutToStart() {
        // Standup 09:00–09:15, Dentist 14:30–15:30; tasks never show.
        XCTAssertEqual(NowEvent.shown(from: sample, now: date(8, 30)), .init(title: "", next: date(8, 55)))
        let soon = Calendar.current.date(byAdding: .second, value: 30, to: date(8, 56))!
        XCTAssertEqual(
            NowEvent.shown(from: sample, now: soon), .init(title: "Standup · in 3m", next: date(8, 57)))
        XCTAssertEqual(NowEvent.shown(from: sample, now: date(9, 0)), .init(title: "Standup", next: date(9, 15)))
        XCTAssertEqual(NowEvent.shown(from: sample, now: date(9, 15)), .init(title: "", next: date(14, 25)))
        XCTAssertEqual(NowEvent.shown(from: sample, now: date(15, 0)), .init(title: "Dentist", next: date(15, 30)))
        XCTAssertEqual(NowEvent.shown(from: sample, now: date(16, 0)), .init(title: "", next: nil))
        // What is next wins over what is under way.
        let overlap = [AgendaGroup(day: "2026-10-26", today: true, rows: [
            entry("l", "Long workshop on something or other", event: true, when: "2026-10-26T08:00", duration: 240),
            entry("s", "Call", event: true, when: "2026-10-26T10:00"),
        ])]
        XCTAssertEqual(NowEvent.shown(from: overlap, now: date(9, 0)).title, "Long workshop on someth…")
        XCTAssertEqual(NowEvent.shown(from: overlap, now: date(9, 58)).title, "Call · in 2m")
        // No duration: named until its start, never under way.
        XCTAssertEqual(NowEvent.shown(from: overlap, now: date(10, 0)).title, "Long workshop on someth…")
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

    func inboxEntry(_ id: String, _ text: String, docId: String? = nil) -> InboxEntry {
        InboxEntry(
            item: Item(
                id: id, text: text, listId: "inbox", state: .backlog, lifecycleAt: 0, createdAt: 0,
                createdBy: nil, assignee: nil, startedAt: nil, doneAt: nil, binnedAt: nil, when: nil,
                whenSuggested: nil, event: false, duration: nil, place: nil),
            docId: docId)
    }

    func testInboxShowsTheHeadOfTheQueueInTheDeviceOrder() {
        let rows = [
            inboxEntry("a", "Oldest", docId: "doc-2"),
            inboxEntry("b", String(repeating: "b", count: 40)),
            inboxEntry("c", "Newest"),
        ]
        let lines = InboxMenu.lines(from: rows, newestFirst: false, limit: 2)
        XCTAssertEqual(lines.map(\.id), ["a", "b"])
        XCTAssertEqual(lines[1].title, String(repeating: "b", count: 31) + "…")
        XCTAssertEqual(lines.map(\.listId), ["inbox", "inbox"])
        XCTAssertEqual(lines.map(\.docId), ["doc-2", nil])
        XCTAssertEqual(InboxMenu.lines(from: rows, newestFirst: true, limit: 2).map(\.id), ["c", "b"])
        XCTAssertTrue(InboxMenu.lines(from: [], newestFirst: false).isEmpty)
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
