import XCTest
@testable import MonoplanDndCore

final class DndListModelTests: XCTestCase {
    /// Rows r0...r(n-1), 32 px each, in a 200 × 320 viewport (10 rows).
    private func model(_ n: Int = 20) -> DndListModel {
        let m = DndListModel(order: (0..<n).map { "r\($0)" })
        m.viewport = DndViewport(scrollTop: 0, width: 200, height: 320)
        return m
    }

    /// The centre of slot `i` in viewport coordinates.
    private func slot(_ i: Int, _ m: DndListModel) -> CGPoint {
        CGPoint(x: 100, y: CGFloat(i) * 32 + 16 - m.viewport.scrollTop)
    }

    private func press(_ m: DndListModel, row i: Int, _ mods: DndModifiers = []) {
        let p = slot(i, m)
        m.pointerDown(key: m.key(at: p), at: p, modifiers: mods)
    }

    private func click(_ m: DndListModel, row i: Int, _ mods: DndModifiers = []) {
        press(m, row: i, mods)
        m.click(modifiers: mods)
    }

    // MARK: Windowing

    func testRendersTheViewportPlusOverscan() {
        let m = model(1000)
        XCTAssertEqual(m.renderState().items.map(\.index), Array(0..<14))
        m.viewport.scrollTop = 3200
        let state = m.renderState()
        XCTAssertEqual(state.items.first?.index, 98)
        XCTAssertEqual(state.items.count, 14)
        XCTAssertEqual(state.listHeight, 32_000)
    }

    func testShortListFillsTheViewport() {
        let m = model(3)
        let state = m.renderState()
        XCTAssertEqual(state.contentHeight, 96)
        XCTAssertEqual(state.listHeight, 320)
    }

    // MARK: Clicks

    func testClickModifiers() {
        let m = model()
        click(m, row: 2)
        click(m, row: 4, .shift)
        XCTAssertEqual(m.selection.selectedKeys, ["r2", "r3", "r4"])
        click(m, row: 7, .command)
        XCTAssertEqual(m.selection.selectedKeys, ["r2", "r3", "r4", "r7"])
        click(m, row: 3, .command)
        XCTAssertEqual(m.selection.selectedKeys, ["r2", "r4", "r7"])
        click(m, row: 5)
        XCTAssertEqual(m.selection.selectedKeys, ["r5"])
    }

    func testPressOnSelectedRowKeepsTheSelectionUntilRelease() {
        let m = model()
        click(m, row: 1)
        click(m, row: 3, .shift)
        press(m, row: 2)
        XCTAssertEqual(m.selection.selectedKeys, ["r1", "r2", "r3"])
        m.click()
        XCTAssertEqual(m.selection.selectedKeys, ["r2"])
    }

    func testBlankClickClearsOnlyWhenConfigured() {
        let m = model(3)
        click(m, row: 0)
        click(m, row: 8)
        XCTAssertEqual(m.selection.selectedKeys, ["r0"])
        m.config.clearOnBlankClick = true
        click(m, row: 8)
        XCTAssertFalse(m.selection.hasSelection)
    }

    func testRoundedRunEnds() {
        let m = model()
        click(m, row: 1)
        click(m, row: 3, .shift)
        let rows = m.renderState().items
        XCTAssertEqual(rows.filter(\.selected).map(\.key), ["r1", "r2", "r3"])
        XCTAssertTrue(rows[1].selFirst && !rows[1].selLast)
        XCTAssertTrue(!rows[2].selFirst && !rows[2].selLast)
        XCTAssertTrue(!rows[3].selFirst && rows[3].selLast)
    }

    // MARK: Drag

    func testDragStartsOnlyPastTheSlop() {
        let m = model()
        press(m, row: 2)
        XCTAssertNil(m.pointerDragged(to: CGPoint(x: 101, y: 82)))
        XCTAssertFalse(m.isDragging)
        XCTAssertEqual(m.pointerDragged(to: CGPoint(x: 100, y: 84)), .began)
        XCTAssertEqual(m.pointerDragged(to: CGPoint(x: 100, y: 90)), .moved)
    }

    func testModifiedPressNeverDrags() {
        let m = model()
        press(m, row: 2, .command)
        XCTAssertNil(m.pointerDragged(to: slot(6, m)))
        XCTAssertFalse(m.isDragging)
    }

