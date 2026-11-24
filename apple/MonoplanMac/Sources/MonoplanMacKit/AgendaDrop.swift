// What dropping a Calendar row on a day writes: the web's `dropOnDay`
// (`js/web/src/dayGroups.ts`; `spec/calendar-plan.md` "Reschedule").

import Foundation

public enum AgendaDrop: Equatable {
    /// The item's `when` becomes this.
    case when(String)

    /// The write for an item with this `when` dropped on `day`
    /// (`YYYY-MM-DD`): the `when` moves there, the clock time kept (its
    /// `duration` is not touched, so a span keeps its length). Nil when
    /// nothing would change, and for an item a drop does not move: a
    /// closed task (`closedTask`: done or cancelled, and not an event
    /// with a `when`) or one with no date.
    public static func write(when: String?, closedTask: Bool, day: String) -> AgendaDrop? {
        guard !closedTask, day.count == 10, let when, when.count >= 10 else { return nil }
        if when.prefix(10) == day { return nil }
        return .when(day + when.dropFirst(10))
    }
}
