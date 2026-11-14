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

    /// The rest of the typed surface (spec/swift-bindings-plan.md,
    /// phase 3) against a local-only engine: an item's page and every
    /// setter, the reads that span workspaces, list management, the
    /// bin, and the engine's own calls.
    func testTypedVerbsRoundTrip() async throws {
        let dir = try tempDir()
        defer { try? FileManager.default.removeItem(atPath: dir) }
        try ensureLocalWorkspace(profileDir: dir)
        let engine = try MonoplanCore.Engine.start(profileDir: dir)
        defer { engine.shutdown() }
        let calls = EngineCalls(engine)

        // The engine itself: no account, one workspace, nothing to sync to.
        let account = try await calls.run { try $0.account() }
        XCTAssertNil(account)
        let workspaces = try await calls.run { try $0.workspaces() }
        XCTAssertEqual(workspaces.count, 1)
        XCTAssertEqual(workspaces.first?.home, true)
        let home = try XCTUnwrap(workspaces.first?.docId)
        let sync = try await calls.run { try $0.sync() }
        XCTAssertTrue(sync.localOnly)
        do {
            _ = try await calls.run { try $0.createWorkspace(name: "Work") }
            XCTFail("a workspace needs an account")
        } catch let error as EngineError {
            guard case .Failed = error else { return XCTFail("unexpected: \(error)") }
        }
        XCTAssertFalse(buildId().isEmpty)

        // A list, and an item moved into it.
        let list = try await calls.run { try $0.addList(workspace: nil, name: "Errands") }
        try await calls.run { try $0.renameList(workspace: nil, list: list, name: "Chores") }
        try await calls.run { try $0.setListIcon(workspace: nil, list: list, icon: "🧹") }
        let id = try await calls.run {
            try $0.addItem(workspace: nil, list: "inbox", text: "sweep", top: true)
        }
        try await calls.run { try $0.moveItem(workspace: nil, id: id, list: list) }

        // The setters, read back from the item's page.
        try await calls.run {
            try $0.setText(workspace: nil, id: id, text: "sweep friday", suggestion: .set(when: "2030-01-04"))
        }
        var page = try await calls.run { try $0.item(workspace: nil, id: id) }
        XCTAssertEqual(page.item.text, "sweep friday")
        XCTAssertEqual(page.item.whenSuggested, "2030-01-04")
        XCTAssertEqual(page.listName, "Chores")
        XCTAssertEqual(page.listIcon, "🧹")
        XCTAssertFalse(page.starred)

        try await calls.run {
            try $0.setWhen(workspace: nil, id: id, when: "2030-01-04T09:00", duration: .minutes(minutes: 45))
        }
        try await calls.run { try $0.setDeadline(workspace: nil, id: id, deadline: "2030-01-05") }
        try await calls.run {
            try $0.setPlace(workspace: nil, id: id, place: Place(label: "Shed", lat: 1.5, lon: 2.5), exact: false)
        }
        try await calls.run { try $0.setState(workspace: nil, id: id, state: .inProgress) }
        try await calls.run { try $0.setStarred(workspace: nil, id: id, starred: true) }
        page = try await calls.run { try $0.item(workspace: nil, id: id) }
        XCTAssertEqual(page.item.when, "2030-01-04T09:00")
        XCTAssertEqual(page.item.duration, 45)
        XCTAssertEqual(page.item.deadline, "2030-01-05")
        XCTAssertEqual(page.item.place?.lat, 1.5)
        XCTAssertEqual(page.item.state, .inProgress)
        XCTAssertNil(page.item.whenSuggested, "a date set answers the suggestion")
        XCTAssertTrue(page.starred)

        // A bare label takes the known place whole.
        let other = try await calls.run {
            try $0.addItem(workspace: nil, list: "inbox", text: "oil hinges", top: true)
        }
        try await calls.run { try $0.setPlace(workspace: nil, id: other, place: Place(label: "shed"), exact: false) }
        let known = try await calls.run { try $0.places(query: "sh", limit: 8) }
        XCTAssertEqual(known.map(\.place.label), ["Shed"])
        XCTAssertEqual(known.first?.count, 2)

        // The reads that span workspaces tag their rows with this one.
        let starred = try await calls.run { try $0.starred() }
        XCTAssertEqual(starred.map(\.id), [id])
        XCTAssertEqual(starred.first?.docId, home)
        XCTAssertEqual(starred.first?.listName, "Chores")
        let agenda = try await calls.run { try $0.agenda(days: 36_500) }
        XCTAssertEqual(agenda.first?.today, true)
        let dated = try XCTUnwrap(agenda.first { $0.day == "2030-01-04" })
        XCTAssertEqual(dated.rows.map(\.id), [id])
        XCTAssertEqual(dated.rows.first?.placedBy, .when)
        XCTAssertEqual(dated.rows.first?.tone, .neutral)
        XCTAssertEqual(dated.rows.first?.docId, home)
        let json = try await calls.run { try $0.agendaJson(days: 36_500) }
        XCTAssertTrue(json.contains("\"day\":\"2030-01-04\""))
        let placed = try await calls.run { try $0.placedItems() }
        XCTAssertEqual(Set(placed.map(\.item.id)), [id, other])
        let hits = try await calls.run { try $0.find(query: "swee", limit: 10) }
        XCTAssertEqual(hits.first?.id, id)
        XCTAssertEqual(hits.first?.kind, .item)
        XCTAssertEqual(hits.first?.state, .inProgress)
        XCTAssertEqual(hits.first?.listName, "Chores")

        // Duplicate under a starred original, starred beside it.
        let copy = try await calls.run { try $0.duplicateItem(workspace: nil, id: id, star: true) }
        var order = try await calls.run { try $0.starred() }
        XCTAssertEqual(order.map(\.id), [id, copy])
        try await calls.run { try $0.moveStarred(id: id, after: copy) }
        order = try await calls.run { try $0.starred() }
        XCTAssertEqual(order.map(\.id), [copy, id])
        try await calls.run { try $0.reorderItem(workspace: nil, id: copy, pos: 1) }
        let rows = try await calls.run { try $0.items(workspace: nil, list: list, includeDone: false) }
        XCTAssertEqual(rows.map(\.id), [copy, id])

        // Done, then the bin.
        try await calls.run { try $0.setDone(workspace: nil, id: copy, done: true) }
        let done = try await calls.run { try $0.done(workspace: nil, limit: 50) }
        XCTAssertEqual(done.map(\.id), [copy])
        let doneCount = try await calls.run { try $0.doneCount(workspace: nil) }
        XCTAssertEqual(doneCount, 1)

        try await calls.run { try $0.setBinned(workspace: nil, id: other, binned: true) }
        try await calls.run { try $0.setBinned(workspace: nil, id: copy, binned: true) }
        var binned = try await calls.run { try $0.binned() }
        XCTAssertEqual(Set(binned.map(\.id)), [other, copy])
        XCTAssertEqual(binned.first?.docId, home)
        try await calls.run { try $0.setBinned(workspace: nil, id: copy, binned: false) }
        try await calls.run { try $0.deleteBinned(workspace: nil, id: other) }
        let removed = try await calls.run { try $0.emptyBin(workspace: nil) }
        XCTAssertEqual(removed, 0)
        binned = try await calls.run { try $0.binned() }
        XCTAssertTrue(binned.isEmpty)

        try await calls.run { try $0.setListArchived(workspace: nil, list: list, archived: true) }
        let archived = try await calls.run { try $0.lists(workspace: nil, filter: .archived) }
        XCTAssertEqual(archived.map(\.id), [list])
    }
}
