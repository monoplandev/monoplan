import XCTest
@testable import MonoplanMacKit

final class AgendaTests: XCTestCase {
    // What `monoplan agenda --json --days 0` prints, after the
    // `RowJson` shape in cli/src/commands/agenda.rs.
    let sample = """
    [{"day":"2026-10-26","today":true,"rows":[
      {"id":"a","text":"Overdue report","list_id":"work","state":"todo","deadline":"2026-10-20","placed_by":"deadline","tone":"overdue"},
      {"id":"b","text":"Standup","list_id":"events","state":"backlog","when":"2026-10-26T09:00","duration":15,"placed_by":"when","tone":"neutral"},
      {"id":"c","text":"Dentist","list_id":"events","state":"backlog","when":"2026-10-26T14:30","duration":60,"placed_by":"when","tone":"neutral"},
      {"id":"d","text":"Groceries","list_id":"inbox","state":"done","when":"2026-10-26","placed_by":"when","tone":"neutral"},
      {"id":"e","text":"Pay rent","list_id":"inbox","state":"todo","deadline":"2026-10-26","placed_by":"deadline","tone":"warning"}
    ]},
    {"day":null,"today":false,"rows":[{"id":"u","text":"Someday","list_id":"events","state":"backlog","tone":"neutral"}]},
    {"day":"2026-10-27","today":false,"rows":[]}]
    """

    func date(_ h: Int, _ m: Int) -> Date {
        Calendar.current.date(from: DateComponents(year: 2026, month: 10, day: 26, hour: h, minute: m))!
    }

    func testParsesTheDaemonsShape() throws {
        let days = try Agenda.parse(sample)
        XCTAssertEqual(days.count, 3)
        XCTAssertEqual(days[0].rows.map(\.id), ["a", "b", "c", "d", "e"])
        XCTAssertEqual(days[0].rows[1].duration, 15)
        XCTAssertEqual(days[0].rows[2].listId, "events")
        XCTAssertNil(days[1].day)
    }

    func testOnlyTodayBecomesLinesWithTimesAndNotes() throws {
        let lines = TodayMenu.lines(from: try Agenda.parse(sample), now: date(9, 5))
        XCTAssertEqual(lines.map(\.title), [
            "Overdue report", "09:00  Standup", "14:30  Dentist", "Groceries", "Pay rent",
        ])
        XCTAssertEqual(lines.map(\.note), ["overdue", "now", "in 5h 25m", nil, "due today"])
        XCTAssertEqual(lines.map(\.done), [false, false, false, true, false])
    }

    func testTimedNotesFollowTheClock() throws {
        let days = try Agenda.parse(sample)
        XCTAssertEqual(TodayMenu.lines(from: days, now: date(8, 59))[1].note, "in 1m")
        XCTAssertEqual(TodayMenu.lines(from: days, now: date(9, 20))[1].note, "started 20m ago")
        XCTAssertEqual(TodayMenu.lines(from: days, now: date(15, 0))[2].note, "now")
    }

    func testSpanFormatting() {
        XCTAssertEqual(TodayMenu.span(30), "<1m")
        XCTAssertEqual(TodayMenu.span(3 * 60), "3m")
        XCTAssertEqual(TodayMenu.span(2 * 3600 + 15 * 60), "2h 15m")
        XCTAssertEqual(TodayMenu.span(3 * 3600), "3h")
        XCTAssertEqual(TodayMenu.span(28 * 3600), "1d 4h")
    }

    func testWhenParsingIgnoresReservedZoneSuffix() {
        XCTAssertEqual(Agenda.start(of: "2026-10-26T14:00[Europe/London]"), date(14, 0))
        XCTAssertEqual(Agenda.start(of: "2026-10-26"), date(0, 0))
        XCTAssertNil(Agenda.start(of: "garbage"))
    }
}

extension AgendaTests {
    func testLongTitlesAreClippedToThirtyTwoWithEllipsis() {
        let exact = String(repeating: "a", count: 32)
        XCTAssertEqual(TodayMenu.clipped(exact), exact)
        let long = String(repeating: "b", count: 33)
        let clipped = TodayMenu.clipped(long)
        XCTAssertEqual(clipped.count, 32)
        XCTAssertTrue(clipped.hasSuffix("…"))
        XCTAssertEqual(clipped, String(repeating: "b", count: 31) + "…")
        // The time prefix is outside the cap.
        let row = """
        [{"day":"2026-10-26","today":true,"rows":[{"id":"x","text":"\(long)","list_id":"events","state":"backlog","when":"2026-10-26T09:00","tone":"neutral"}]}]
        """
        let line = TodayMenu.lines(from: try! Agenda.parse(row), now: date(8, 0))[0]
        XCTAssertEqual(line.title, "09:00  " + clipped)
    }
}
