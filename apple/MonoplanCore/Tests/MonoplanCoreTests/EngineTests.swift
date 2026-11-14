import XCTest
@testable import MonoplanCore

/// The embedded engine's start-up contract, without a server: the two
/// refusals the app branches on. A full engine run, and the takeover of
/// a headless daemon, need an account and live in `cli/tests/daemon.rs`.
final class EngineTests: XCTestCase {
    private func tempDir() throws -> String {
        let dir = (NSTemporaryDirectory() as NSString)
            .appendingPathComponent("monoplan-engine-\(UUID().uuidString)")
        try FileManager.default.createDirectory(atPath: dir, withIntermediateDirectories: true)
        return dir
    }

    func testNoProfileIsNotLoggedIn() throws {
        let dir = try tempDir()
        defer { try? FileManager.default.removeItem(atPath: dir) }
        XCTAssertThrowsError(try MonoplanCore.Engine.start(profileDir: dir)) { error in
            XCTAssertEqual(error as? EngineError, .NotLoggedIn)
        }
    }

    func testHeldLockWithNoEngineIsBusy() throws {
        let dir = try tempDir()
        defer { try? FileManager.default.removeItem(atPath: dir) }
        // Signed in by the CLI's rule: secrets.toml exists.
        FileManager.default.createFile(atPath: dir + "/secrets.toml", contents: Data())
        // Someone holds engine.lock with no socket to ask: an engine still
        // starting, or a CLI command rewriting the profile.
        let fd = open(dir + "/engine.lock", O_RDWR | O_CREAT, 0o644)
        XCTAssertGreaterThanOrEqual(fd, 0)
        defer { close(fd) }
        XCTAssertEqual(flock(fd, LOCK_EX | LOCK_NB), 0)
        XCTAssertThrowsError(try MonoplanCore.Engine.start(profileDir: dir)) { error in
            XCTAssertEqual(error as? EngineError, .Busy)
        }
    }

    /// The typed calls against a local-only engine (no server): an item
    /// added, listed, ticked and counted, the observer told, and a
    /// workspace nobody has refused. Records, enums and errors all cross
    /// from the `monoplan_cli` crate's generated file.
    func testTypedCallsRoundTripOnALocalOnlyEngine() async throws {
        let dir = try tempDir()
        defer { try? FileManager.default.removeItem(atPath: dir) }
        try ensureLocalWorkspace(profileDir: dir)
        let engine = try MonoplanCore.Engine.start(profileDir: dir)
        defer { engine.shutdown() }
        let calls = EngineCalls(engine)

        let changed = expectation(description: "changed")
        changed.assertForOverFulfill = false
        let synced = expectation(description: "sync state on observe")
        synced.assertForOverFulfill = false
        let watcher = try engine.observe(
            onChanged: { changed.fulfill() },
            onSync: { state in
                XCTAssertTrue(state.localOnly)
                synced.fulfill()
            })
        defer { watcher.cancel() }

        let id = try await calls.run {
            try $0.addItem(workspace: nil, list: "inbox", text: "from swift", top: true)
        }
        var rows = try await calls.run { try $0.items(workspace: nil, list: "inbox", includeDone: false) }
        XCTAssertEqual(rows.first?.id, id)
        XCTAssertEqual(rows.first?.text, "from swift")
        XCTAssertEqual(rows.first?.state, .backlog)
        XCTAssertNil(rows.first?.doneAt)

        try await calls.run { try $0.setDone(workspace: nil, id: id, done: true) }
        rows = try await calls.run { try $0.items(workspace: nil, list: "inbox", includeDone: false) }
        XCTAssertFalse(rows.contains { $0.id == id })
        rows = try await calls.run { try $0.items(workspace: nil, list: "inbox", includeDone: true) }
        let done = try XCTUnwrap(rows.first { $0.id == id })
        XCTAssertEqual(done.state, .done)
        XCTAssertNotNil(done.doneAt)

        // A fresh workspace has nothing archived.
        let archived = try await calls.run { try $0.lists(workspace: nil, filter: .archived) }
        XCTAssertTrue(archived.isEmpty)

        do {
            _ = try await calls.run { try $0.items(workspace: "nope", list: "inbox", includeDone: false) }
            XCTFail("an unknown workspace should throw")
        } catch let error as EngineError {
            guard case .Failed = error else { return XCTFail("unexpected: \(error)") }
        }

        await fulfillment(of: [changed, synced], timeout: 5)
    }

    /// The callback form of `run`: results on the main queue, and in
    /// one order with the async form (a read made after a write sees
    /// it, whichever form each used). Rows list by their own id.
    func testCallbackCallsDeliverOnMainInOrder() async throws {
        let dir = try tempDir()
        defer { try? FileManager.default.removeItem(atPath: dir) }
        try ensureLocalWorkspace(profileDir: dir)
        let engine = try MonoplanCore.Engine.start(profileDir: dir)
        defer { engine.shutdown() }
        let calls = EngineCalls(engine)

        let added = expectation(description: "added")
        var newId: String?
        calls.run({ try $0.addItem(workspace: nil, list: "inbox", text: "by callback", top: true) }) { result in
            XCTAssertTrue(Thread.isMainThread)
            newId = try? result.get()
            added.fulfill()
        }
        // Made before the write's completion has run: still after it.
        let rows = try await calls.run { try $0.items(workspace: nil, list: "inbox", includeDone: false) }
        await fulfillment(of: [added], timeout: 5)
        let id = try XCTUnwrap(newId)
        XCTAssertEqual(rows.first?.id, id)
        func listed<Row: Identifiable>(_ rows: [Row]) -> [Row.ID] { rows.map(\.id) }
        XCTAssertEqual(listed(rows).first, id)

        let failed = expectation(description: "failed")
        calls.run({ try $0.lists(workspace: "nope", filter: .all) }) { result in
            if case .failure(EngineError.Failed) = result { failed.fulfill() }
        }
        await fulfillment(of: [failed], timeout: 5)
    }
}
