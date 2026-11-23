// What dropping a Calendar row on a day writes: the web's `dropOnDay`
// (`js/web/src/dayGroups.ts`; `spec/calendar-plan.md` "Reschedule").

import Foundation

public enum AgendaDrop: Equatable {
    /// The item's `when` becomes this.
    case when(String)
    /// The item's deadline becomes this.
    case deadline(String)

    /// The write for an item with these dates dropped on `day`
    /// (`YYYY-MM-DD`). An item with a `when` moves its `when` there,
    /// the clock time kept (its `duration` is not touched, so a span
    /// keeps its length), whichever of its rows was grabbed; an item
    /// with only a deadline moves the deadline. Nil when nothing would
    /// change, and for an item a drop does not move: a closed task
    /// (`closedTask`: done or cancelled, and not an event with a
    /// `when`) or one with no date.
    public static func write(when: String?, deadline: String?, closedTask: Bool, day: String) -> AgendaDrop? {
        guard !closedTask, day.count == 10 else { return nil }
        if let when, when.count >= 10 {
            if when.prefix(10) == day { return nil }
            return .when(day + when.dropFirst(10))
        }
        if let deadline, !deadline.isEmpty {
            return deadline == day ? nil : .deadline(day)
        }
        return nil
    }
}
