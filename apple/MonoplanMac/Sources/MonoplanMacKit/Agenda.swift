// The agenda projection as `monoplan agenda --json --days 0` returns it
// (cli/src/commands/agenda.rs `DayJson` / `RowJson`; the `--json` contract
// is in spec/cli.md), and the pure rendering of its Today group into menu
// lines. Rendering takes `now` as a parameter so it is unit-tested
// without a clock.

import Foundation

public struct AgendaDay: Decodable, Equatable {
    /// `YYYY-MM-DD`, or nil for the Unscheduled group.
    public let day: String?
    public let today: Bool
    public let rows: [AgendaRow]
}

public struct AgendaRow: Decodable, Equatable {
    public let id: String
    public let text: String
    public let listId: String
    public let state: String
    /// `YYYY-MM-DD` (all-day) or `YYYY-MM-DDTHH:MM` (timed, floating).
    public let when: String?
    /// Minutes.
    public let duration: Int?
    public let deadline: String?
    public let placedBy: String?
    public let tone: String

    public var isDone: Bool { state == "done" }
    public var isTimed: Bool { (when?.count ?? 0) >= 16 }
}

public enum Agenda {
    public static func parse(_ json: String) throws -> [AgendaDay] {
        let decoder = JSONDecoder()
        decoder.keyDecodingStrategy = .convertFromSnakeCase
        return try decoder.decode([AgendaDay].self, from: Data(json.utf8))
    }

    /// Start of a `when`, in the local calendar. Floating values are
    /// wall-clock intents (spec/calendar-plan.md), so local is the only
    /// sensible reading; a reserved `[Zone]` suffix is ignored for now.
    public static func start(of when: String, calendar: Calendar = .current) -> Date? {
        let core = when.split(separator: "[", maxSplits: 1).first.map(String.init) ?? when
        guard core.count >= 10,
              let y = Int(core.prefix(4)),
              let m = Int(core.dropFirst(5).prefix(2)),
              let d = Int(core.dropFirst(8).prefix(2))
        else { return nil }
        var comps = DateComponents(year: y, month: m, day: d)
        if core.count >= 16 {
            guard let hh = Int(core.dropFirst(11).prefix(2)),
                  let mm = Int(core.dropFirst(14).prefix(2))
            else { return nil }
            comps.hour = hh
            comps.minute = mm
        }
        return calendar.date(from: comps)
    }
}

/// One line of the Today section.
public struct MenuLine: Equatable {
    public let id: String
    public let title: String
    /// Dimmed trailing note: "in 2h 15m", "now", "due today", "overdue".
    public let note: String?
    public let done: Bool
    /// The row is past its deadline: the note is drawn in red.
    public let overdue: Bool
}

public enum TodayMenu {
    /// Today's rows as menu lines, in the order the daemon placed them.
    /// Only the Today group is shown; other groups (Unscheduled, later
    /// days) are not a tray's business.
    public static func lines(from days: [AgendaDay], now: Date, calendar: Calendar = .current) -> [MenuLine] {
        guard let today = days.first(where: { $0.today }) else { return [] }
        return today.rows.map { row in
            var title = clipped(row.text)
            if row.isTimed, let when = row.when {
                title = "\(when.dropFirst(11).prefix(5))  \(title)"
            }
            return MenuLine(
                id: row.id,
                title: title,
                note: note(for: row, now: now, calendar: calendar),
                done: row.isDone,
                overdue: row.tone == "overdue"
            )
        }
    }

    static func note(for row: AgendaRow, now: Date, calendar: Calendar) -> String? {
        switch row.tone {
        case "overdue": return String(localized: "overdue")
        case "warning": return String(localized: "due today")
        default: break
        }
        guard !row.isDone, row.isTimed, let when = row.when,
              let start = Agenda.start(of: when, calendar: calendar)
        else { return nil }
        let until = start.timeIntervalSince(now)
        if until > 0 {
            return String(localized: "in \(span(until))")
        }
        let length = TimeInterval((row.duration ?? 0) * 60)
        if length > 0, -until < length {
            return String(localized: "now")
        }
        return String(localized: "started \(span(-until)) ago")
    }

    /// Longest item text shown in the menu, ellipsis included. A menu is
    /// a glance, not a reader; the full text is one click away.
    public static let maxTitleLength = 32

    static func clipped(_ text: String) -> String {
        if text.count <= maxTitleLength { return text }
        return String(text.prefix(maxTitleLength - 1)) + "…"
    }

    /// "3m", "2h 15m", "1d 4h": two units at most, no seconds. Each
    /// unit is its own catalog string so a language can respell it.
    static func span(_ seconds: TimeInterval) -> String {
        let total = Int(seconds.rounded(.down)) / 60
        if total < 1 { return String(localized: "<1m") }
        let days = total / 1440
        let hours = (total % 1440) / 60
        let mins = total % 60
        let d = String(localized: "\(days)d")
        let h = String(localized: "\(hours)h")
        let m = String(localized: "\(mins)m")
        if days > 0 { return hours > 0 ? "\(d) \(h)" : d }
        if hours > 0 { return mins > 0 ? "\(h) \(m)" : h }
        return m
    }
}
