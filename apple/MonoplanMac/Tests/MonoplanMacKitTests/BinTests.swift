import XCTest
@testable import MonoplanMacKit

final class BinTests: XCTestCase {
    func testParsesBinnedRows() throws {
        let rows = try Bin.parse("""
        [{"id":"a","text":"Call the bank","list_id":"inbox","binned_at":1762992000000}]
        """)
        XCTAssertEqual(rows, [
            BinnedRow(id: "a", text: "Call the bank", listId: "inbox", binnedAt: 1_762_992_000_000),
        ])
        XCTAssertEqual(try Bin.parse("[]"), [])
    }
}
