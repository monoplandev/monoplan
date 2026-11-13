import XCTest
@testable import MonoplanMacKit

final class ItemDetailTests: XCTestCase {
    func testParsesTheShowShape() throws {
        let item = try ItemDetail.parse("""
        {"id":"i1","text":"Dentist","list_id":"l1","state":"done","lifecycle_at":30,
         "created_at":10,"started_at":null,"done_at":25,"binned_at":null,
         "deadline":"2026-09-14","when":"2026-09-12T14:00","duration":90,
         "place":{"label":"Clinic","lat":1.5,"lon":2.5,"address":"1 High St"},
         "notes":"bring forms","list_name":"Health","list_icon":"🩺","starred":true}
        """)
        XCTAssertEqual(item.text, "Dentist")
        XCTAssertEqual(item.notes, "bring forms")
        XCTAssertEqual(item.listLabel, "Health")
        XCTAssertEqual(item.listIcon, "🩺")
        XCTAssertTrue(item.starred)
        XCTAssertTrue(item.isTimed)
        XCTAssertEqual(item.duration, 90)
        XCTAssertEqual(item.place, PlaceInfo(label: "Clinic", lat: 1.5, lon: 2.5, address: "1 High St"))
        // Done: the completion stamp, not the register's transition.
        XCTAssertEqual(item.closedAt, 25)
    }

    func testBareItemInAReservedList() throws {
        let item = try ItemDetail.parse("""
        {"id":"i2","text":"loose","list_id":"inbox","state":"backlog","lifecycle_at":10,
         "created_at":10,"started_at":null,"done_at":null,"binned_at":null,"notes":"","starred":false}
        """)
        XCTAssertEqual(item.listLabel, "Inbox")
        XCTAssertNil(item.listIcon)
        XCTAssertNil(item.when)
        XCTAssertNil(item.place)
        XCTAssertFalse(item.isTimed)
        XCTAssertNil(item.closedAt)
        XCTAssertNil(item.pendingSuggestion)
    }

    func testASuggestionIsPendingOnlyOnAnOpenUndatedItem() throws {
        func item(_ extra: String, state: String = "backlog", binned: String = "null") throws -> ItemDetail {
            try ItemDetail.parse("""
            {"id":"i3","text":"dentist friday","list_id":"inbox","state":"\(state)","lifecycle_at":10,
             "created_at":10,"started_at":null,"done_at":null,"binned_at":\(binned),
             "when_suggested":"2026-11-20"\(extra),"notes":"","starred":false}
            """)
        }
        XCTAssertEqual(try item("").pendingSuggestion, "2026-11-20")
        XCTAssertNil(try item(#","when":"2026-11-21""#).pendingSuggestion)
        XCTAssertNil(try item("", state: "done").pendingSuggestion)
        XCTAssertNil(try item("", binned: "99").pendingSuggestion)
    }

    func testParsesKnownPlaces() throws {
        let rows = try KnownPlace.parse("""
        [{"label":"Gym","lat":1.5,"lon":2.5,"address":"1 High St","count":3,"last_used":99},
         {"label":"Office","count":1,"last_used":5}]
        """)
        XCTAssertEqual(rows.map(\.label), ["Gym", "Office"])
        XCTAssertEqual(rows[0].place, PlaceInfo(label: "Gym", lat: 1.5, lon: 2.5, address: "1 High St"))
        XCTAssertEqual(rows[1].place, PlaceInfo(label: "Office"))
        XCTAssertEqual(rows[0].count, 3)
    }

    func testCancelledClosesAtTheRegistersInstant() throws {
        let item = try ItemDetail.parse("""
        {"id":"i3","text":"x","list_id":"inbox","state":"cancelled","lifecycle_at":40,
         "created_at":10,"started_at":null,"done_at":25,"binned_at":null,"notes":"","starred":false}
        """)
        XCTAssertEqual(item.closedAt, 40)
    }
}
