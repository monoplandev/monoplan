import XCTest
@testable import MonoplanMacKit

final class DaemonClientTests: XCTestCase {
    func testSocketPathMatchesTheCli() {
        // Same vector as `long_profile_paths_fall_back_to_temp` in cli/src/ipc.rs.
        XCTAssertEqual(ProfileLocation.fnv1a(Array("/tmp/p".utf8)), 0x6cc1_2add_f274_5004)
        XCTAssertEqual(ProfileLocation(dir: URL(fileURLWithPath: "/tmp/p")).socketPath(), "/tmp/p/daemon.sock")
        let long = ProfileLocation(dir: URL(fileURLWithPath: "/" + String(repeating: "x", count: 150)))
        XCTAssertEqual(
            long.socketPath(tempDir: "/var/tmp/"),
            "/var/tmp/monoplan-\(String(format: "%016llx", ProfileLocation.fnv1a(Array(long.dir.path.utf8)))).sock"
        )
        XCTAssertTrue(long.socketPath(tempDir: "/var/tmp").hasPrefix("/var/tmp/monoplan-"))
    }

    func testDataDirOverrideIsTakenVerbatim() {
        XCTAssertEqual(ProfileLocation.resolve(environment: ["MONOPLAN_DATA_DIR": "/x/y"]).dir.path, "/x/y")
        XCTAssertTrue(ProfileLocation.resolve(environment: [:]).dir.path.hasSuffix("/Library/Application Support/monoplan"))
    }

