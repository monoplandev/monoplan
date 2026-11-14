// The tray menu's rows, rendered from the engine's typed records
// (`Engine.agenda` → `AgendaGroup`, `Engine.starred` → `StarredItem`;
// spec/swift-bindings-plan.md). Pure: rendering takes `now` as a
// parameter so it is unit-tested without a clock or an engine.

import Foundation
import MonoplanCore
import MonoplanMacKit

/// One line of the Today or Starred section.
struct MenuLine: Equatable {
    let id: String
    let title: String
    /// Dimmed trailing note: "in 2h 15m", "now", "due today", "overdue".
    let note: String?
    let done: Bool
    /// The row is past its deadline, or a task past its planned day:
    /// the note is drawn in red.
    let overdue: Bool
    /// Where the item lives, which is what a click opens the window
    /// on: its list, in its workspace (nil for Home's).
    let listId: String
    let docId: String?
}

enum TodayMenu {
    /// Today's rows as menu lines, in the order the engine placed them.
    /// Only the Today group is shown; other groups (Unscheduled, later
    /// days) are not a tray's business.
    static func lines(from groups: [AgendaGroup], now: Date, calendar: Calendar = .current) -> [MenuLine] {
        guard let today = groups.first(where: { $0.today }) else { return [] }
        return today.rows.map { row in
            var title = clipped(row.text)
            if isTimed(row), let when = row.when {
                title = "\(when.dropFirst(11).prefix(5))  \(title)"
            }
            return MenuLine(
                id: row.id,
                title: title,
                note: note(for: row, now: now, calendar: calendar),
                done: row.state == .done,
                overdue: row.tone == .overdue,
                listId: row.listId,
                docId: row.docId
            )
        }
    }

    /// `when` carries a clock time, not only a day.
    static func isTimed(_ row: AgendaEntry) -> Bool { (row.when?.count ?? 0) >= 16 }

    static func note(for row: AgendaEntry, now: Date, calendar: Calendar) -> String? {
        switch row.tone {
        case .overdue: return String(localized: "overdue")
        case .warning: return String(localized: "due today")
        case .neutral: break
        }
        guard row.state != .done, isTimed(row), let when = row.when,
              let start = Agenda.start(of: when, calendar: calendar)
        else { return nil }
        let until = start.timeIntervalSince(now)
        if until > 0 {
            return String(localized: "in \(span(until))")
        }
        let length = TimeInterval(row.duration ?? 0) * 60
        if length > 0, -until < length {
            return String(localized: "now")
        }
        return String(localized: "started \(span(-until)) ago")
    }

    /// Longest item text shown in the menu, ellipsis included. A menu is
    /// a glance, not a reader; the full text is one click away.
    static let maxTitleLength = 32

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

enum StarredMenu {
    /// How many starred rows the tray shows: the top of the list is the
    /// point of curating it.
    static let limit = 3

    /// The item's list is the muted note, by the name the engine
    /// resolved for it; nothing when it has none.
    static func lines(from rows: [StarredItem], limit: Int = limit) -> [MenuLine] {
        rows.prefix(limit).map { row in
            MenuLine(
                id: row.id,
                title: TodayMenu.clipped(row.text),
                note: row.listName.map(TodayMenu.clipped),
                done: false,
                overdue: false,
                listId: row.listId,
                docId: row.docId
            )
        }
    }
}

extension SyncState {
    /// Mirrors `SyncStatus::offline_notice` in cli/src/ipc.rs: nil while
    /// online, and for a local-only workspace, which has nothing to be
    /// offline from.
    func offlineNotice(now: Date = Date()) -> String? {
        if online || localOnly { return nil }
        var line: String
        if let at = lastSyncedAt {
            line = String(localized: "Offline · last synced \(Self.ago(since: at, now: now)) ago")
        } else {
            line = String(localized: "Offline · never synced")
        }
        if let offlineReason { line += " · \(offlineReason)" }
        return line
    }

    /// One line for any state: "Syncing…", "Synced · 2m ago", or the
    /// offline notice. What the window and the Settings window show.
    func line(now: Date = Date()) -> String {
        if localOnly { return String(localized: "Local workspace · not synced") }
        if !online { return offlineNotice(now: now) ?? String(localized: "Offline") }
        if pending { return String(localized: "Syncing…") }
        if let at = lastSyncedAt {
            return String(localized: "Synced · \(Self.ago(since: at, now: now)) ago")
        }
        return String(localized: "Synced")
    }

    /// `12s`, `3m`, `2h`, `5d`: the CLI's `ago` (cli/src/ipc.rs).
    static func ago(since millis: Int64, now: Date) -> String {
        let secs = max(0, Int(now.timeIntervalSince1970) - Int(millis / 1000))
        if secs < 60 { return String(localized: "\(secs)s") }
        if secs < 3600 { return String(localized: "\(secs / 60)m") }
        if secs < 86_400 { return String(localized: "\(secs / 3600)h") }
        return String(localized: "\(secs / 86_400)d")
    }
}
