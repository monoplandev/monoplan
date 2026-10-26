import XCTest
@testable import MonoplanMacKit

final class FocusTests: XCTestCase {
    let sample = """
    [{"pos":1,"id":"a","text":"Ship the tray","list_id":"work","state":"live"},
     {"pos":2,"id":"b","text":"Book flights","list_id":"inbox","state":"todo"},
     {"pos":3,"id":"c","text":"\(String(repeating: "c", count: 40))","list_id":"inbox","state":"backlog","place":{"label":"Home"}},
     {"pos":4,"id":"d","text":"Fourth","list_id":"inbox","state":"todo"}]
    """

    func testParsesTheDaemonsShapeIgnoringPlace() throws {
        let rows = try Focus.parse(sample)
        XCTAssertEqual(rows.map(\.id), ["a", "b", "c", "d"])
        XCTAssertEqual(rows[0].listId, "work")
    }

    func testShowsTheFirstThreeClipped() throws {
        let lines = StarredMenu.lines(from: try Focus.parse(sample))
        XCTAssertEqual(lines.map(\.id), ["a", "b", "c"])
        XCTAssertEqual(lines[2].title, String(repeating: "c", count: 31) + "…")
        XCTAssertEqual(lines.map(\.note), [nil, nil, nil])
        XCTAssertTrue(StarredMenu.lines(from: []).isEmpty)
    }
}
