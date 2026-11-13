import XCTest
@testable import MonoplanMacKit

final class FindTests: XCTestCase {
    func testParsesListAndItemHits() throws {
        let hits = try Find.parse("""
        [{"kind":"list","id":"l1","title":"Groceries","doc_id":"d1","icon":"🛒","score":10000},
         {"kind":"item","id":"i1","title":"Groceries budget","doc_id":"d2","doc_name":"Work",
          "list_id":"old","list_name":"Old","state":"binned","archived":true,"score":10000}]
        """)
        XCTAssertEqual(hits.map(\.kind), [.list, .item])
        XCTAssertEqual(hits[0].icon, "🛒")
        XCTAssertFalse(hits[0].archived)
        XCTAssertNil(hits[0].docName)
        XCTAssertEqual(hits[1].docName, "Work")
        XCTAssertEqual(hits[1].listName, "Old")
        XCTAssertEqual(hits[1].state, "binned")
        XCTAssertTrue(hits[1].archived)
        // A list and an item may share an id across docs and kinds.
        XCTAssertEqual(Set(hits.map(\.id)).count, 2)
        XCTAssertTrue(try Find.parse("[]").isEmpty)
    }
}