    func testRequestShapes() {
        XCTAssertEqual(Request.watch.value, .string("Watch"))
        XCTAssertEqual(Request.account.value, .string("Account"))
        XCTAssertEqual(
            Request.focus().value,
            .map(["Doc": .map(["cmd": .map(["Focus": .map(["cmd": .nil, "json": .bool(true)])])])])
        )
        XCTAssertEqual(
            Request.focus(all: true).value,
            .map(["Doc": .map(["cmd": .map(["Focus": .map(["cmd": .nil, "json": .bool(true), "all": .bool(true)])])])])
        )
        XCTAssertEqual(
            Request.items(list: "inbox").value,
            .map(["Doc": .map(["cmd": .map(["Ls": .map([
                "list": .string("inbox"), "done": .bool(false), "json": .bool(true),
            ])])])])
        )
        XCTAssertEqual(
            Request.item(id: "i1", workspace: "d1").value,
            .map(["Doc": .map(["workspace": .string("d1"), "cmd": .map(["Show": .map([
                "item_id": .string("i1"), "json": .bool(true),
            ])])])])
        )
        XCTAssertEqual(
            Request.setDone(id: "i1", done: true).value,
            .map(["Doc": .map(["cmd": .map(["Set": .map([
                "item_id": .string("i1"), "lifecycle": .string("Done"),
                "text": .nil, "list": .nil, "when": .nil, "duration": .nil, "deadline": .nil,
                "place": .nil, "coords": .nil, "address": .nil, "new_place": .bool(false),
                "star": .nil, "unstar": .bool(false),
            ])])])])
        )
        XCTAssertEqual(
            Request.setLifecycle(id: "i1", state: "in_progress", workspace: "d1").value,
            .map(["Doc": .map(["workspace": .string("d1"), "cmd": .map(["Set": .map([
                "item_id": .string("i1"), "lifecycle": .string("InProgress"),
                "text": .nil, "list": .nil, "when": .nil, "duration": .nil, "deadline": .nil,
                "place": .nil, "coords": .nil, "address": .nil, "new_place": .bool(false),
                "star": .nil, "unstar": .bool(false),
            ])])])])
        )
        XCTAssertEqual(
            Request.setBinned(id: "i1", binned: true, workspace: "d1").value,
            .map(["Doc": .map(["workspace": .string("d1"), "cmd": .map(["ItemBin": .map(["item_id": .string("i1")])])])])
        )
        XCTAssertEqual(
            Request.duplicateItem(id: "i1", workspace: "d1").value,
            .map(["Doc": .map(["workspace": .string("d1"), "cmd": .map(["Duplicate": .map(["item_id": .string("i1")])])])])
        )
        XCTAssertEqual(
            Request.duplicateItem(id: "i1", star: true).value,
            .map(["Doc": .map(["cmd": .map(["Duplicate": .map(["item_id": .string("i1"), "star": .bool(true)])])])])
        )
        XCTAssertEqual(
            Request.setBinned(id: "i1", binned: false).value,
            .map(["Doc": .map(["cmd": .map(["Restore": .map(["item_id": .string("i1")])])])])
        )
        XCTAssertEqual(
            Request.deleteBinned(id: "i1").value,
            .map(["Doc": .map(["cmd": .map(["Bin": .map([
                "sub": .map(["Rm": .map(["item_id": .string("i1")])]), "json": .bool(false),
            ])])])])
        )
        XCTAssertEqual(
            Request.addItem(text: "Call the bank", list: "inbox", workspace: "d1").value,
            .map(["Doc": .map(["workspace": .string("d1"), "cmd": .map(["Add": .map([
                "list": .string("inbox"), "texts": .array([.string("Call the bank")]),
            ])])])])
        )
        XCTAssertEqual(
            Request.addItem(text: "Call the bank", list: "inbox", top: true).value,
            .map(["Doc": .map(["cmd": .map(["Add": .map([
                "list": .string("inbox"), "texts": .array([.string("Call the bank")]), "top": .bool(true),
            ])])])])
        )
        XCTAssertEqual(
            Request.setText(id: "i1", text: "Call the bank").value,
            .map(["Doc": .map(["cmd": .map(["Set": .map([
                "item_id": .string("i1"), "text": .string("Call the bank"),
                "list": .nil, "lifecycle": .nil, "when": .nil, "duration": .nil, "deadline": .nil,
                "place": .nil, "coords": .nil, "address": .nil, "new_place": .bool(false),
                "star": .nil, "unstar": .bool(false),
            ])])])])
        )
        XCTAssertEqual(
            Request.moveItem(id: "i1", list: "l2", workspace: "d1").value,
            .map(["Doc": .map(["workspace": .string("d1"), "cmd": .map(["Set": .map([
                "item_id": .string("i1"), "list": .string("l2"), "lifecycle": .nil,
                "text": .nil, "when": .nil, "duration": .nil, "deadline": .nil,
                "place": .nil, "coords": .nil, "address": .nil, "new_place": .bool(false),
                "star": .nil, "unstar": .bool(false),
            ])])])])
        )
        func setArgs(_ request: Request) -> [String: MPValue]? {
            guard case .map(let frame) = request.value, case .map(let doc)? = frame["Doc"],
                  case .map(let cmd)? = doc["cmd"], case .map(let args)? = cmd["Set"] else { return nil }
            return args
        }
        let starred = setArgs(.setStarred(id: "i1", starred: true))
        XCTAssertEqual(starred?["star"], .map(["At": .int(1)]))
        XCTAssertEqual(starred?["unstar"], .bool(false))
        let unstarred = setArgs(.setStarred(id: "i1", starred: false))
        XCTAssertEqual(unstarred?["star"], .nil)
        XCTAssertEqual(unstarred?["unstar"], .bool(true))
        // A plain retitle carries no `when_suggested` key at all.
        XCTAssertNil(setArgs(.setText(id: "i1", text: "x"))?["when_suggested"])
        let raised = setArgs(.setText(id: "i1", text: "dentist friday", suggestion: .set("2026-11-20")))
        XCTAssertEqual(raised?["text"], .string("dentist friday"))
        XCTAssertEqual(raised?["when_suggested"], .string("2026-11-20"))
        XCTAssertEqual(setArgs(.setText(id: "i1", text: "dentist", suggestion: .clear))?["when_suggested"], .string("-"))
        XCTAssertEqual(setArgs(.setWhenSuggested(id: "i1", when: "2026-11-20T08:30"))?["when_suggested"], .string("2026-11-20T08:30"))
        XCTAssertEqual(setArgs(.setWhenSuggested(id: "i1", when: nil))?["when_suggested"], .string("-"))
        let timed = setArgs(.setWhen(id: "i1", when: "2026-09-12T14:00", duration: .minutes(90)))
        XCTAssertEqual(timed?["when"], .string("2026-09-12T14:00"))
        XCTAssertEqual(timed?["duration"], .string("90"))
        let cleared = setArgs(.setWhen(id: "i1", when: nil))
        XCTAssertEqual(cleared?["when"], .string("-"))
        XCTAssertEqual(cleared?["duration"], .nil)
        XCTAssertEqual(setArgs(.setWhen(id: "i1", when: "2026-09-12", duration: .clear))?["duration"], .string("-"))
        XCTAssertEqual(setArgs(.setDuration(id: "i1", minutes: 45))?["duration"], .string("45"))
        XCTAssertEqual(setArgs(.setDuration(id: "i1", minutes: nil))?["duration"], .string("-"))
        let placed = setArgs(.setPlace(id: "i1", place: PlaceInfo(label: "Gym", lat: 1.5, lon: -2.25, address: "1 High St")))
        XCTAssertEqual(placed?["place"], .string("Gym"))
        XCTAssertEqual(placed?["coords"], .string("1.5,-2.25"))
        XCTAssertEqual(placed?["address"], .string("1 High St"))
        XCTAssertEqual(placed?["new_place"], .bool(false))
        let bare = setArgs(.setPlace(id: "i1", place: PlaceInfo(label: "Gym"), exact: true))
        XCTAssertEqual(bare?["coords"], .nil)
        XCTAssertEqual(bare?["new_place"], .bool(true))
        XCTAssertEqual(setArgs(.setPlace(id: "i1", place: nil))?["place"], .string("-"))
        XCTAssertEqual(
            Request.places(query: "gy", limit: 8).value,
            .map(["Doc": .map(["cmd": .map(["Places": .map([
                "query": .string("gy"), "limit": .int(8), "json": .bool(true),
            ])])])])
        )
        XCTAssertEqual(
            Request.find(query: "buy gro", limit: 50).value,
            .map(["Doc": .map(["cmd": .map(["Find": .map([
                "query": .string("buy gro"), "limit": .int(50), "json": .bool(true),
            ])])])])
        )
        XCTAssertEqual(
            Request.done(limit: 50).value,
            .map(["Doc": .map(["cmd": .map(["Done": .map([
                "limit": .int(50), "since": .nil, "json": .bool(true),
            ])])])])
        )
        XCTAssertEqual(
            Request.doneCount.value,
            .map(["Doc": .map(["cmd": .map(["Done": .map([
                "limit": .nil, "since": .nil, "json": .bool(false), "count": .bool(true),
            ])])])])
        )
        XCTAssertEqual(
            Request.lists().value,
            .map(["Doc": .map(["cmd": .map(["Lists": .map(["cmd": .map(["Ls": .map([
                "json": .bool(true), "archived": .bool(false), "all": .bool(true),
            ])])])])])])
        )
        // A workspace selector rides beside `cmd`, as `-w` does.
        XCTAssertEqual(
            Request.lists(workspace: "d1").value,
            .map(["Doc": .map(["workspace": .string("d1"), "cmd": .map(["Lists": .map(["cmd": .map(["Ls": .map([
                "json": .bool(true), "archived": .bool(false), "all": .bool(true),
            ])])])])])])
        )
        XCTAssertEqual(
            Request.items(list: "work", workspace: "d1").value,
            .map(["Doc": .map(["workspace": .string("d1"), "cmd": .map(["Ls": .map([
                "list": .string("work"), "done": .bool(false), "json": .bool(true),
            ])])])])
        )
        XCTAssertEqual(
            Request.agenda(days: 0).value,
            .map(["Doc": .map(["cmd": .map(["Agenda": .map(["days": .int(0), "today": .nil, "json": .bool(true)])])])])
        )
        XCTAssertEqual(
            Request.agenda(days: 14, all: true).value,
            .map(["Doc": .map(["cmd": .map(["Agenda": .map([
                "days": .int(14), "today": .nil, "json": .bool(true), "all": .bool(true),
            ])])])])
        )
        // The shape cli/src/ipc.rs `wire_shape_is_named_maps_and_bare_unit_variants` pins.
        XCTAssertEqual(
            Request.createWorkspace(name: "Work").value,
            .map(["CreateWorkspace": .map(["name": .string("Work")])])
        )
        XCTAssertEqual(
            Request.deleteWorkspace(docId: "d").value,
            .map(["DeleteWorkspace": .map(["workspace": .string("d")])])
        )
    }

