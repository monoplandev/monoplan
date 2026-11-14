import XCTest
@testable import MonoplanMacKit

final class FocusTests: XCTestCase {
    let sample = """
    [{"pos":1,"id":"a","text":"Ship the tray","list_id":"work","state":"live"},
     {"pos":2,"id":"b","text":"Book flights","list_id":"inbox","state":"todo"},
     {"pos":3,"id":"c","text":"\(String(repeating: "c", count: 40))","list_id":"inbox","state":"backlog","place":{"label":"Home"}},
     {"pos":4,"id":"d","text":"Fourth","list_id":"inbox","state":"todo"}]
    """

    func testParsesTheEnginesShapeIgnoringPlace() throws {
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

    func testActiveListsDropArchivedAndReservedRows() throws {
        let rows = try Lists.parse("""
        [{"id":"work","name":"Work","archived_at":null,"created_at":1},
         {"id":"old","name":"Old","archived_at":5,"created_at":1},
         {"id":"inbox","name":"stray","archived_at":null,"created_at":1},
         {"id":"home","name":"Home","icon":"🏠","archived_at":null,"created_at":2}]
        """)
        XCTAssertEqual(Lists.active(rows).map(\.id), ["work", "home"])
        XCTAssertEqual(Lists.active(rows).map(\.icon), [nil, "🏠"])
    }

    func testListNamesBecomeTheMutedNote() throws {
        let names = Lists.names(from: try Lists.parse("""
        [{"id":"work","name":"Work","archived_at":null,"created_at":1},
         {"id":"inbox","name":"should not shadow","archived_at":null,"created_at":1}]
        """))
        XCTAssertEqual(names, ["work": "Work", "inbox": "Inbox", "events": "Calendar"])
        let lines = StarredMenu.lines(from: try Focus.parse(sample), listNames: names)
        XCTAssertEqual(lines.map(\.note), ["Work", "Inbox", "Inbox"])
        // Unknown ids show nothing rather than a uuid.
        let orphan = try Focus.parse("""
        [{"pos":1,"id":"z","text":"t","list_id":"0192-uuid","state":"todo"}]
        """)
        XCTAssertNil(StarredMenu.lines(from: orphan, listNames: names)[0].note)
    }

    func testFullLensRowsCarryWorkspaceAndListName() throws {
        let json = """
        [{"pos":1,"id":"w","text":"Work row","list_id":"inbox","state":"todo",
          "doc_id":"00000000-0000-0000-0000-000000000002","doc_name":"Work","list_name":"Inbox"},
         {"pos":2,"id":"h","text":"Home row","list_id":"abc","state":"backlog",
          "doc_id":"00000000-0000-0000-0000-000000000001","list_name":"Errands"}]
        """
        let rows = try Focus.parse(json)
        XCTAssertEqual(rows[0].docName, "Work")
        XCTAssertEqual(rows[0].listName, "Inbox")
        XCTAssertNil(rows[1].docName)
        XCTAssertEqual(rows[1].listName, "Errands")
        XCTAssertNil(try Focus.parse(sample)[0].docId)
    }
}
