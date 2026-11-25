// The tray menu's rows, rendered from the engine's typed records
// (`Engine.agenda` → `AgendaGroup`, `Engine.inbox` → `InboxEntry`;
// spec/swift-bindings-plan.md). Pure: rendering takes `now` as a
// parameter so it is unit-tested without a clock or an engine.

import Foundation
import MonoplanCore
import MonoplanMacKit

/// One line of the Overdue, Today or Inbox section.
struct MenuLine: Equatable {
    let id: String
    let title: String
    /// Dimmed trailing note: "in 2h 15m", "now", or the day an overdue
    /// row was planned for ("20 Oct").
    let note: String?
    let done: Bool
    /// The row is a task past its planned day: it goes in the menu's
    /// Overdue section, above Today.
    let overdue: Bool
    /// The row leads with the event pill, as its Calendar row does
    /// (`AgendaEntry.showsCheck`): an open event.
    var event = false
    /// Where the item lives, which is what a click opens the window
    /// on: its list, in its workspace (nil for Home's).
    let listId: String
    let docId: String?
    /// A dated item with no list has no row in Inbox
    /// (spec/inbox-plan.md): a click opens the Calendar instead.
    var onCalendar = false
}

enum TodayMenu {
    /// Today's rows as menu lines, in the order the engine placed them.
    /// Only the Today group is shown; later days are not a tray's
    /// business. A task leaves the agenda once done (the engine no
    /// longer places it), so the tray is what is left.
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
                event: !row.showsCheck,
                listId: row.listId,
                docId: row.docId,
                onCalendar: row.listId == MainWindowModel.inbox && row.when != nil
            )
        }
    }

    /// The row shows a clock time: its `when` carries one.
    static func isTimed(_ row: AgendaEntry) -> Bool {
        (row.when?.count ?? 0) >= 16
    }

    static func note(for row: AgendaEntry, now: Date, calendar: Calendar) -> String? {
        switch row.tone {
        // The day it was planned for; the Overdue header already says
        // it is late. Nothing for one planned earlier today.
        case .overdue: return MainWindowView.overdueDay(row, now: now, calendar: calendar)
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

/// The event the menu bar names beside the moon: one under way, or one
/// about to start. Device-only, on by default (Settings › General).
enum NowEvent {
    /// The `UserDefaults` key; absent reads as on.
    static let key = "menuBarShowsEvent"
    static let defaultOn = true

    static func isOn(in defaults: UserDefaults = .standard) -> Bool {
        defaults.object(forKey: key) as? Bool ?? defaultOn
    }

    /// How long before its start an event is named.
    static let lead: TimeInterval = 5 * 60
    /// Shorter than the menu's rows: the menu bar is shared.
    static let maxTitleLength = 24

    struct Shown: Equatable {
        /// "Standup" under way, "Standup · in 3m" ahead.
        let title: String
        /// When the title next changes (a countdown's next minute, an
        /// event starting, ending or coming within the lead): nil when
        /// nothing today will change it.
        let next: Date?
    }

    /// Among today's open timed events: the one starting within the
    /// lead, soonest first, since what is next is the news; else the
    /// one under way that started last. An event with no duration is
    /// never under way: it is named until its start.
    static func shown(from groups: [AgendaGroup], now: Date, calendar: Calendar = .current) -> Shown {
        let rows = groups.first(where: { $0.today })?.rows ?? []
        var spans: [(title: String, start: Date, end: Date)] = []
        // A span has a row on each of its days: one entry an event.
        var seen = Set<String>()
        for row in rows where !row.showsCheck && TodayMenu.isTimed(row) {
            guard seen.insert("\(row.docId ?? "")/\(row.id)").inserted,
                  let when = row.when, let start = Agenda.start(of: when, calendar: calendar)
            else { continue }
            let end = start.addingTimeInterval(TimeInterval(row.duration ?? 0) * 60)
            spans.append((clipped(row.text), start, end))
        }
        var changes = spans.flatMap { [$0.start.addingTimeInterval(-lead), $0.start, $0.end] }
        var title: String?
        let ahead = spans.filter { $0.start > now && $0.start.timeIntervalSince(now) <= lead }
        if let soon = ahead.min(by: { $0.start < $1.start }) {
            let until = soon.start.timeIntervalSince(now)
            title = "\(soon.title) · " + String(localized: "in \(TodayMenu.span(until))")
            // The countdown's next whole minute.
            let step = until.truncatingRemainder(dividingBy: 60)
            changes.append(now.addingTimeInterval(step > 0 ? step : 60))
        } else if let current = spans.filter({ $0.start <= now && now < $0.end }).max(by: { $0.start < $1.start }) {
            title = current.title
        }
        return Shown(title: title ?? "", next: changes.filter { $0 > now }.min())
    }

    static func clipped(_ text: String) -> String {
        if text.count <= maxTitleLength { return text }
        return String(text.prefix(maxTitleLength - 1)) + "…"
    }
}

enum InboxMenu {
    /// How many Inbox rows the tray shows: enough to see what is
    /// waiting, the rest is the window's.
    static let limit = 5

    /// The head of the combined Inbox as this Mac shows it: the
    /// engine's queue is oldest first, reversed when the device's
    /// order is newest first (`inboxNewestFirstKey`).
    static func lines(from rows: [InboxEntry], newestFirst: Bool, limit: Int = limit) -> [MenuLine] {
        let shown = newestFirst ? Array(rows.reversed()) : rows
        return shown.prefix(limit).map { row in
            MenuLine(
                id: row.item.id,
                title: TodayMenu.clipped(row.item.text),
                note: nil,
                done: false,
                overdue: false,
                listId: MainWindowModel.inbox,
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
