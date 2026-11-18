import XCTest
@testable import MonoplanMac

/// The end-date arithmetic, as the web's span helpers in `format.tsx`
/// (`spanEndOffset`, `allDayDurationFor`, `durationBetween` under
/// `isValidEndDayOffset`, `sameDayDuration`).
final class DateOffersTests: XCTestCase {
    private let day = 1440

    func testEndOffsetAllDayCountsWholeDays() {
        XCTAssertEqual(DateOffers.endOffset(when: "2026-11-18", duration: nil), 0)
        XCTAssertEqual(DateOffers.endOffset(when: "2026-11-18", duration: day), 0)
        XCTAssertEqual(DateOffers.endOffset(when: "2026-11-18", duration: 3 * day), 2)
    }

    func testEndOffsetTimedIsTheDayTheEndFallsOn() {
        XCTAssertEqual(DateOffers.endOffset(when: "2026-11-18T09:00", duration: nil), 0)
        XCTAssertEqual(DateOffers.endOffset(when: "2026-11-18T09:00", duration: 60), 0)
        XCTAssertEqual(DateOffers.endOffset(when: "2026-11-18T23:30", duration: 60), 1)
        XCTAssertEqual(DateOffers.endOffset(when: "2026-11-18T09:00", duration: 2 * day + 60), 2)
    }

    func testEndLengthAllDay() {
        XCTAssertNil(DateOffers.endLength(when: "2026-11-18", duration: nil, offset: -1))
        XCTAssertEqual(DateOffers.endLength(when: "2026-11-18", duration: 3 * day, offset: 0), 0)
        XCTAssertEqual(DateOffers.endLength(when: "2026-11-18", duration: nil, offset: 2), 3 * day)
        XCTAssertEqual(DateOffers.endLength(when: "2026-11-18", duration: nil, offset: 364), 365 * day)
        XCTAssertNil(DateOffers.endLength(when: "2026-11-18", duration: nil, offset: 365))
    }

    func testEndLengthTimedKeepsTheEndClockTime() {
        // 09:00 to 10:30, moved two days on.
        XCTAssertEqual(DateOffers.endLength(when: "2026-11-18T09:00", duration: 90, offset: 2), 2 * day + 90)
        // Back onto the start day.
        XCTAssertEqual(DateOffers.endLength(when: "2026-11-18T09:00", duration: 2 * day + 90, offset: 0), 90)
        // 22:00 to 01:00: the start day cannot hold that end.
        XCTAssertNil(DateOffers.endLength(when: "2026-11-18T22:00", duration: 180, offset: 0))
        XCTAssertEqual(DateOffers.endLength(when: "2026-11-18T22:00", duration: 180, offset: 2), day + 180)
        // No length stored: an hour after the start.
        XCTAssertEqual(DateOffers.endLength(when: "2026-11-18T09:00", duration: nil, offset: 1), day + 60)
    }

    func testSameDayLength() {
        XCTAssertEqual(DateOffers.sameDayLength(when: "2026-11-18", duration: 3 * day), 0)
        XCTAssertEqual(DateOffers.sameDayLength(when: "2026-11-18T09:00", duration: 90), 90)
        XCTAssertEqual(DateOffers.sameDayLength(when: "2026-11-18T09:00", duration: 2 * day + 90), 90)
        // The end's clock time is before the start's: the default hour.
        XCTAssertEqual(DateOffers.sameDayLength(when: "2026-11-18T22:00", duration: day + 180), 60)
        XCTAssertEqual(DateOffers.sameDayLength(when: "2026-11-18T23:30", duration: day + 180), 29)
    }

    func testDaysBetween() {
        XCTAssertEqual(DateOffers.days(from: "2026-11-18", to: "2026-11-18"), 0)
        XCTAssertEqual(DateOffers.days(from: "2026-11-18", to: "2026-12-02"), 14)
        XCTAssertEqual(DateOffers.days(from: "2026-11-18", to: "2026-11-17"), -1)
    }
}
