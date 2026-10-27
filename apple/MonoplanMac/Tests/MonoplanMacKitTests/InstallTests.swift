import XCTest
@testable import MonoplanMacKit

final class InstallTests: XCTestCase {
    func testBundledBinaryIsTheSiblingNamedMonoplan() throws {
        let dir = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: dir) }
        let app = dir.appendingPathComponent("MonoplanMac")
        XCTAssertNil(bundledBinary(executable: app))
        let bin = dir.appendingPathComponent("monoplan")
        FileManager.default.createFile(atPath: bin.path, contents: Data(), attributes: [.posixPermissions: 0o755])
        XCTAssertEqual(bundledBinary(executable: app), bin.path)
    }

    func testCommandLineLinkStates() throws {
        let dir = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: dir) }
        let target = dir.appendingPathComponent("Monoplan.app/Contents/MacOS/monoplan").path
        let link = CommandLineLink(link: dir.appendingPathComponent("bin/monoplan").path, target: target)
        XCTAssertEqual(link.state, .missing)

        try FileManager.default.createDirectory(atPath: (link.link as NSString).deletingLastPathComponent, withIntermediateDirectories: true)
        try FileManager.default.createSymbolicLink(atPath: link.link, withDestinationPath: "/elsewhere/monoplan")
        XCTAssertEqual(link.state, .other("/elsewhere/monoplan"))

        // A stale symlink is ours to replace (no admin needed in a temp dir).
        try link.install()
        XCTAssertEqual(link.state, .installed)

        // A regular file is not.
        try FileManager.default.removeItem(atPath: link.link)
        FileManager.default.createFile(atPath: link.link, contents: Data())
        XCTAssertEqual(link.state, .other(link.link))
        XCTAssertThrowsError(try link.install())
    }
}
