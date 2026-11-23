import XCTest
import MonoplanCore
@testable import MonoplanMac

/// A capture's date beside a zone (`ItemCapture.setWhen` / `setZone`,
/// spec/calendar-plan.md "Fixed instants"): it keeps what an item's
/// row would carry, wall time on this device and the zone beside it,
/// and is made with the register value. Tokyo has no clock changes,
/// so the digits round-trip whatever zone the tests run in.
final class CaptureZoneTests: XCTestCase {
    private let tokyo = "Asia/Tokyo"

    private func capture(when: String? = nil) -> ItemCapture {
        var capture = ItemCapture(key: 1, workspace: nil, list: "inbox", returnTo: nil)
        capture.when = when
        return capture
    }

    func testFloatingIsStoredAsWritten() {
        let capture = capture(when: "2030-01-04T09:00")
        XCTAssertNil(capture.zone)
        XCTAssertEqual(capture.storedWhen, "2030-01-04T09:00")
        XCTAssertNil(self.capture().storedWhen)
    }

    func testPickingAZoneKeepsTheDigits() {
        var capture = capture(when: "2030-01-04T09:00")
        capture.setZone(tokyo)
        XCTAssertEqual(capture.zone, tokyo)
        XCTAssertEqual(capture.storedWhen, "2030-01-04T09:00[Asia/Tokyo]")
        // What the panel's rows show: the zone's clock.
        XCTAssertEqual(WhenZone.wall(try XCTUnwrap(capture.when), zone: capture.zone), "2030-01-04T09:00")
    }

    func testAnEditOfTheZoneClockIsKeptInTheZone() {
        var capture = capture(when: "2030-01-04T09:00")
        capture.setZone(tokyo)
        capture.setWhen(WhenZone.fixed("2030-01-05T14:30", zone: tokyo))
        XCTAssertEqual(capture.zone, tokyo)
        XCTAssertEqual(capture.storedWhen, "2030-01-05T14:30[Asia/Tokyo]")
    }

    func testFloatingAgainStaysAtTheTimeTheDeviceShows() {
        var capture = capture(when: "2030-01-04T09:00")
        capture.setZone(tokyo)
        let shown = capture.when
        capture.setZone(nil)
        XCTAssertNil(capture.zone)
        XCTAssertEqual(capture.when, shown)
        XCTAssertEqual(capture.storedWhen, shown)
    }

    func testNoZoneWithoutATimeOrForAnUnknownZone() {
        var allDay = capture(when: "2030-01-04")
        allDay.setZone(tokyo)
        XCTAssertNil(allDay.zone)
        var timed = capture(when: "2030-01-04T09:00")
        timed.setZone("Europe/Atlantis")
        XCTAssertNil(timed.zone)
        XCTAssertEqual(timed.when, "2030-01-04T09:00")
    }

    func testSplitReadsTheZoneOffARegisterValue() {
        XCTAssertEqual(WhenZone.split("2030-01-04T09:00[Asia/Tokyo]").wall, "2030-01-04T09:00")
        XCTAssertEqual(WhenZone.split("2030-01-04T09:00[Asia/Tokyo]").zone, tokyo)
        XCTAssertNil(WhenZone.split("2030-01-04T09:00").zone)
        XCTAssertNil(WhenZone.split("2030-01-04").zone)
        XCTAssertEqual(WhenZone.split("2030-01-04").wall, "2030-01-04")
    }
}