    func testReplyParsing() throws {
        let sync: MPValue = .map(["online": .bool(true), "last_synced_at": .int(99)])
        XCTAssertEqual(
            try Reply.parse(.map(["Hello": .map(["build_id": .string("b"), "pid": .int(4), "sync": sync])])),
            .hello(buildId: "b", pid: 4, sync: SyncStatus(online: true, lastSyncedAt: 99))
        )
        XCTAssertEqual(try Reply.parse(.map(["Event": .string("Changed")])), .event(.changed))
        XCTAssertEqual(
            try Reply.parse(.map(["WorkspaceCreated": .map(["doc_id": .string("d"), "name": .string("Work"), "sync": sync])])),
            .workspaceCreated(docId: "d", name: "Work", sync: SyncStatus(online: true, lastSyncedAt: 99))
        )
        XCTAssertEqual(
            try Reply.parse(.map(["WorkspaceDeleted": .map(["doc_id": .string("d"), "sync": sync])])),
            .workspaceDeleted(docId: "d", sync: SyncStatus(online: true, lastSyncedAt: 99))
        )
        XCTAssertEqual(
            try Reply.parse(.map(["Event": .map(["Sync": sync])])),
            .event(.sync(SyncStatus(online: true, lastSyncedAt: 99)))
        )
        XCTAssertEqual(try Reply.parse(.map(["Err": .map(["message": .string("no")])])), .error("no"))
        // `pending` and `Synced` (the `Reconnect` reply) are newer than
        // the fields above; absent `pending` reads as false.
        let busy: MPValue = .map(["online": .bool(true), "last_synced_at": .int(99), "pending": .bool(true)])
        XCTAssertEqual(
            try Reply.parse(.map(["Synced": .map(["sync": busy])])),
            .synced(sync: SyncStatus(online: true, lastSyncedAt: 99, pending: true))
        )
        XCTAssertEqual(Request.reconnect.value, .string("Reconnect"))
    }

