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
            Request.focus.value,
            .map(["Doc": .map(["cmd": .map(["Focus": .map(["cmd": .nil, "json": .bool(true)])])])])
        )
        XCTAssertEqual(
            Request.lists.value,
            .map(["Doc": .map(["cmd": .map(["Lists": .map(["cmd": .map(["Ls": .map([
                "json": .bool(true), "archived": .bool(false), "all": .bool(true),
            ])])])])])])
        )
        XCTAssertEqual(
            Request.agenda(days: 0).value,
            .map(["Doc": .map(["cmd": .map(["Agenda": .map(["days": .int(0), "today": .nil, "json": .bool(true)])])])])
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
            try Reply.parse(.map(["Event": .map(["Sync": sync])])),
            .event(.sync(SyncStatus(online: true, lastSyncedAt: 99)))
        )
        XCTAssertEqual(try Reply.parse(.map(["Err": .map(["message": .string("no")])])), .error("no"))
        XCTAssertEqual(
            try Reply.parse(.map(["Account": .map([
                "account_id": .string("a"), "email": .string("e"), "device_id": .string("d"),
            ])])),
            .account(accountId: "a", email: "e", deviceId: "d")
        )
        XCTAssertEqual(try Reply.parse(.string("ShuttingDown")), .shuttingDown)
        XCTAssertThrowsError(try Reply.parse(.map(["Nope": .nil])))
    }

    /// An in-memory host: answers hello and doc requests with canned
    /// frames, feeds the watch, and can "stop" like a loop ending.
    final class FakeTransport: HostTransport {
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
        client.hostProvider = { .transport(transport) }

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

        // An `Err` reply is a failure to the caller, not a dropped host.
        transport.replies = [.map(["Err": .map(["message": .string("nope")])])]
        queue.sync { client.send(.lists) { got = $0 } }
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
        client.hostProvider = { asked += 1; return .pending("starting again") }
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

        // A poke asks straight away and reconnects when a host is back.
        let second = FakeTransport()
        client.hostProvider = { .transport(second) }
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
        XCTAssertFalse(ProfileLocation(dir: URL(fileURLWithPath: "/nonexistent/monoplan")).isSignedIn)
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
    }
}
