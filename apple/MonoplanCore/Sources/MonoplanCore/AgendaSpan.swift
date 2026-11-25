// A Calendar row's reading of its day, shared by the Mac's
// `AgendaTable` and the iOS page's `AgendaCollection`, as the web's
// `Upcoming.tsx` reads a `DayRow` (`spec/calendar-plan.md` "Spans"):
// `Engine.agenda` puts a `when` covering several days on each of them,
// every row with its `span`, so the day a row stands for, the "Day 2"
// under its title and what its time lane says come from the row alone.
// The words are the apps' own (their strings catalogs); this says which.

import Foundation

/// What a Calendar row's time lane holds, at its trailing edge.
public enum AgendaLane: Equatable {
    /// Nothing: a task for the day, at no particular time, on every day
    /// an event would say "All day".
    case none
    /// "All day": an event with no clock time, or a timed span's day
    /// it runs clean through.
    case allDay
    /// The start's clock time, `HH:MM`.
    case start(String)
    /// "until HH:MM": a timed span's last day, when it ends.
    case until(String)
}

extension AgendaEntry {
    /// The day the row is on (`YYYY-MM-DD`): the `when`'s day, moved on
    /// to the span's day for a later row of a span. For a row in
    /// Overdue, the day it was planned for. Nil without a `when`.
    public var rowDay: String? {
        guard let when, when.count >= 10 else { return nil }
        let start = String(when.prefix(10))
        guard let span, span.day > 1 else { return start }
        return AgendaGroup.addDays(start, Int(span.day) - 1)
    }

    /// The day of a multi-day span this row is, for the "Day 2" under
    /// its title, the first day included. Nil off a span, in Overdue
    /// (one row), and on a timed span under 24 hours: an evening that
    /// runs past midnight is one occasion, its rows already reading
    /// "22:00" and "until 03:00".
    public var spanDayLabel: Int? {
        guard let span, tone != .overdue, let when else { return nil }
        if when.count >= 16, (duration ?? 0) < 1440 { return nil }
        return Int(span.day)
    }

    /// The time lane of a row outside Overdue (which shows the day it
    /// was planned for instead): the start's clock on a timed row, or
    /// on a timed span its first day's; "All day" for an event without
    /// one, or on the days a timed span runs through; its end clock on
    /// the last day ("until 03:00"). A task says nothing where an event
    /// would say "All day": it is for the day, it does not fill it.
    public var lane: AgendaLane {
        let allDay: AgendaLane = event ? .allDay : .none
        guard let when, when.count >= 16 else { return allDay }
        let clock = String(when.dropFirst(11).prefix(5))
        guard let span, span.day > 1 else { return .start(clock) }
        guard span.day == span.total, let minutes = duration, minutes > 0,
              let start = Self.minutes(clock)
        else { return allDay }
        let end = (start + Int(minutes)) % 1440
        return .until(String(format: "%02d:%02d", end / 60, end % 60))
    }

    /// Minutes past midnight of an `HH:MM` clock.
    private static func minutes(_ clock: String) -> Int? {
        let parts = clock.split(separator: ":").compactMap { Int($0) }
        return parts.count == 2 ? parts[0] * 60 + parts[1] : nil
    }
}

extension AgendaGroup {
    /// The groups less the days gone by: what is shown under "Hide
    /// Past Days" (the web's `agendaHidePastDays`), starting at
    /// Today, with Overdue, which the apps lift out of Today's group.
    /// Without a Today group (no read yet) the groups as they are.
    public static func upcoming(_ groups: [AgendaGroup]) -> [AgendaGroup] {
        guard let today = groups.first(where: \.today)?.day else { return groups }
        return groups.filter { $0.day >= today }
    }
}
