import XCTest
@testable import MonoplanMacKit

final class RemindersTests: XCTestCase {
    private var calendar: Calendar = {
        var calendar = Calendar(identifier: .gregorian)
        calendar.timeZone = TimeZone(identifier: "Europe/Madrid")!
        return calendar
    }()

    private func at(_ when: String) -> Date { Agenda.start(of: when, calendar: calendar)! }

    private func source(_ id: String, _ when: String, event: Bool = false) -> ReminderSource {
        ReminderSource(docId: "d", itemId: id, title: id, when: when, event: event)
    }

    private func clock(_ reminder: PlannedReminder) -> String {
        String(format: "%02d:%02d", reminder.fire.hour!, reminder.fire.minute!)
    }

    /// Events ten minutes ahead, tasks at the time: what the other
    /// tests plan with.
    private let on = ReminderSettings(event: .tenMinutes, task: .atTime)

    func testBothKindsAreOffUntilTurnedOn() {
        XCTAssertEqual(ReminderSettings(), ReminderSettings(event: .off, task: .off))
        let plan = Reminders.plan(
            [source("task", "2026-12-01T14:30"), source("event", "2026-12-01T18:00", event: true)],
            settings: ReminderSettings(), now: at("2026-12-01T12:00"), calendar: calendar
        )
        XCTAssertTrue(plan.isEmpty)
    }

    func testAnEventFiresAheadAndATaskAtItsTime() {
        let plan = Reminders.plan(
            [source("task", "2026-12-01T14:30"), source("event", "2026-12-01T18:00", event: true)],
            settings: on, now: at("2026-12-01T12:00"), calendar: calendar
        )
        XCTAssertEqual(plan.map(\.itemId), ["task", "event"])
        XCTAssertEqual(plan.map(clock), ["14:30", "17:50"])
        XCTAssertEqual(plan.map(\.lead), [0, 10])
        XCTAssertNil(plan[0].fire.timeZone)
    }

    func testEachSettingAppliesToItsOwnKind() {
        let sources = [source("task", "2026-12-01T14:30"), source("event", "2026-12-01T18:00", event: true)]
        let now = at("2026-12-01T12:00")
        let tasksEarly = Reminders.plan(
            sources, settings: ReminderSettings(event: .off, task: .tenMinutes), now: now, calendar: calendar
        )
        XCTAssertEqual(tasksEarly.map(\.itemId), ["task"])
        XCTAssertEqual(tasksEarly.map(clock), ["14:20"])
        let eventsOnly = Reminders.plan(
            sources, settings: ReminderSettings(event: .tenMinutes, task: .off), now: now, calendar: calendar
        )
        XCTAssertEqual(eventsOnly.map(\.itemId), ["event"])
        XCTAssertTrue(
            Reminders.plan(sources, settings: ReminderSettings(event: .off, task: .off), now: now, calendar: calendar)
                .isEmpty
        )
    }

    func testAFiringTimeGoneByIsLeftOutAndAllDayNeverFires() {
        let plan = Reminders.plan(
            [
                // Starts in five minutes: its ten-minute reminder is over.
                source("soon", "2026-12-01T12:05", event: true),
                source("now", "2026-12-01T12:00"),
                source("all-day", "2026-12-02"),
                source("next", "2026-12-01T12:01"),
            ],
            settings: on, now: at("2026-12-01T12:00"), calendar: calendar
        )
        XCTAssertEqual(plan.map(\.itemId), ["next"])
    }

    func testOrderIsByFiringTimeAndTheFurthestAreCut() {
        let plan = Reminders.plan(
            [
                source("task", "2026-12-01T13:00"),
                // Starts later, fires sooner.
                source("event", "2026-12-01T13:05", event: true),
                source("far", "2026-12-09T09:00"),
            ],
            settings: on, now: at("2026-12-01T12:00"), calendar: calendar, limit: 2
        )
        XCTAssertEqual(plan.map(\.itemId), ["event", "task"])
    }

    func testALeadCrossesMidnight() {
        let plan = Reminders.plan(
            [source("event", "2026-12-02T00:05", event: true)],
            settings: on, now: at("2026-12-01T12:00"), calendar: calendar
        )
        XCTAssertEqual(plan.first?.fire.day, 1)
        XCTAssertEqual(plan.map(clock), ["23:55"])
    }

    func testSettingsLoadFromDefaultsAndFallBackWhenAbsentOrUnknown() {
        let suite = "RemindersTests.\(UUID().uuidString)"
        let defaults = UserDefaults(suiteName: suite)!
        defer { defaults.removePersistentDomain(forName: suite) }
        XCTAssertEqual(ReminderSettings.load(from: defaults), ReminderSettings(event: .off, task: .off))
        defaults.set("10", forKey: ReminderSettings.eventKey)
        defaults.set("0", forKey: ReminderSettings.taskKey)
        XCTAssertEqual(ReminderSettings.load(from: defaults), ReminderSettings(event: .tenMinutes, task: .atTime))
        defaults.set("nonsense", forKey: ReminderSettings.taskKey)
        XCTAssertEqual(ReminderSettings.load(from: defaults).task, .off)
    }
}