    func testSyncStatusLine() {
        let now = Date(timeIntervalSince1970: 1_000_000)
        let at = Int64(1_000_000 - 120) * 1000
        XCTAssertEqual(SyncStatus(online: true, lastSyncedAt: at).level, .synced)
        XCTAssertEqual(SyncStatus(online: true, lastSyncedAt: at).line(now: now), "Synced · 2m ago")
        XCTAssertEqual(SyncStatus(online: true, lastSyncedAt: nil).line(now: now), "Synced")
        XCTAssertEqual(SyncStatus(online: true, lastSyncedAt: at, pending: true).level, .syncing)
        XCTAssertEqual(SyncStatus(online: true, lastSyncedAt: at, pending: true).line(now: now), "Syncing…")
        // Offline with unconfirmed work is still offline: the dot and
        // the line say so, not "syncing".
        let off = SyncStatus(online: false, lastSyncedAt: at, offlineReason: "pong timeout", pending: true)
        XCTAssertEqual(off.level, .offline)
        XCTAssertEqual(off.line(now: now), "Offline · last synced 2m ago · pong timeout")
        XCTAssertNil(SyncStatus(online: true, lastSyncedAt: at).offlineNotice(now: now))
        XCTAssertEqual(
            try Reply.parse(.map(["Account": .map([
                "account_id": .string("a"), "email": .string("e"), "device_id": .string("d"),
            ])])),
            .account(accountId: "a", email: "e", deviceId: "d")
        )
        XCTAssertEqual(try Reply.parse(.string("ShuttingDown")), .shuttingDown)
        XCTAssertThrowsError(try Reply.parse(.map(["Nope": .nil])))
    }

