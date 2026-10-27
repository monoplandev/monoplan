import XCTest
@testable import MonoplanCore

/// The embedded host's start-up contract, without a server: the two
/// refusals the app branches on. A full host run, and the takeover of
/// a headless daemon, need an account and live in `cli/tests/daemon.rs`.
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

    func testHeldLockWithNoHostIsBusy() throws {
        let dir = try tempDir()
        defer { try? FileManager.default.removeItem(atPath: dir) }
        // Signed in by the CLI's rule: secrets.toml exists.
        FileManager.default.createFile(atPath: dir + "/secrets.toml", contents: Data())
        // Someone holds daemon.lock with no socket to ask: a host still
        // starting, or a CLI command rewriting the profile.
        let fd = open(dir + "/daemon.lock", O_RDWR | O_CREAT, 0o644)
        XCTAssertGreaterThanOrEqual(fd, 0)
        defer { close(fd) }
        XCTAssertEqual(flock(fd, LOCK_EX | LOCK_NB), 0)
        XCTAssertThrowsError(try MonoplanCore.Host.start(profileDir: dir)) { error in
            XCTAssertEqual(error as? HostError, .Busy)
        }
    }
}
