import XCTest
@testable import MonoplanMacKit

/// A Calendar row dropped on a day, as the web's `dropOnDay`
/// (`js/web/src/dayGroups.ts`).
final class AgendaDropTests: XCTestCase {
    private func write(
        when: String? = nil, deadline: String? = nil, closedTask: Bool = false, day: String = "2026-11-25"
    ) -> AgendaDrop? {
        AgendaDrop.write(when: when, deadline: deadline, closedTask: closedTask, day: day)
    }

    func testAllDayWhenMovesToTheDay() {
        XCTAssertEqual(write(when: "2026-11-23"), .when("2026-11-25"))
    }

    func testTimedWhenKeepsItsClock() {
        XCTAssertEqual(write(when: "2026-11-23T09:30"), .when("2026-11-25T09:30"))
    }

    func testWhenMovesAndTheDeadlineStays() {
        XCTAssertEqual(write(when: "2026-11-23", deadline: "2026-11-30"), .when("2026-11-25"))
        // Grabbed by its deadline's row, on the day the `when` is not.
        XCTAssertEqual(write(when: "2026-11-23", deadline: "2026-11-25"), .when("2026-11-25"))
    }

    func testDeadlineAloneMoves() {
        XCTAssertEqual(write(deadline: "2026-11-23"), .deadline("2026-11-25"))
    }

    func testTheSameDayWritesNothing() {
        XCTAssertNil(write(when: "2026-11-25"))
        XCTAssertNil(write(when: "2026-11-25T18:00"))
        XCTAssertNil(write(deadline: "2026-11-25"))
        XCTAssertNil(write(when: "2026-11-25", deadline: "2026-11-23"))
    }

    func testAClosedTaskStaysPut() {
        XCTAssertNil(write(when: "2026-11-23", closedTask: true))
        XCTAssertNil(write(deadline: "2026-11-23", closedTask: true))
    }

    func testNoDateWritesNothing() {
        XCTAssertNil(write())
    }

    func testOnlyADayIsATarget() {
        XCTAssertNil(write(when: "2026-11-23", day: "overdue"))
    }
}
