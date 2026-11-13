import XCTest
@testable import MonoplanMacKit

// `NSDataDetector` resolves against the real clock, so expectations are
// built from the present rather than a pinned date.
final class WhenPhraseTests: XCTestCase {
    private let now = Date()

    private var calendar: Calendar {
        var calendar = Calendar(identifier: .gregorian)
        calendar.timeZone = .current
        return calendar
    }

    private func day(plus days: Int) -> String {
        let date = calendar.date(byAdding: .day, value: days, to: now)!
        let c = calendar.dateComponents([.year, .month, .day], from: date)
        return String(format: "%04d-%02d-%02d", c.year!, c.month!, c.day!)
    }

    private func when(_ text: String) -> String? { WhenPhrase.find(in: text, now: now)?.when }

    func testRelativeDaysAreAllDay() {
        XCTAssertEqual(when("call mum tomorrow"), day(plus: 1))
        XCTAssertEqual(when("pay rent in 3 days"), day(plus: 3))
        XCTAssertEqual(when("llamar mañana"), day(plus: 1))
        XCTAssertEqual(when("ship today"), day(plus: 0))
    }

    func testAClockTimeIsTimed() {
        XCTAssertEqual(when("call mum tomorrow at 5pm"), "\(day(plus: 1))T17:00")
        XCTAssertEqual(when("standup tomorrow 9:30am"), "\(day(plus: 1))T09:30")
        // Noon said out loud is a time, not the all-day placeholder.
        XCTAssertEqual(when("lunch at noon tomorrow"), "\(day(plus: 1))T12:00")
    }

    func testTimeOfDayWordsTakeOurHours() {
        XCTAssertEqual(when("run tomorrow morning"), "\(day(plus: 1))T09:00")
        XCTAssertEqual(when("nap tomorrow afternoon"), "\(day(plus: 1))T15:00")
        XCTAssertEqual(when("drinks tomorrow evening"), "\(day(plus: 1))T18:00")
        XCTAssertEqual(when("movie tomorrow night"), "\(day(plus: 1))T20:00")
        XCTAssertEqual(when("cena mañana por la noche"), "\(day(plus: 1))T20:00")
        // A clock time beside the word is the time.
        XCTAssertEqual(when("run tomorrow morning at 6:15"), "\(day(plus: 1))T06:15")
    }

    func testThePhraseIsTheWordsAsTyped() {
        let phrase = WhenPhrase.find(in: "Call mum  Tomorrow at 5pm", now: now)
        XCTAssertEqual(phrase?.text, "Tomorrow at 5pm")
        XCTAssertEqual(phrase?.key, "tomorrow at 5pm")
    }

    func testTheLastPhraseWins() {
        XCTAssertEqual(when("email today then call tomorrow"), day(plus: 1))
        // "from … to …" is one phrase to the detector, a span: its start.
        XCTAssertEqual(when("away from today to tomorrow"), day(plus: 0))
    }

    func testATimeAloneIsItsNextOccurrence() {
        let clock = calendar.dateComponents([.hour, .minute], from: now)
        let minutes = clock.hour! * 60 + clock.minute!
        XCTAssertEqual(when("gym 3pm"), "\(day(plus: 15 * 60 >= minutes ? 0 : 1))T15:00")
        XCTAssertEqual(when("gym 9am"), "\(day(plus: 9 * 60 >= minutes ? 0 : 1))T09:00")
    }

    func testDaysGoneAreNoPlan() {
        XCTAssertNil(when("what happened yesterday"))
        XCTAssertNil(when("notes from last friday"))
    }

    func testTitleWordsAreNotDates() {
        for title in [
            "", "   ", "sat on the couch", "sun cream", "march through the park", "may need to call bob",
            "frida kahlo exhibition", "wait 3 days", "book flights next month", "chapter 12", "version 3.2",
            "second draft", "call 555 1234",
        ] {
            XCTAssertNil(when(title), title)
        }
    }

    // MARK: The suggestion rule

    private func edit(_ title: String, was: String, when: String? = nil, pending: String? = nil, closed: Bool = false) -> SuggestionEdit {
        WhenSuggest.edit(title: title, was: was, when: when, pending: pending, closed: closed, now: now)
    }

    func testANewPhraseRaisesASuggestion() {
        XCTAssertEqual(edit("dentist tomorrow", was: "dentist"), .set(day(plus: 1)))
        // A changed wording asks again.
        XCTAssertEqual(edit("dentist tomorrow night", was: "dentist tomorrow"), .set("\(day(plus: 1))T20:00"))
    }

    func testTheSameWordingIsNotAskedAgain() {
        // Declined ("no" leaves nothing pending), then a typo fixed.
        XCTAssertEqual(edit("dentist tomorrow", was: "dentst tomorrow"), .keep)
        XCTAssertEqual(edit("Dentist  Tomorrow", was: "dentist tomorrow", pending: day(plus: 1)), .keep)
    }

    func testAPhraseEditedOutTakesItsSuggestionAlong() {
        XCTAssertEqual(edit("dentist", was: "dentist tomorrow", pending: day(plus: 1)), .clear)
        // Nothing pending, nothing to clear.
        XCTAssertEqual(edit("dentist", was: "dentist tomorrow"), .keep)
        XCTAssertEqual(edit("dentist", was: "dentis", pending: nil), .keep)
    }

    func testADatedOrClosedItemTakesNone() {
        XCTAssertEqual(edit("dentist tomorrow", was: "dentist", when: "2026-11-20"), .keep)
        XCTAssertEqual(edit("dentist tomorrow", was: "dentist", closed: true), .keep)
    }
}