    /// At the start of a one-row drag the collapsed list with its nudge
    /// slot under the pointer is the layout the user was looking at:
    /// nothing jumps.
    func testSingleRowDragStartsInPlace() {
        let m = model()
        let before = m.renderState().items
        press(m, row: 2)
        _ = m.pointerDragged(to: CGPoint(x: 100, y: 84))
        let state = m.renderState()
        XCTAssertEqual(m.drag?.hoverIndex, 2)
        XCTAssertEqual(state.placeholderY, 64)
        XCTAssertEqual(state.contentHeight, 640)
        for row in state.items {
            // One more row is windowed in below; it has no "before".
            guard let was = before.first(where: { $0.key == row.key }) else { continue }
            XCTAssertEqual(row.top, was.top, row.key)
        }
        XCTAssertNil(state.items.first { $0.key == "r2" })
    }

    func testNudgeOpensOneSlotInCollapsedSpace() {
        let m = model()
        click(m, row: 1)
        click(m, row: 3, .command)
        click(m, row: 4, .command)
        press(m, row: 3)
        _ = m.pointerDragged(to: slot(3, m).applying(.init(translationX: 0, y: 4)))
        // Dragging r1, r3, r4 leaves r0, r2, r5... : slot 5 is before r7.
        _ = m.pointerDragged(to: slot(5, m))
        let state = m.renderState()
        XCTAssertEqual(m.drag?.collapsed.prefix(6), ["r0", "r2", "r5", "r6", "r7", "r8"])
        XCTAssertEqual(state.items.map(\.key).prefix(6), ["r0", "r2", "r5", "r6", "r7", "r8"])
        XCTAssertEqual(state.items.map(\.top).prefix(6), [0, 32, 64, 96, 128, 192])
        XCTAssertEqual(state.placeholderY, 160)
        XCTAssertFalse(state.items.contains { $0.selected })
        // One slot, however many rows are dragged.
        XCTAssertEqual(state.contentHeight, CGFloat(17 + 1) * 32)
    }

    func testDropMovesTheBlockAboveTheHoverRow() {
        let m = model(8)
        click(m, row: 1)
        click(m, row: 3, .command)
        press(m, row: 3)
        _ = m.pointerDragged(to: CGPoint(x: 100, y: 120))
        _ = m.pointerDragged(to: slot(4, m))
        // Collapsed: r0 r2 r4 r5 r6 r7; slot 4 is r6.
        let move = m.endDrag()
        XCTAssertEqual(move, DndMove(keys: ["r1", "r3"], before: "r6"))
        XCTAssertEqual(m.order, ["r0", "r2", "r4", "r5", "r1", "r3", "r6", "r7"])
        XCTAssertEqual(m.selection.selectedKeys, ["r1", "r3"])
        XCTAssertEqual(m.selection.snapshot.blocks.count, 1)
        XCTAssertFalse(m.isDragging)
    }

    func testDropPastTheLastRowAppends() {
        let m = model(4)
        press(m, row: 0)
        _ = m.pointerDragged(to: CGPoint(x: 100, y: 30))
        // Below the three remaining rows, still in the viewport.
        _ = m.pointerDragged(to: CGPoint(x: 100, y: 300))
        XCTAssertEqual(m.drag?.hoverIndex, 3)
        XCTAssertEqual(m.renderState().placeholderY, 96)
        XCTAssertEqual(m.endDrag(), DndMove(keys: ["r0"], before: nil))
        XCTAssertEqual(m.order, ["r1", "r2", "r3", "r0"])
    }

    func testDropBackInPlaceIsNoMove() {
        let m = model()
        press(m, row: 2)
        _ = m.pointerDragged(to: CGPoint(x: 100, y: 84))
        XCTAssertNil(m.endDrag())
        XCTAssertEqual(m.order.prefix(4), ["r0", "r1", "r2", "r3"])
    }

    func testLeavingTheViewportResetsTheNudge() {
        let m = model()
        press(m, row: 2)
        _ = m.pointerDragged(to: slot(5, m))
        _ = m.pointerDragged(to: CGPoint(x: 260, y: 100))
        let state = m.renderState()
        XCTAssertNil(m.drag?.hoverIndex)
        XCTAssertNil(state.placeholderY)
        XCTAssertEqual(state.items.map(\.top).prefix(4), [0, 32, 64, 96])
        // Released out there: the rows snap back.
        XCTAssertNil(m.endDrag())
        XCTAssertEqual(m.order.prefix(4), ["r0", "r1", "r2", "r3"])
    }

    func testConsumedDropAndCancelLeaveTheOrder() {
        let m = model()
        press(m, row: 2)
        _ = m.pointerDragged(to: slot(6, m))
        XCTAssertNil(m.endDrag(consumed: true))
        press(m, row: 2)
        _ = m.pointerDragged(to: slot(6, m))
        XCTAssertTrue(m.cancelDrag())
        XCTAssertFalse(m.cancelDrag())
        XCTAssertEqual(m.order.prefix(4), ["r0", "r1", "r2", "r3"])
        XCTAssertEqual(m.selection.selectedKeys, ["r2"])
    }

