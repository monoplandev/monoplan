import XCTest
@testable import MonoplanMacKit

final class AgendaSnapshotTests: XCTestCase {
    let sample = """
    [{"day":"2026-10-26","today":true,"rows":[
      {"id":"a","text":"Overdue report","list_id":"work","state":"todo","deadline":"2026-10-20","placed_by":"deadline","tone":"overdue"},
      {"id":"b","text":"Standup","list_id":"events","state":"backlog","when":"2026-10-26T09:00","duration":15,"placed_by":"when","tone":"neutral"},
      {"id":"d","text":"Groceries","list_id":"inbox","state":"done","when":"2026-10-26","placed_by":"when","tone":"neutral"}
    ]},
    {"day":null,"today":false,"rows":[{"id":"u","text":"Someday","list_id":"events","state":"backlog","tone":"neutral"}]},
    {"day":"2026-10-27","today":false,"rows":[]},
    {"day":"2026-10-28","today":false,"rows":[
      {"id":"x","text":"Dentist","list_id":"events","state":"backlog","when":"2026-10-28T14:30","placed_by":"when","tone":"neutral"},
      {"id":"y","text":"Cancelled thing","list_id":"events","state":"cancelled","when":"2026-10-28","placed_by":"when","tone":"neutral"}
    ]}]
    """

    func date(_ day: Int) -> Date {
        Calendar.current.date(from: DateComponents(year: 2026, month: 10, day: day, hour: 8))!
    }

    func ids(_ lines: [WidgetLine]) -> [String] { lines.map(\.id) }

    func testTodayThenLaterDaysOpenRowsOnly() throws {
        let days = try Agenda.parse(sample)
        let out = WidgetAgenda.lines(from: days, on: date(26), limit: 10)
        XCTAssertEqual(ids(out.lines), ["/a", "/b", "day/2026-10-28", "2026-10-28//x"])
        XCTAssertEqual(out.todayCount, 2)
        guard case .row(let standup) = out.lines[1] else { return XCTFail("not a row") }
        XCTAssertEqual(standup.time, "09:00")
    }

    func testLimitNeverEndsOnAHeading() throws {
        let days = try Agenda.parse(sample)
        XCTAssertEqual(ids(WidgetAgenda.lines(from: days, on: date(26), limit: 3).lines), ["/a", "/b"])
        let one = WidgetAgenda.lines(from: days, on: date(26), limit: 1)
        XCTAssertEqual(ids(one.lines), ["/a"])
        XCTAssertEqual(one.todayCount, 2)
    }

    func testAStaleSnapshotFoldsPassedDaysIntoToday() throws {
        let days = try Agenda.parse(sample)
        // Two days on, the app not opened since: the 28th is today, and
        // what was on the 26th is still to do.
        let out = WidgetAgenda.lines(from: days, on: date(28), limit: 10)
        XCTAssertEqual(ids(out.lines), ["/a", "/b", "/x"])
        XCTAssertEqual(out.todayCount, 3)
    }

    func testWriteReadClear() throws {
        let url = FileManager.default.temporaryDirectory.appendingPathComponent("agenda-\(UUID().uuidString).json")
        XCTAssertNil(AgendaSnapshot.read(from: url))
        XCTAssertTrue(try AgendaSnapshot.write(sample, to: url))
        XCTAssertFalse(try AgendaSnapshot.write(sample, to: url))
        XCTAssertEqual(AgendaSnapshot.read(from: url)?.count, 4)
        XCTAssertTrue(AgendaSnapshot.clear(at: url))
        XCTAssertFalse(AgendaSnapshot.clear(at: url))
    }
}
