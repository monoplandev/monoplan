import XCTest
@testable import MonoplanMac

/// The typed-time grammar, case for case with the web's
/// `timeSuggest.test.ts` readings on the 24-hour clock.
final class TimeOffersTests: XCTestCase {
    private func at(_ hour: Int, _ minute: Int = 0) -> Int { hour * 60 + minute }

    func testBlankListIsEveryQuarterHour() {
        let all = TimeOffers.suggestions("")
        XCTAssertEqual(all.count, 96)
        XCTAssertEqual(Array(all.prefix(3)), [at(0), at(0, 15), at(0, 30)])
    }

    func testBlankListAfterAStartRunsRoundTheClock() {
        let all = TimeOffers.suggestions("", after: at(14))
        XCTAssertEqual(all.count, 96)
        XCTAssertEqual(all.first, at(14, 15))
        XCTAssertEqual(all.last, at(14))
    }

    func testAnHourReadsInBothHalvesDaytimeFirst() {
        XCTAssertEqual(TimeOffers.suggestions("1"), [at(13), at(1)])
        XCTAssertEqual(TimeOffers.suggestions("9"), [at(9), at(21)])
        XCTAssertEqual(TimeOffers.suggestions("12"), [at(12), at(0)])
        XCTAssertEqual(TimeOffers.suggestions("13"), [at(13)])
        XCTAssertEqual(TimeOffers.suggestions("0"), [at(0)])
        XCTAssertEqual(TimeOffers.suggestions("24"), [])
    }

    func testMinutes() {
        XCTAssertEqual(TimeOffers.suggestions("1:3"), [at(13, 30), at(1, 30)])
        XCTAssertEqual(TimeOffers.suggestions("1:03"), [at(13, 3), at(1, 3)])
        XCTAssertEqual(TimeOffers.suggestions("130"), [at(13, 30), at(1, 30)])
        XCTAssertEqual(TimeOffers.suggestions("1330"), [at(13, 30)])
        XCTAssertEqual(TimeOffers.suggestions("9.45"), [at(9, 45), at(21, 45)])
        XCTAssertEqual(TimeOffers.suggestions("1:60"), [])
    }

    func testSuffixPicksTheHalf() {
        XCTAssertEqual(TimeOffers.suggestions("1p"), [at(13)])
        XCTAssertEqual(TimeOffers.suggestions("1 AM"), [at(1)])
        XCTAssertEqual(TimeOffers.suggestions("12am"), [at(0)])
        XCTAssertEqual(TimeOffers.suggestions("13a"), [])
        XCTAssertEqual(TimeOffers.suggestions("lunch"), [])
    }

    func testAfterAStartTheSoonerReadingLeads() {
        XCTAssertEqual(TimeOffers.suggestions("3", after: at(14)), [at(15), at(3)])
        XCTAssertEqual(TimeOffers.suggestions("9", after: at(14)), [at(21), at(9)])
    }

    func testNearestIndex() {
        XCTAssertEqual(TimeOffers.nearestIndex(nil), 36)
        XCTAssertEqual(TimeOffers.nearestIndex(at(14, 10)), 56)
        XCTAssertEqual(TimeOffers.nearestIndex(nil, after: at(14)), 3)
        XCTAssertEqual(TimeOffers.nearestIndex(at(15, 30), after: at(14)), 5)
    }

    func testDuration() {
        XCTAssertEqual(TimeOffers.duration(from: at(14), to: at(15, 30)), 90)
        XCTAssertEqual(TimeOffers.duration(from: at(23), to: at(1)), 120)
        XCTAssertEqual(TimeOffers.duration(from: at(9), to: at(9)), 1440)
        XCTAssertEqual(TimeOffers.duration(from: at(14), to: at(16), dayOffset: 2), 2 * 1440 + 120)
    }
}