    /// An in-memory engine: answers hello and doc requests with canned
    /// frames, feeds the watch, and can "stop" like a loop ending.
    final class FakeTransport: EngineTransport {
        var requests: [MPValue] = []
        var onFrame: ((Data) -> Void)?
        var onStopped: (() -> Void)?
        var closed = false
        var replies: [MPValue] = []

        func request(_ frame: Data, completion: @escaping (Result<Data, Error>) -> Void) {
            let value = try! MessagePack.decode(frame)
            requests.append(value)
            if case .map(let m) = value, m["Hello"] != nil {
                completion(.success(MessagePack.encode(.map(["Hello": .map([
                    "build_id": .string("b:1"), "pid": .int(7), "embedded": .bool(true),
                    "sync": .map(["online": .bool(false)]),
                ])]))))
            } else if !replies.isEmpty {
                completion(.success(MessagePack.encode(replies.removeFirst())))
            } else {
                completion(.failure(ClientError.daemon("no canned reply")))
            }
        }

        func watch(onFrame: @escaping (Data) -> Void, onStopped: @escaping () -> Void) {
            self.onFrame = onFrame
            self.onStopped = onStopped
            onFrame(MessagePack.encode(.map(["Watching": .map(["sync": .map(["online": .bool(true), "last_synced_at": .int(5)])])])))
        }

        func close() { closed = true }
    }

    /// Drain the client's serial queue so every hop has landed.
    private func settle(_ queue: DispatchQueue) {
        let done = expectation(description: "settled")
        queue.async { done.fulfill() }
        wait(for: [done], timeout: 2)
    }

    func testClientHandshakesWatchesAndRequestsOverATransport() {
        let queue = DispatchQueue(label: "test.client")
        let transport = FakeTransport()
        let client = DaemonClient(queue: queue)
        var states: [DaemonClient.State] = []
        var events: [WatchEvent] = []
        client.onState = { states.append($0) }
        client.onEvent = { events.append($0) }
        client.engineProvider = { .transport(transport) }

        queue.sync { client.start() }
        settle(queue)
        // Hello, then the watch's first frame put us online.
        XCTAssertEqual(client.daemon, DaemonClient.DaemonInfo(buildId: "b:1", pid: 7))
        XCTAssertEqual(client.state, .connected(SyncStatus(online: true, lastSyncedAt: 5)))
        XCTAssertEqual(transport.requests.count, 1)

        // A request gets its own reply and refreshes the sync status.
        transport.replies = [.map(["Output": .map(["stdout": .string("[]"), "sync": .map(["online": .bool(false)])])])]
        var got: Result<Reply, Error>?
        queue.sync { client.send(.agenda(days: 0)) { got = $0 } }
        settle(queue)
        guard case .success(.output(let out, _)) = got else { return XCTFail("\(String(describing: got))") }
        XCTAssertEqual(out, "[]")
        XCTAssertEqual(client.state, .connected(SyncStatus(online: false, lastSyncedAt: nil)))

        // An `Err` reply is a failure to the caller, not a dropped engine.
        transport.replies = [.map(["Err": .map(["message": .string("nope")])])]
        queue.sync { client.send(.lists()) { got = $0 } }
        settle(queue)
        guard case .failure(let e) = got, case ClientError.daemon("nope") = e else {
            return XCTFail("\(String(describing: got))")
        }

        // Watch frames: a change is an event, a sync flip updates state.
        transport.onFrame?(MessagePack.encode(.map(["Event": .string("Changed")])))
        transport.onFrame?(MessagePack.encode(.map(["Event": .map(["Sync": .map(["online": .bool(true)])])])))
        settle(queue)
        XCTAssertEqual(events, [.changed, .sync(SyncStatus(online: true, lastSyncedAt: nil))])
        XCTAssertEqual(client.state, .connected(SyncStatus(online: true, lastSyncedAt: nil)))

        // The loop ends: the transport is closed, the provider is asked
        // again, and what it says is the new state.
        var asked = 0
        client.engineProvider = { asked += 1; return .pending("starting again") }
        transport.onStopped?()
        settle(queue)
        XCTAssertTrue(transport.closed)
        XCTAssertNil(client.daemon)
        XCTAssertEqual(asked, 1)
        XCTAssertEqual(client.state, .connecting("starting again"))
        queue.sync { client.send(.account) { got = $0 } }
        guard case .failure(let e2) = got, case ClientError.notConnected = e2 else {
            return XCTFail("\(String(describing: got))")
        }
        XCTAssertEqual(states.last, .connecting("starting again"))

        // A poke asks straight away and reconnects when an engine is back.
        let second = FakeTransport()
        client.engineProvider = { .transport(second) }
        queue.sync { client.poke() }
        settle(queue)
        XCTAssertEqual(client.state, .connected(SyncStatus(online: true, lastSyncedAt: 5)))
        XCTAssertEqual(second.requests.count, 1)
    }

