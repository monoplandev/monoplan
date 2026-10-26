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
        XCTAssertEqual(
            Request.agenda(days: 0).value,
            .map(["Doc": .map(["Agenda": .map(["days": .int(0), "today": .nil, "json": .bool(true)])])])
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
        XCTAssertEqual(try Reply.parse(.string("ShuttingDown")), .shuttingDown)
        XCTAssertThrowsError(try Reply.parse(.map(["Nope": .nil])))
    }

    func testOfflineNotice() {
        XCTAssertNil(SyncStatus(online: true, lastSyncedAt: nil).offlineNotice())
        XCTAssertEqual(SyncStatus(online: false, lastSyncedAt: nil).offlineNotice(), "Offline · never synced")
        let now = Date(timeIntervalSince1970: 1_000 + 3 * 60)
        XCTAssertEqual(
            SyncStatus(online: false, lastSyncedAt: 1_000_000).offlineNotice(now: now),
            "Offline · last synced 3m ago"
        )
    }
}
