import XCTest
@testable import MonoplanCore

/// The embedded host's start-up contract, without a signed-in profile:
/// the two refusals the app branches on. A full host run needs an
/// account and a server and lives in `cli/tests/daemon.rs`.
final class HostTests: XCTestCase {
    private func tempDir() throws -> String {
        let dir = (NSTemporaryDirectory() as NSString)
            .appendingPathComponent("monoplan-host-\(UUID().uuidString)")
        try FileManager.default.createDirectory(atPath: dir, withIntermediateDirectories: true)
        return dir
    }

    func testNoProfileIsNotLoggedIn() throws {
        let dir = try tempDir()
        defer { try? FileManager.default.removeItem(atPath: dir) }
        XCTAssertThrowsError(try MonoplanCore.Host.start(profileDir: dir)) { error in
            XCTAssertEqual(error as? HostError, .NotLoggedIn)
        }
    }

    func testHeldLockIsAlreadyRunning() throws {
        let dir = try tempDir()
        defer { try? FileManager.default.removeItem(atPath: dir) }
        // Signed in by the CLI's rule: secrets.toml exists.
        FileManager.default.createFile(atPath: dir + "/secrets.toml", contents: Data())
        // Someone holds daemon.lock (what a running daemon does).
        let fd = open(dir + "/daemon.lock", O_RDWR | O_CREAT, 0o644)
        XCTAssertGreaterThanOrEqual(fd, 0)
        defer { close(fd) }
        XCTAssertEqual(flock(fd, LOCK_EX | LOCK_NB), 0)
        XCTAssertThrowsError(try MonoplanCore.Host.start(profileDir: dir)) { error in
            XCTAssertEqual(error as? HostError, .AlreadyRunning)
        }
    }
}