    func testConfigTomlStringScan() {
        let text = "# comment\nserver_url = \"http://127.0.0.1:8000\"\nother = 1\n"
        XCTAssertEqual(ProfileLocation.tomlString(key: "server_url", in: text), "http://127.0.0.1:8000")
        XCTAssertNil(ProfileLocation.tomlString(key: "missing", in: text))
        XCTAssertNil(ProfileLocation(dir: URL(fileURLWithPath: "/nonexistent/monoplan")).serverURL)
        XCTAssertFalse(ProfileLocation(dir: URL(fileURLWithPath: "/nonexistent/monoplan")).hasProfile)
        XCTAssertFalse(ProfileLocation(dir: URL(fileURLWithPath: "/nonexistent/monoplan")).hasAccount)
    }

    /// A profile is `secrets.toml`; an account is its `device_token`
    /// (a local-only workspace has the file without the key).
    func testProfileAndAccountFromSecrets() throws {
        let dir = URL(fileURLWithPath: NSTemporaryDirectory()).appendingPathComponent("monoplan-loc-\(UUID().uuidString)")
        try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: dir) }
        let location = ProfileLocation(dir: dir)
        let secrets = dir.appendingPathComponent("secrets.toml")
        try "[docs.abc]\ndek = \"00\"\n".write(to: secrets, atomically: true, encoding: .utf8)
        XCTAssertTrue(location.hasProfile)
        XCTAssertFalse(location.hasAccount)
        try "device_token = \"tok\"\n\n[docs.abc]\ndek = \"00\"\n".write(to: secrets, atomically: true, encoding: .utf8)
        XCTAssertTrue(location.hasAccount)
    }

    func testOfflineNotice() {
        XCTAssertNil(SyncStatus(online: true, lastSyncedAt: nil).offlineNotice())
        XCTAssertEqual(SyncStatus(online: false, lastSyncedAt: nil).offlineNotice(), "Offline · never synced")
        XCTAssertEqual(
            SyncStatus(online: false, lastSyncedAt: nil, offlineReason: "connect failed: x").offlineNotice(),
            "Offline · never synced · connect failed: x")
        XCTAssertEqual(
            SyncStatus.from(.map(["online": .bool(false), "offline_reason": .string("x")])),
            SyncStatus(online: false, lastSyncedAt: nil, offlineReason: "x"))
        let now = Date(timeIntervalSince1970: 1_000 + 3 * 60)
        XCTAssertEqual(
            SyncStatus(online: false, lastSyncedAt: 1_000_000).offlineNotice(now: now),
            "Offline · last synced 3m ago"
        )
        // A local-only workspace is never "offline": nothing to sync to.
        let local = SyncStatus.from(.map(["online": .bool(false), "local_only": .bool(true)]))
        XCTAssertEqual(local, SyncStatus(online: false, lastSyncedAt: nil, localOnly: true))
        XCTAssertNil(local?.offlineNotice())
        XCTAssertEqual(local?.level, .synced)
        XCTAssertEqual(local?.line(), "Local workspace · not synced")
    }
}
