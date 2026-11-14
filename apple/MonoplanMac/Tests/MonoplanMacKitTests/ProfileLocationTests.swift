import XCTest
@testable import MonoplanMacKit

final class ProfileLocationTests: XCTestCase {
    func testDataDirOverrideIsTakenVerbatim() {
        XCTAssertEqual(ProfileLocation.resolve(environment: ["MONOPLAN_DATA_DIR": "/x/y"]).dir.path, "/x/y")
        XCTAssertTrue(ProfileLocation.resolve(environment: [:]).dir.path.hasSuffix("/Library/Application Support/monoplan"))
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
}
