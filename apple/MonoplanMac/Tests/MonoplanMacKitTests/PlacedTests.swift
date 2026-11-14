import XCTest
@testable import MonoplanMacKit

final class PlacedTests: XCTestCase {
    func testParsesTheEnginesShape() throws {
        let rows = try Placed.parse("""
        [{"id":"a","text":"lift","list_id":"inbox","state":"backlog","lifecycle_at":1,"created_at":1,
          "started_at":null,"done_at":null,"binned_at":null,
          "place":{"label":"Gym","lat":1.5,"lon":2.5,"address":"1 High St"},
          "list_name":"Inbox","doc_id":"d1"},
         {"id":"b","text":"standup","list_id":"ops","state":"todo","lifecycle_at":1,"created_at":1,
          "started_at":null,"done_at":null,"binned_at":null,"when":"2026-11-20T09:00",
          "place":{"label":"Office"},"list_name":"Ops","doc_id":"d2","doc_name":"Work"}]
        """)
        XCTAssertEqual(rows.map(\.key), ["d1/a", "d2/b"])
        XCTAssertEqual(rows[0].place, PlaceInfo(label: "Gym", lat: 1.5, lon: 2.5, address: "1 High St"))
        XCTAssertNil(rows[0].docName)
        XCTAssertNil(rows[1].place.lat)
        XCTAssertEqual(rows[1].docName, "Work")
        XCTAssertEqual(rows[1].when, "2026-11-20T09:00")
    }
}
