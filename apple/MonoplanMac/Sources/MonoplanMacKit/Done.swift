// The Done lens: `monoplan done --json` rows (cli/src/commands/done.rs;
// the same `ItemJson` as `item ls`, newest first by `lifecycle_at`)
// grouped by the local day each was ticked, as the CLI's text output
// groups them.

import Foundation

public struct DoneDay: Equatable, Identifiable {
    /// Local midnight opening the day.
    public let day: Date
    public let rows: [ItemRow]

    public var id: Date { day }
}

public enum Done {
    /// Consecutive rows sharing a local day become one group; the input
    /// order (newest first) is kept, so the days come out newest first.
    public static func days(from rows: [ItemRow], calendar: Calendar = .current) -> [DoneDay] {
        var out: [DoneDay] = []
        var day: Date?
        var group: [ItemRow] = []
        for row in rows {
            let at = Date(timeIntervalSince1970: TimeInterval(row.lifecycleAt) / 1000)
            let start = calendar.startOfDay(for: at)
            if start != day {
                if let day { out.append(DoneDay(day: day, rows: group)) }
                day = start
                group = []
            }
            group.append(row)
        }
        if let day { out.append(DoneDay(day: day, rows: group)) }
        return out
    }
}
