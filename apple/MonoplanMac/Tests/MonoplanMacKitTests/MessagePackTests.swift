import XCTest
@testable import MonoplanMacKit

final class MessagePackTests: XCTestCase {
    /// The same bytes `cli/src/ipc.rs` pins for `rmp_serde::to_vec_named`.
    func testEncodesTheShapesRmpSerdeNamedProduces() {
        XCTAssertEqual(
            MessagePack.encode(.string("Watch")),
            Data([0xa5] + Array("Watch".utf8))
        )
        XCTAssertEqual(
            MessagePack.encode(.map(["Hello": .map(["build_id": .string("x")])])),
            Data([0x81, 0xa5] + Array("Hello".utf8) + [0x81, 0xa8] + Array("build_id".utf8) + [0xa1] + Array("x".utf8))
        )
    }

    func testRoundTripsEveryShapeTheProtocolUses() throws {
        let value: MPValue = .map([
            "Doc": .map([
                "Agenda": .map(["days": .int(0), "today": .nil, "json": .bool(true)])
            ]),
            "big": .int(1_700_000_000_000),
            "neg": .int(-1234),
            "small_neg": .int(-5),
            "u8": .int(200),
            "u16": .int(60_000),
            "f": .double(1.5),
            "bin": .binary(Data([1, 2, 3])),
            "arr": .array([.string(String(repeating: "x", count: 40)), .int(7)]),
        ])
        let bytes = MessagePack.encode(value)
        XCTAssertEqual(try MessagePack.decode(bytes), value)
    }

    func testDecodesWhatTheDaemonSends() throws {
        // {"Event": {"Sync": {"online": false, "last_synced_at": 7}}}
        var bytes: [UInt8] = [0x81, 0xa5] + Array("Event".utf8)
        bytes += [0x81, 0xa4] + Array("Sync".utf8)
        bytes += [0x82, 0xa6] + Array("online".utf8) + [0xc2]
        bytes += [0xae] + Array("last_synced_at".utf8) + [0x07]
        let value = try MessagePack.decode(Data(bytes))
        XCTAssertEqual(value["Event"]?["Sync"]?["online"]?.boolValue, false)
        XCTAssertEqual(value["Event"]?["Sync"]?["last_synced_at"]?.intValue, 7)
        // A unit variant is a bare string.
        XCTAssertEqual(try MessagePack.decode(Data([0xa7] + Array("Changed".utf8))), .string("Changed"))
    }

    func testRejectsGarbage() {
        XCTAssertThrowsError(try MessagePack.decode(Data([0x81, 0xa5])))
        XCTAssertThrowsError(try MessagePack.decode(Data([0xc0, 0xc0])))
        XCTAssertThrowsError(try MessagePack.decode(Data([0x81, 0x01, 0x01])))
    }
}
