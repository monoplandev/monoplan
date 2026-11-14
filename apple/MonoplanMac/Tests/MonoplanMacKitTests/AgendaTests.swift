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

    func testParsesTheEnginesShape() throws {
        let days = try Agenda.parse(sample)
        XCTAssertEqual(days.count, 3)
        XCTAssertEqual(days[0].rows.map(\.id), ["a", "b", "c", "d", "e"])
        XCTAssertEqual(days[0].rows[1].duration, 15)
        XCTAssertEqual(days[0].rows[2].listId, "events")
        XCTAssertNil(days[1].day)
    }

    func testWhenParsingIgnoresReservedZoneSuffix() {
        XCTAssertEqual(Agenda.start(of: "2026-10-26T14:00[Europe/London]"), date(14, 0))
        XCTAssertEqual(Agenda.start(of: "2026-10-26"), date(0, 0))
        XCTAssertNil(Agenda.start(of: "garbage"))
    }
}

extension AgendaTests {
    func testMergedRowsCarryTheirWorkspace() throws {
        let json = """
        [{"day":"2026-09-09","today":true,"rows":[
          {"id":"a","text":"Home row","list_id":"inbox","state":"todo","placed_by":"when","tone":"neutral",
           "when":"2026-09-09","doc_id":"00000000-0000-0000-0000-000000000001"},
          {"id":"b","text":"Work row","list_id":"inbox","state":"todo","placed_by":"when","tone":"neutral",
           "when":"2026-09-09","doc_id":"00000000-0000-0000-0000-000000000002","doc_name":"Work"}]}]
        """
        let rows = try Agenda.parse(json)[0].rows
        XCTAssertNil(rows[0].docName)
        XCTAssertEqual(rows[0].docId, "00000000-0000-0000-0000-000000000001")
        XCTAssertEqual(rows[1].docName, "Work")
    }
}