    func testHoverFollowsTheScrollUnderAStillPointer() {
        let m = model(100)
        press(m, row: 2)
        _ = m.pointerDragged(to: CGPoint(x: 100, y: 310))
        XCTAssertEqual(m.drag?.hoverIndex, 9)
        m.viewport.scrollTop = 320
        m.scrolled()
        XCTAssertEqual(m.drag?.hoverIndex, 19)
        XCTAssertEqual(m.renderState().placeholderY, 19 * 32 - 320)
    }

    func testLockedOrderDragsButNeverReorders() {
        let m = model()
        m.config.reorder = false
        press(m, row: 2)
        XCTAssertEqual(m.pointerDragged(to: slot(6, m)), .began)
        let state = m.renderState()
        XCTAssertNil(state.placeholderY)
        XCTAssertEqual(state.items.map(\.top).prefix(4), [0, 32, 64, 96])
        XCTAssertNil(m.endDrag())
        XCTAssertNil(m.handleKey(.moveSelection(.down))?.move)
    }

    func testOrderChangeMidDrag() {
        let m = model(6)
        click(m, row: 1)
        click(m, row: 2, .shift)
        press(m, row: 1)
        _ = m.pointerDragged(to: slot(3, m))
        m.setOrder(["r0", "r2", "r3", "r5"])
        XCTAssertEqual(m.drag?.keys, ["r2"])
        XCTAssertEqual(m.drag?.collapsed, ["r0", "r3", "r5"])
        m.setOrder(["r0", "r3", "r5"])
        XCTAssertFalse(m.isDragging)
    }

    // MARK: Keyboard

    func testArrowNavigation() {
        let m = model(5)
        XCTAssertEqual(m.handleKey(.navigate(.down))?.scrollTo, "r0")
        XCTAssertEqual(m.handleKey(.navigate(.down))?.scrollTo, "r1")
        _ = m.handleKey(.extend(.down))
        _ = m.handleKey(.extend(.down))
        XCTAssertEqual(m.selection.selectedKeys, ["r1", "r2", "r3"])
        // Plain arrows leave from the selection's edge.
        XCTAssertEqual(m.handleKey(.navigate(.up))?.scrollTo, "r0")
        XCTAssertEqual(m.handleKey(.selectOnlyLast)?.scrollTo, "r4")
        XCTAssertTrue(m.navigateDownIsPastEnd)
        _ = m.handleKey(.extendToFirst)
        XCTAssertEqual(m.selection.selectedKeys, m.order)
        _ = m.handleKey(.clear)
        XCTAssertFalse(m.selection.hasSelection)
        XCTAssertEqual(m.handleKey(.navigate(.up))?.scrollTo, "r4")
    }

    func testArrowNavigateOffLeavesPlainArrowsUnanswered() {
        let m = model(5)
        m.config.arrowNavigate = false
        XCTAssertNil(m.handleKey(.navigate(.down)))
        XCTAssertNotNil(m.handleKey(.extend(.down)))
    }

    func testSingleSelectCollapsesMultiGestures() {
        let m = model(5)
        m.config.multi = false
        click(m, row: 1)
        click(m, row: 3, .shift)
        XCTAssertEqual(m.selection.selectedKeys, ["r3"])
        _ = m.handleKey(.extend(.down))
        XCTAssertEqual(m.selection.selectedKeys, ["r4"])
        _ = m.handleKey(.selectAll)
        XCTAssertEqual(m.selection.selectedKeys, ["r4"])
    }

    func testMoveSelectionWithTheKeyboard() {
        let m = model(5)
        click(m, row: 1)
        click(m, row: 2, .shift)
        var result = m.handleKey(.moveSelection(.down))
        XCTAssertEqual(result?.move, DndMove(keys: ["r1", "r2"], before: "r4"))
        XCTAssertEqual(result?.scrollTo, "r2")
        XCTAssertEqual(m.order, ["r0", "r3", "r1", "r2", "r4"])
        result = m.handleKey(.moveSelection(.down))
        XCTAssertEqual(result?.move, DndMove(keys: ["r1", "r2"], before: nil))
        XCTAssertNil(m.handleKey(.moveSelection(.down))?.move)
        result = m.handleKey(.moveSelection(.up))
        XCTAssertEqual(result?.move, DndMove(keys: ["r1", "r2"], before: "r4"))
        XCTAssertEqual(result?.scrollTo, "r1")
        XCTAssertEqual(m.selection.selectedKeys, ["r1", "r2"])
    }

