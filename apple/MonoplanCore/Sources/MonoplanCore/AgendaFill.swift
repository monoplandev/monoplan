// The Calendar's days ahead as a calendar: every day from Today on has
// a group, empty or not, for two weeks and on to the last day
// something is on (the web's `fillEmptyDays` in
// `js/web/src/dayGroups.ts`). Shared by the Mac's `AgendaTable` and the
// iOS page's `AgendaCollection`, both of which leave it out under
// their "Hide Empty Days" (`agendaHideEmptyDaysKey`, a device
// preference).

import Foundation

extension AgendaGroup {
    /// How many days past Today are shown when nothing is on them, if
    /// nothing is placed later: two weeks of days in all, the web's
    /// `AGENDA_ALL_DAYS_AHEAD`.
    public static let daysAhead = 13

    /// The groups with every day from Today on: an empty group for
    /// each day missing from Today through the later of the last day
    /// with rows and `ahead` days past Today. Days gone by are kept
    /// only when they have rows. Without a Today group (no read yet)
    /// the groups are the non-empty ones as they are.
    public static func filled(_ groups: [AgendaGroup], ahead: Int = daysAhead) -> [AgendaGroup] {
        var out: [AgendaGroup] = []
        var byDay: [String: AgendaGroup] = [:]
        var today: String?
        for group in groups {
            if group.today { today = group.day }
            if let today, group.day >= today {
                byDay[group.day] = byDay[group.day] ?? group
            } else if group.today || !group.rows.isEmpty {
                out.append(group)
            }
        }
        guard let today else { return out }
        var last = addDays(today, ahead)
        if let latest = byDay.filter({ !$0.value.rows.isEmpty }).keys.max(), latest > last {
            last = latest
        }
        var day = today
        while day <= last {
            out.append(byDay[day] ?? AgendaGroup(day: day, today: day == today, rows: []))
            day = addDays(day, 1)
        }
        return out
    }

    /// Today and the days something is on: what is shown under "Hide
    /// Empty Days".
    public static func nonEmpty(_ groups: [AgendaGroup]) -> [AgendaGroup] {
        groups.filter { $0.today || !$0.rows.isEmpty }
    }

    /// `stamp` (`YYYY-MM-DD`) moved by `days`, on the Gregorian
    /// calendar in UTC, so no clock change skips or repeats a day.
    public static func addDays(_ stamp: String, _ days: Int) -> String {
        var calendar = Calendar(identifier: .gregorian)
        calendar.timeZone = TimeZone(identifier: "UTC")!
        let parts = stamp.split(separator: "-").compactMap { Int($0) }
        guard parts.count == 3,
              let date = calendar.date(from: DateComponents(year: parts[0], month: parts[1], day: parts[2])),
              let moved = calendar.date(byAdding: .day, value: days, to: date)
        else { return stamp }
        let c = calendar.dateComponents([.year, .month, .day], from: moved)
        return String(format: "%04d-%02d-%02d", c.year!, c.month!, c.day!)
    }
}
