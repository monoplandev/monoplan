import XCTest
@testable import MonoplanDndCore

final class DndSelectionTests: XCTestCase {
    private let order = ["a", "b", "c", "d", "e", "f"]

    func testSelectOnlyAndExtend() {
        let sel = DndSelection(order: order)
        sel.selectOnly("b")
        sel.extendActive("d")
        XCTAssertEqual(sel.selectedKeys, ["b", "c", "d"])
        // Inside the block: contracts toward the anchor.
        sel.extendActive("c")
        XCTAssertEqual(sel.selectedKeys, ["b", "c"])
    }

    func testExtendPastAnchorPivotsRatherThanFlips() {
        let sel = DndSelection(order: order)
        sel.selectOnly("c")
        sel.extendActive("e")
        // The far side of the anchor: nothing already selected is dropped.
        sel.extendActive("a")
        XCTAssertEqual(sel.selectedKeys, ["a", "b", "c", "d", "e"])
        XCTAssertEqual(sel.activeBlock, DndBlock(anchor: "e", to: "a"))
    }

    func testToggleAddsSplitsAndRemoves() {
        let sel = DndSelection(order: order)
        sel.selectOnly("a")
        sel.toggle("c")
        XCTAssertEqual(sel.selectedKeys, ["a", "c"])
        XCTAssertEqual(sel.snapshot.blocks.count, 2)

        sel.selectOnly("a")
        sel.extendActive("e")
        sel.toggle("c")
        XCTAssertEqual(sel.selectedKeys, ["a", "b", "d", "e"])
        // The lower half of a split stays active.
        XCTAssertEqual(sel.activeBlock, DndBlock(anchor: "d", to: "e"))

        sel.toggle("d")
        sel.toggle("e")
        sel.toggle("a")
        sel.toggle("b")
        XCTAssertFalse(sel.hasSelection)
        XCTAssertNil(sel.activeBlock)
    }

    func testAdjacentBlocksMerge() {
        let sel = DndSelection(order: order)
        sel.selectOnly("a")
        sel.addBlock("b")
        XCTAssertEqual(sel.snapshot.blocks.count, 1)
        XCTAssertEqual(sel.selectedKeys, ["a", "b"])
        // The active anchor survives the merge.
        XCTAssertEqual(sel.activeBlock, DndBlock(anchor: "b", to: "a"))
    }

    /// The web drops the lower block here (its merge keeps `anchor...top`
    /// when the anchor is interior to the merged range).
    func testFillingTheGapBetweenTwoBlocksKeepsBoth() {
        let sel = DndSelection(order: order)
        sel.selectOnly("a")
        sel.extendActive("b")
        sel.addBlock("d")
        sel.extendActive("e")
        sel.toggle("c")
        XCTAssertEqual(sel.selectedKeys, ["a", "b", "c", "d", "e"])
        XCTAssertEqual(sel.snapshot.blocks.count, 1)
    }

    func testSetSelectedKeysCoalesces() {
        let sel = DndSelection(order: order)
        sel.setSelectedKeys(["c", "e", "b"])
        XCTAssertEqual(sel.selectedKeys, ["b", "c", "e"])
        XCTAssertEqual(sel.snapshot.blocks.count, 2)
        sel.setSelectedKeys([])
        XCTAssertFalse(sel.hasSelection)
    }

    func testUpdateOrderSpansInsertedRowsAndDropsRemovedOnes() {
        let sel = DndSelection(order: ["a", "b", "c"])
        sel.selectOnly("a")
        sel.extendActive("c")
        sel.updateOrder(["a", "b", "d", "c"])
        XCTAssertEqual(sel.selectedKeys, ["a", "b", "d", "c"])

        var changes = 0
        sel.onChange = { changes += 1 }
        sel.updateOrder(["a", "d", "c"])
        XCTAssertEqual(sel.selectedKeys, ["a", "d", "c"])
        XCTAssertEqual(changes, 1)
        sel.updateOrder(["x", "y"])
        XCTAssertFalse(sel.hasSelection)
    }

    func testEdgesAndNeighbours() {
        let sel = DndSelection(order: order)
        XCTAssertEqual(sel.next("f"), "f")
        XCTAssertEqual(sel.prev("a"), "a")
        sel.setSelectedKeys(["b", "e"])
        XCTAssertEqual(sel.top, "b")
        XCTAssertEqual(sel.bottom, "e")
        sel.selectAll()
        XCTAssertEqual(sel.selectedKeys, order)
    }
}