    func testScrollOffsetToReveal() {
        let m = model(100)
        XCTAssertNil(m.scrollOffset(toReveal: "r5"))
        XCTAssertEqual(m.scrollOffset(toReveal: "r10"), 32)
        // The last row goes to the end of the content.
        XCTAssertEqual(m.scrollOffset(toReveal: "r99"), 3200 - 320)
        m.viewport.scrollTop = 640
        XCTAssertEqual(m.scrollOffset(toReveal: "r3"), 96)
    }

    func testTopInset() {
        let m = model(100)
        m.viewport.topInset = 52
        m.viewport.scrollTop = -52
        XCTAssertEqual(m.minScrollTop, -52)
        // Row 0 is in view below the inset; row 8 ends at 288 + 52.
        XCTAssertNil(m.scrollOffset(toReveal: "r0"))
        XCTAssertEqual(m.scrollOffset(toReveal: "r9"), 320 - 320)
        m.viewport.scrollTop = 320
        // Scrolling up to a row leaves it clear of the inset.
        XCTAssertEqual(m.scrollOffset(toReveal: "r5"), 160 - 52)
        // A short list is not made scrollable by the inset.
        let short = model(2)
        short.viewport.topInset = 52
        XCTAssertEqual(short.renderState().listHeight, 320 - 52)
        XCTAssertEqual(short.maxScrollTop, -52)
    }

    func testDragOff() {
        let m = model()
        m.config.drag = false
        press(m, row: 2)
        XCTAssertNil(m.pointerDragged(to: slot(6, m)))
        m.click()
        XCTAssertEqual(m.selection.selectedKeys, ["r2"])
    }

    func testSlideStepsLandOnWholePixels() {
        let steps = dndSlideSteps(delta: 32, steps: 18, scale: 2)
        XCTAssertEqual(steps.first, 32)
        XCTAssertTrue(steps.allSatisfy { ($0 * 2).rounded() == $0 * 2 })
        XCTAssertEqual(steps, steps.sorted(by: >))
        XCTAssertLessThan(steps.last ?? 32, 1)
        XCTAssertEqual(dndEase(0.5), 0.8024, accuracy: 0.001)
    }

    // MARK: Pieces

    func testKeyMap() {
        XCTAssertEqual(dndKeyAction(.down, []), .navigate(.down))
        XCTAssertEqual(dndKeyAction(.up, .shift), .extend(.up))
        XCTAssertEqual(dndKeyAction(.up, .command), .moveSelection(.up))
        XCTAssertEqual(dndKeyAction(.down, [.shift, .command]), .extendToLast)
        XCTAssertEqual(dndKeyAction(.up, .option), .selectOnlyFirst)
        XCTAssertEqual(dndKeyAction(.a, .command), .selectAll)
        XCTAssertNil(dndKeyAction(.a, []))
        XCTAssertEqual(dndKeyAction(.escape, []), .clear)
    }

    func testAutoscrollZonesAndRamp() {
        let viewport = DndViewport(scrollTop: 0, width: 200, height: 320)
        var auto = DndAutoscroll(buffer: 32)
        auto.update(pointer: CGPoint(x: 100, y: 160), viewport: viewport)
        XCTAssertNil(auto.direction)
        auto.update(pointer: CGPoint(x: 100, y: 300), viewport: viewport)
        XCTAssertEqual(auto.direction, .down)
        // 20% of 720 px/s at first, the full speed after half a second.
        XCTAssertEqual(auto.step(dt: 1.0 / 60), 2.4, accuracy: 0.001)
        for _ in 0..<40 { _ = auto.step(dt: 1.0 / 60) }
        XCTAssertEqual(auto.step(dt: 1.0 / 60), 12, accuracy: 0.001)
        // Unconfined: past the edge still scrolls; off to the side stops.
        auto.update(pointer: CGPoint(x: 100, y: -40), viewport: viewport)
        XCTAssertEqual(auto.direction, .up)
        XCTAssertLessThan(auto.step(dt: 1.0 / 60), 0)
        auto.update(pointer: CGPoint(x: 260, y: 10), viewport: viewport)
        XCTAssertNil(auto.direction)
        auto.confine = true
        auto.update(pointer: CGPoint(x: 100, y: -40), viewport: viewport)
        XCTAssertNil(auto.direction)
    }

    func testMoveApplied() {
        let order = ["a", "b", "c", "d"]
        XCTAssertEqual(DndMove(keys: ["a", "c"], before: "d").applied(to: order), ["b", "a", "c", "d"])
        XCTAssertEqual(DndMove(keys: ["a"], before: nil).applied(to: order), ["b", "c", "d", "a"])
        // A `before` that is itself moving resolves to the end.
        XCTAssertEqual(DndMove(keys: ["b", "c"], before: "c").applied(to: order), ["a", "d", "b", "c"])
    }
}
