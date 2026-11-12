// The agenda as the iOS home-screen widget sees it. A widget is its own
// process and runs no engine (no DEK, no sqlite, no socket), so the app
// leaves it a snapshot: the agenda projection's JSON, as fetched, in a
// file in the app group's container, rewritten whenever the app reads
// the agenda. The widget only ever reads that file; what it shows is as
// fresh as the app's last look.
//
// The snapshot is item text in the clear, outside the engine's
// encrypted store: on disk it is protected by iOS data protection only
// (readable after the first unlock, so the widget can draw while the
// phone is locked). It is removed on sign-out.

import Foundation

public enum AgendaSnapshot {
    /// The app group the app and its widget extension share
    /// (`Config/*.entitlements` in `apple/MonoplanIOS`).
    public static let appGroup = "group.app.monoplan.ios"
    /// The widget's `kind`, for `WidgetCenter.reloadTimelines(ofKind:)`.
    public static let widgetKind = "app.monoplan.ios.agenda"

    /// Where the snapshot lives; nil when the process has no such app
    /// group (an unsigned build, or the entitlement is missing).
    public static func url() -> URL? {
        FileManager.default
            .containerURL(forSecurityApplicationGroupIdentifier: appGroup)?
            .appendingPathComponent("agenda-widget.json")
    }

    /// Write the agenda JSON. False when the file already says this,
    /// so the caller reloads the widget only on a real change.
    @discardableResult
    public static func write(_ json: String, to url: URL) throws -> Bool {
        let data = Data(json.utf8)
        if (try? Data(contentsOf: url)) == data { return false }
        try data.write(to: url, options: [.atomic, .completeFileProtectionUntilFirstUserAuthentication])
        return true
    }

    /// The snapshot's day groups; nil when there is none (never
    /// written, or signed out) or it does not parse.
    public static func read(from url: URL) -> [AgendaDay]? {
        guard let data = try? Data(contentsOf: url), let json = String(data: data, encoding: .utf8) else {
            return nil
        }
        return try? Agenda.parse(json)
    }

    /// Remove the snapshot. True when there was one.
    @discardableResult
    public static func clear(at url: URL) -> Bool {
        (try? FileManager.default.removeItem(at: url)) != nil
    }
}

/// One line of the widget: a row of the agenda, or the heading of a
/// later day's rows.
public enum WidgetLine: Equatable, Identifiable {
    /// `day` is the `YYYY-MM-DD` the rows under it are on.
    case heading(day: String)
    case row(WidgetRow)

    public var id: String {
        switch self {
        case .heading(let day): return "day/\(day)"
        case .row(let row): return row.id
        }
    }
}

public struct WidgetRow: Equatable {
    /// Unique among the widget's lines (the item on its day).
    public let id: String
    public let text: String
    /// `HH:MM` when the row is timed.
    public let time: String?
    /// The projection's tone: `overdue`, `warning` (due today) or
    /// `neutral`.
    public let tone: String
}

public enum WidgetAgenda {
    /// The widget's lines for `date`: the day's open rows first (no
    /// heading: the widget is titled Today), then each later day's
    /// under its heading, cut to `limit` lines and never ending on a
    /// heading. `todayCount` is how many open rows the day has in all.
    ///
    /// The snapshot may be older than `date`: the widget keeps drawing
    /// after midnight without the app having run. So "the day" is
    /// every dated group up to `date`, not the group the projection
    /// flagged: rows of days that have since passed are still to do.
    /// Done and cancelled rows and the Unscheduled group are left out.
    public static func lines(
        from days: [AgendaDay], on date: Date, limit: Int, calendar: Calendar = .current
    ) -> (lines: [WidgetLine], todayCount: Int) {
        let today = dayText(date, calendar: calendar)
        var seen = Set<String>()
        var todayRows: [WidgetRow] = []
        var later: [(day: String, rows: [WidgetRow])] = []
        for group in days {
            guard let day = group.day else { continue }
            let open = group.rows.filter { $0.state != "done" && $0.state != "cancelled" }
            if day <= today {
                for row in open {
                    // A multi-day span has a row on each of its days.
                    let key = "\(row.docId ?? "")/\(row.id)"
                    if seen.insert(key).inserted { todayRows.append(widgetRow(row, key: key)) }
                }
            } else if !open.isEmpty {
                later.append((day, open.map { widgetRow($0, key: "\(day)/\($0.docId ?? "")/\($0.id)") }))
            }
        }
        var lines = todayRows.map(WidgetLine.row)
        for (day, rows) in later.sorted(by: { $0.day < $1.day }) {
            lines.append(.heading(day: day))
            lines.append(contentsOf: rows.map(WidgetLine.row))
        }
        lines = Array(lines.prefix(max(0, limit)))
        if case .heading = lines.last { lines.removeLast() }
        return (lines, todayRows.count)
    }

    static func widgetRow(_ row: AgendaRow, key: String) -> WidgetRow {
        let time = row.isTimed ? row.when.map { String($0.dropFirst(11).prefix(5)) } : nil
        return WidgetRow(id: key, text: row.text, time: time, tone: row.tone)
    }

    /// `YYYY-MM-DD` of `date` in the local calendar: the form the
    /// projection's days take, so the two compare as text.
    public static func dayText(_ date: Date, calendar: Calendar = .current) -> String {
        let c = calendar.dateComponents([.year, .month, .day], from: date)
        return String(format: "%04d-%02d-%02d", c.year ?? 0, c.month ?? 0, c.day ?? 0)
    }
}
