import XCTest
import MonoplanCore
@testable import MonoplanMac

final class LingerTests: XCTestCase {
    private func rows(_ ids: String...) -> [ItemListRow] {
        ids.map { ItemListRow(id: $0, workspace: nil, text: $0, state: .todo) }
    }

    private func ids(_ rows: [ItemListRow]) -> [String] { rows.map(\.id) }

    func testATickedRowHoldsItsPlaceTickedBeforeAndAfterTheReadDropsIt() {
        var linger = Linger()
        let shown = rows("a", "b", "c")
        linger.close(shown[1], in: shown)
        // The write not yet read back: the row is still in the read.
        let before = linger.overlay(shown)
        XCTAssertEqual(ids(before), ["a", "b", "c"])
        XCTAssertEqual(before[1].state, .done)
        // Read back: gone from the read, put back where it was.
        let after = linger.overlay(rows("a", "c"))
        XCTAssertEqual(ids(after), ["a", "b", "c"])
        XCTAssertEqual(after[1].state, .done)
    }

    func testAChainOfTicksDownTheListKeepsEveryRowInPlace() {
        var linger = Linger()
        var shown = rows("a", "b", "c", "d")
        // Each tick lands on the row now under the pointer's next click,
        // the screen showing the earlier ones still lingering.
        linger.close(shown[0], in: shown)
        shown = linger.overlay(rows("b", "c", "d"))
        linger.close(shown[1], in: shown)
        shown = linger.overlay(rows("c", "d"))
        linger.close(shown[3], in: shown)
        let out = linger.overlay(rows("c"))
        XCTAssertEqual(ids(out), ["a", "b", "c", "d"])
        XCTAssertEqual(out.map(\.state), [.done, .done, .todo, .done])
    }

    func testAReopenedRowStaysUntilTheReadHasItBack() {
        var linger = Linger()
        let shown = rows("a", "b", "c")
        linger.close(shown[1], in: shown)
        linger.reopen("b")
        let waiting = linger.overlay(rows("a", "c"))
        XCTAssertEqual(ids(waiting), ["a", "b", "c"])
        XCTAssertEqual(waiting[1].state, .todo)
        // Back in the read, wherever it put it: the read's.
        XCTAssertEqual(ids(linger.overlay(rows("b", "a", "c"))), ["b", "a", "c"])
    }
}
