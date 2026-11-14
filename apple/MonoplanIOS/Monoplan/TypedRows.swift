// What the app derives from the engine's generated records
// (`MonoplanCore`; spec/swift-bindings-plan.md): the predicates,
// labels and groupings the views share, which the records themselves do
// not carry.

import Foundation
import MonoplanCore
import MonoplanMacKit

extension Array where Element == ListInfo {
    /// The lists a workspace's index shows: active ones, in the doc's
    /// order, without the reserved ids (which have no `ListMeta` row of
    /// their own; a row carrying one is not a second Inbox).
    var userLists: [ListInfo] {
        filter { $0.archivedAt == nil && $0.id != "inbox" && $0.id != "events" }
    }

    /// List id → display name. Reserved ids first, so a user-created
    /// list can never shadow them and an empty read still names Inbox
    /// and Calendar.
    var names: [String: String] {
        var names = ["inbox": "Inbox", "events": "Calendar"]
        for row in self where names[row.id] == nil {
            names[row.id] = row.name
        }
        return names
    }
}

extension ItemState {
    /// Done or Cancelled: nothing left to do.
    var isClosed: Bool { self == .done || self == .cancelled }

    var label: String {
        switch self {
        case .backlog: return "Backlog"
        case .todo: return "To do"
        case .inProgress: return "In progress"
        case .review: return "Review"
        case .done: return "Done"
        case .cancelled: return "Cancelled"
        }
    }
}

extension HitState {
    /// The workflow state a hit's checkbox shows; a binned item's is
    /// masked, so it reads as not done.
    var itemState: ItemState {
        switch self {
        case .backlog, .binned: return .backlog
        case .todo: return .todo
        case .inProgress: return .inProgress
        case .review: return .review
        case .done: return .done
        case .cancelled: return .cancelled
        }
    }
}

extension SyncState {
    /// The browser footer's fold of the two facts
    /// (js/web/src/syncFacts.tsx).
    enum Level: Equatable {
        case offline
        case syncing
        case synced
    }

    /// A local-only workspace is quiet, like a synced one.
    var level: Level {
        if localOnly { return .synced }
        if !online { return .offline }
        return pending ? .syncing : .synced
    }

    /// One line for any state: "Syncing…", "Synced · 2m ago", or the
    /// offline notice (`SyncStatus::offline_notice` in cli/src/ipc.rs).
    func line(now: Date = Date()) -> String {
        if localOnly { return "Local workspace · not synced" }
        switch level {
        case .syncing:
            return "Syncing…"
        case .synced:
            if let at = lastSyncedAt { return "Synced · \(Self.ago(since: at, now: now)) ago" }
            return "Synced"
        case .offline:
            var line = "Offline · never synced"
            if let at = lastSyncedAt { line = "Offline · last synced \(Self.ago(since: at, now: now)) ago" }
            if let offlineReason { line += " · \(offlineReason)" }
            return line
        }
    }

    /// `12s`, `3m`, `2h`, `5d`: the CLI's `ago` (cli/src/ipc.rs).
    static func ago(since millis: Int64, now: Date) -> String {
        let secs = max(0, Int(now.timeIntervalSince1970) - Int(millis / 1000))
        if secs < 60 { return "\(secs)s" }
        if secs < 3600 { return "\(secs / 60)m" }
        if secs < 86_400 { return "\(secs / 3600)h" }
        return "\(secs / 86_400)d"
    }
}

extension AgendaEntry {
    /// The row's identity within its day, across workspaces.
    var key: String { "\(docId ?? "")/\(id)" }
    var isDone: Bool { state == .done }
    /// `when` carries a clock time, not only a day.
    var isTimed: Bool { (when?.count ?? 0) >= 16 }
}

extension BinEntry {
    /// The row's identity across workspaces.
    var key: String { "\(docId ?? "")/\(id)" }
    var ref: ItemRef { ItemRef(docId: docId, itemId: id, title: text) }
}

extension FindHit {
    /// The row's identity across workspaces and kinds: an id is unique
    /// only within its doc.
    var key: String { "\(docId ?? "")/\(kind)/\(id)" }
    var isBinned: Bool { state == .binned }
}

extension LocatedItem {
    /// The row's identity across workspaces: an item id alone can be
    /// in two docs for a moment while it moves between them.
    var key: String { "\(docId ?? "")/\(item.id)" }
    var id: String { item.id }
    var text: String { item.text }
}

/// An item's page reads its row's fields as its own.
extension ItemInfo {
    var id: String { item.id }
    var text: String { item.text }
    var listId: String { item.listId }
    var state: ItemState { item.state }
    var lifecycleAt: Int64 { item.lifecycleAt }
    var createdAt: Int64 { item.createdAt }
    var doneAt: Int64? { item.doneAt }
    var binnedAt: Int64? { item.binnedAt }
    var deadline: String? { item.deadline }
    var when: String? { item.when }
    var whenSuggested: String? { item.whenSuggested }
    /// Whole minutes, when set.
    var duration: Int? { item.duration.map(Int.init) }
    var place: Place? { item.place }

    var isTimed: Bool { (when?.count ?? 0) >= 16 }

    /// The suggestion to ask about: only on an open item with no date
    /// (a date is a plan; a closed or binned item has none to make, and
    /// a suggestion left beside a set `when` is ignored).
    var pendingSuggestion: String? {
        when == nil && binnedAt == nil && closedAt == nil ? whenSuggested : nil
    }

    /// The list's display name: the doc's, else the reserved label.
    var listLabel: String {
        listName ?? [ListInfo]().names[listId] ?? listId
    }

    /// When the item was closed, while it is: the completion stamp for
    /// Done (falling back to the register's transition), the register's
    /// own instant for Cancelled, which stamps nothing.
    var closedAt: Int64? {
        switch state {
        case .done: return doneAt ?? lifecycleAt
        case .cancelled: return lifecycleAt
        default: return nil
        }
    }
}

/// The Done lens's rows of one local day.
struct DoneDay: Equatable, Identifiable {
    /// Local midnight opening the day.
    let day: Date
    let rows: [Item]

    var id: Date { day }

    /// Consecutive rows sharing the local day each was ticked become
    /// one group; the input order (newest first) is kept, so the days
    /// come out newest first.
    static func days(from rows: [Item], calendar: Calendar = .current) -> [DoneDay] {
        var out: [DoneDay] = []
        var day: Date?
        var group: [Item] = []
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

extension SuggestionChange {
    /// What the Kit's `WhenSuggest.edit` decided, as the engine takes
    /// it. The one Kit request type the app still meets: the helper
    /// returns the generated enum once the Kit depends on
    /// `MonoplanCore` (spec/swift-bindings-plan.md, phase 4).
    init(_ edit: SuggestionEdit) {
        switch edit {
        case .keep: self = .keep
        case .clear: self = .clear
        case .set(let when): self = .set(when: when)
        }
    }
}
