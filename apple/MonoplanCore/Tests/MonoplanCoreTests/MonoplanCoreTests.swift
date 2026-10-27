import XCTest
@testable import MonoplanCore

final class MonoplanCoreTests: XCTestCase {
    /// End-to-end smoke test of the FFI boundary: open a store with a
    /// generated DEK, mutate it, read views back — then close and reopen
    /// and assert the state replayed from disk (the acceptance gate for
    /// spec/swift-ffi-plan.md).
    func testCaptureReadAndReopen() throws {
        let dir = (NSTemporaryDirectory() as NSString)
            .appendingPathComponent("monoplan-smoke-\(UUID().uuidString)")
        try FileManager.default.createDirectory(
            atPath: dir, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(atPath: dir) }

        let dek = generateDek()
        XCTAssertEqual(dek.count, 32, "DEK is 32 raw bytes")

        var doneId = ""
        var groceriesId = ""

        // First handle: capture some work.
        do {
            let store = try MonoplanStore.open(dir: dir, dek: dek)
            _ = try store.addItem(listId: "inbox", text: "first")
            doneId = try store.addItem(listId: "inbox", text: "second")
            try store.setItemDone(itemId: doneId, done: true)
            groceriesId = try store.addList(name: "Groceries")
            _ = try store.addItem(listId: groceriesId, text: "milk")

            // itemsInList returns every non-binned item, done included.
            let main = store.itemsInList(listId: "inbox")
            XCTAssertEqual(main.map(\.text), ["first", "second"])
            let done = try XCTUnwrap(main.first { $0.id == doneId })
            XCTAssertTrue(done.isDone)
        }

        // Second handle over the same dir + DEK: state must replay.
        let store = try MonoplanStore.open(dir: dir, dek: dek)
        let main = store.itemsInList(listId: "inbox")
        XCTAssertEqual(main.map(\.text), ["first", "second"],
                       "items replay from disk on reopen")
        let done = try XCTUnwrap(main.first { $0.id == doneId })
        XCTAssertTrue(done.isDone, "done status survived reopen")

        let groceries = store.itemsInList(listId: groceriesId)
        XCTAssertEqual(groceries.map(\.text), ["milk"])

        let lists = store.allLists()
        XCTAssertEqual(lists.map(\.name), ["Groceries"])
    }

    /// A store opened with the wrong DEK can't decrypt the persisted ops,
    /// so booting surfaces an error rather than silently losing data.
    func testWrongKeyFailsToBoot() throws {
        let dir = (NSTemporaryDirectory() as NSString)
            .appendingPathComponent("monoplan-wrongkey-\(UUID().uuidString)")
        try FileManager.default.createDirectory(
            atPath: dir, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(atPath: dir) }

        let dek = generateDek()
        do {
            let store = try MonoplanStore.open(dir: dir, dek: dek)
            _ = try store.addItem(listId: "inbox", text: "secret")
        }

        let wrongDek = generateDek()
        XCTAssertThrowsError(try MonoplanStore.open(dir: dir, dek: wrongDek))
    }
}
