import XCTest
@testable import MonoplanMacKit

final class DoneTests: XCTestCase {
    private var utc: Calendar {
        var calendar = Calendar(identifier: .gregorian)
        calendar.timeZone = TimeZone(identifier: "UTC")!
        return calendar
    }

    private func row(_ id: String, at millis: Int64) -> ItemRow {
        ItemRow(id: id, text: id, listId: "inbox", state: "done", lifecycleAt: millis, createdAt: 0)
    }

    func testGroupsByTheDayTickedNewestFirst() {
        let day: Int64 = 86_400_000
        let rows = [
            row("c", at: 2 * day + 5_000),
            row("b", at: 2 * day + 1_000),
            row("a", at: day - 1),
        ]
        let days = Done.days(from: rows, calendar: utc)
        XCTAssertEqual(days.map { $0.rows.map(\.id) }, [["c", "b"], ["a"]])
        XCTAssertEqual(days.map(\.day), [
            Date(timeIntervalSince1970: 2 * 86_400),
            Date(timeIntervalSince1970: 0),
        ])
    }

    func testNoRowsIsNoDays() {
        XCTAssertEqual(Done.days(from: []), [])
    }

    func testParsesTheTickStamp() throws {
        let rows = try Items.parse("""
        [{"id":"a","text":"Call the bank","list_id":"inbox","state":"done","lifecycle_at":1762992000000,
          "created_at":1,"started_at":null,"done_at":1762992000000,"binned_at":null}]
        """)
        XCTAssertEqual(rows[0].lifecycleAt, 1_762_992_000_000)
    }
}
