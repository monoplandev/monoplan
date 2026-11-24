// The end of a planned date as a day: the second way to read and write
// the stored `duration` (spec/calendar-plan.md; the web's span helpers
// in `js/web/src/format.tsx`: `spanEndOffset`, `allDayDurationFor`,
// `durationBetween` under `isValidEndDayOffset`, `sameDayDuration`).
// Shared by the Mac panel's date row and the iOS date editor, which
// both show a start day and, after an arrow, the day the span ends on.

import Foundation

public enum WhenSpan {
    public static let day = 24 * 60
    /// The longest length the register holds, in minutes.
    public static let maxDuration = 365 * day
    /// The length a timed date with none stored is drawn with.
    public static let defaultLength = 60

    /// Minutes past midnight a `when` starts at, nil when it is all-day.
    public static func clock(of when: String) -> Int? {
        guard when.count >= 16 else { return nil }
        let parts = when.dropFirst(11).prefix(5).split(separator: ":").compactMap { Int($0) }
        return parts.count == 2 ? parts[0] * 60 + parts[1] : nil
    }

    /// Whole days from one `YYYY-MM-DD` to another.
    public static func days(from: String, to: String, calendar: Calendar) -> Int? {
        guard let a = Agenda.start(of: from, calendar: calendar),
              let b = Agenda.start(of: to, calendar: calendar)
        else { return nil }
        return calendar.dateComponents([.day], from: a, to: b).day
    }

    /// Days past the start day the span ends on, 0 for the start day
    /// itself: beside a time, the day the end's clock time falls on;
    /// all-day, the last whole day.
    public static func endOffset(when: String, duration: Int?) -> Int {
        let minutes = max(0, duration ?? 0)
        if let start = clock(of: when) { return (start + minutes) / day }
        return minutes == 0 ? 0 : (minutes + day - 1) / day - 1
    }

    /// The span runs past one day, which makes the item an event
    /// whatever its Event switch says (the core's `spans_days`,
    /// spec/events-plan.md "Multi-day is an event"): all-day, two days
    /// or more; beside a time, 24 hours or more. A timed span under 24
    /// hours that crosses midnight is one sitting and stays what it is.
    public static func spansDays(when: String?, duration: Int?) -> Bool {
        guard let when, let minutes = duration, minutes > 0 else { return false }
        return clock(of: when) != nil ? minutes >= day : minutes > day
    }

    /// The length that ends the span `offset` days past its start day:
    /// 0 for none stored, nil when the register cannot hold that day
    /// (before the start; with a timed start, the start day itself
    /// while the end is not past the start on the clock; past the
    /// one-year ceiling). All-day it counts whole days. Beside a time
    /// the end's clock time is kept (`defaultLength` after the start
    /// while no length is stored).
    public static func endLength(when: String, duration: Int?, offset: Int) -> Int? {
        guard offset >= 0 else { return nil }
        guard let start = clock(of: when) else {
            let length = (offset + 1) * day
            guard length <= maxDuration else { return nil }
            return offset == 0 ? 0 : length
        }
        let stored = max(0, duration ?? 0)
        let end = (start + (stored > 0 ? stored : defaultLength)) % day
        let length = offset * day + end - start
        return (1...maxDuration).contains(length) ? length : nil
    }

    /// The length with the end brought back onto the start day: none
    /// all-day; beside a time the end's clock time where that is after
    /// the start, else `defaultLength`, cut short of midnight.
    public static func sameDayLength(when: String, duration: Int?) -> Int {
        guard let start = clock(of: when) else { return 0 }
        let stored = max(0, duration ?? 0)
        if stored > 0 {
            if (start + stored) / day == 0 { return stored }
            let end = (start + stored) % day
            if end > start { return end - start }
        }
        return max(1, min(defaultLength, day - 1 - start))
    }

    /// The length a time put on an all-day date leaves it with. A span
    /// of whole days would end a day late (Tue to Thu as three days is
    /// 72 hours, so from Tue 09:00 it would end Fri 09:00): it is
    /// reshaped to end on the same last day, `defaultLength` after the
    /// start. Anything else is kept.
    public static func lengthForTimedStart(_ duration: Int?) -> Int? {
        guard let duration, duration >= 2 * day, duration % day == 0 else { return duration }
        return duration - day + defaultLength
    }

    /// A day as a narrow box shows it: "Wed 23 Sept", and in another
    /// year the year in the weekday's place ("23 Sept 2027"), as the
    /// web's date inputs.
    public static func dayLabel(_ day: String, now: Date = Date(), calendar: Calendar) -> String {
        guard let date = Agenda.start(of: day, calendar: calendar) else { return day }
        if calendar.isDate(date, equalTo: now, toGranularity: .year) {
            return date.formatted(.dateTime.weekday(.abbreviated).day().month(.abbreviated))
        }
        return date.formatted(.dateTime.day().month(.abbreviated).year())
    }
}
