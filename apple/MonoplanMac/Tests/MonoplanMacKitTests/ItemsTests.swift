import XCTest
@testable import MonoplanMacKit

final class ItemsTests: XCTestCase {
    /// `item ls --json` rows: the optional dates are absent when unset,
    /// `place` and the per-state stamps are present but not decoded.
    let sample = """
    [{"id":"a","text":"Call the bank","list_id":"inbox","state":"backlog","lifecycle_at":1,"created_at":1,
      "started_at":null,"done_at":null,"binned_at":null},
     {"id":"b","text":"Dentist","list_id":"inbox","state":"todo","lifecycle_at":2,"created_at":2,
      "created_by":"acct","started_at":null,"done_at":null,"binned_at":null,
      "deadline":"2026-11-20","when":"2026-11-19T09:30","duration":45,"place":{"label":"Clinic"}}]
    """

    func testParsesTheDaemonsShape() throws {
        let rows = try Items.parse(sample)
        XCTAssertEqual(rows.map(\.id), ["a", "b"])
        XCTAssertEqual(rows[0].state, "backlog")
        XCTAssertNil(rows[0].deadline)
        XCTAssertNil(rows[0].when)
        XCTAssertEqual(rows[1].listId, "inbox")
        XCTAssertEqual(rows[1].deadline, "2026-11-20")
        XCTAssertEqual(rows[1].when, "2026-11-19T09:30")
        XCTAssertEqual(rows[1].createdAt, 2)
    }

    func testEmptyListIsNoRows() throws {
        XCTAssertEqual(try Items.parse("[]"), [])
    }
}
