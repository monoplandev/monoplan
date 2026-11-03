import XCTest
@testable import MonoplanMacKit

final class DevicesTests: XCTestCase {
    func testParsesTheListReply() throws {
        let json = """
        {"devices":[{"id":"d1","name":"Mac","kind":"macos","last_seen_at":1700000000000,"last_seen_ip":"1.2.3.4","created_at":1},
                    {"id":"d2","name":"Phone","kind":"ios","last_seen_at":5,"last_seen_ip":null,"created_at":2}]}
        """
        let rows = try Devices.parse(Data(json.utf8))
        XCTAssertEqual(rows, [
            DeviceRow(id: "d1", name: "Mac", kind: "macos", lastSeenAt: 1_700_000_000_000, lastSeenIp: "1.2.3.4"),
            DeviceRow(id: "d2", name: "Phone", kind: "ios", lastSeenAt: 5, lastSeenIp: nil),
        ])
    }

    func testRequestShapes() throws {
        let list = try XCTUnwrap(Devices.list(server: "https://x.test", token: "tok"))
        XCTAssertEqual(list.httpMethod, "GET")
        XCTAssertEqual(list.url?.absoluteString, "https://x.test/api/devices")
        XCTAssertEqual(list.value(forHTTPHeaderField: "Authorization"), "Bearer tok")
        XCTAssertNil(list.httpBody)

        let rename = try XCTUnwrap(Devices.rename(server: "https://x.test", token: "tok", id: "a b", name: "Desk"))
        XCTAssertEqual(rename.httpMethod, "PATCH")
        XCTAssertEqual(rename.url?.absoluteString, "https://x.test/api/devices/a%20b")
        XCTAssertEqual(rename.value(forHTTPHeaderField: "Content-Type"), "application/json")
        XCTAssertEqual(rename.httpBody.map { String(decoding: $0, as: UTF8.self) }, "{\"name\":\"Desk\"}")

        let revoke = try XCTUnwrap(Devices.revoke(server: "https://x.test", token: "tok", id: "d2"))
        XCTAssertEqual(revoke.httpMethod, "DELETE")
        XCTAssertEqual(revoke.url?.absoluteString, "https://x.test/api/devices/d2")

        XCTAssertNil(Devices.list(server: "not a url", token: "tok"))
    }

    func testErrorMessageAndSymbols() {
        XCTAssertEqual(Devices.errorMessage(status: 404, body: Data("{\"code\":\"not_found\",\"message\":\"no such device\"}".utf8)), "no such device")
        XCTAssertEqual(Devices.errorMessage(status: 502, body: Data("<html>".utf8)), "HTTP 502")
        XCTAssertEqual(Devices.symbol(forKind: "web"), "globe")
        XCTAssertEqual(Devices.symbol(forKind: "macos"), "laptopcomputer")
        XCTAssertEqual(Devices.symbol(forKind: "toaster"), "questionmark.circle")
    }
}
