// What the app derives from the engine's generated records
// (`MonoplanCore`; spec/swift-bindings-plan.md): the predicates,
// labels and groupings the views share, which the records themselves do
// not carry.

import Foundation
import MonoplanCore
import MonoplanMacKit

/// What an item with no list is filed under, by whether it has a
/// date: Inbox holds only undated items, a dated one lives on the
/// Calendar (spec/inbox-plan.md "Membership"). A label only: the list
/// id is `inbox` either way.
enum NoList {
    static func label(dated: Bool) -> String { dated ? String(localized: "Calendar") : String(localized: "Inbox") }

    /// A list name the engine supplied, for display. The engine names
    /// "no list" in English ("Inbox", or "Calendar" for a dated item:
    /// `no_list_label` in cli/src/api/lists.rs); here it is said in the
    /// app's language. Any other list's name is the user's own.
    static func display(_ name: String?, listId: String?) -> String? {
        guard listId == "inbox", let name else { return name }
        return label(dated: name == "Calendar")
    }
}

extension Array where Element == ListInfo {
    /// The lists a workspace's index shows: active ones, in the doc's
    /// order, without the reserved id (which has no `ListMeta` row of
    /// its own; a row carrying it is not a second Inbox).
    var userLists: [ListInfo] {
        filter { $0.archivedAt == nil && $0.id != "inbox" }
    }

    /// List id → display name. The reserved id first, so a user-created
    /// list can never shadow it and an empty read still names Inbox.
    var names: [String: String] {
        var names = ["inbox": String(localized: "Inbox")]
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
        case .backlog: return String(localized: "Backlog")
        case .todo: return String(localized: "To do")
        case .inProgress: return String(localized: "In progress")
        case .review: return String(localized: "Review")
        case .done: return String(localized: "Done")
        case .cancelled: return String(localized: "Cancelled")
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
        if localOnly { return String(localized: "Local workspace · not synced") }
        switch level {
        case .syncing:
            return String(localized: "Syncing…")
        case .synced:
            if let at = lastSyncedAt { return String(localized: "Synced · \(Self.ago(since: at, now: now)) ago") }
            return String(localized: "Synced")
        case .offline:
            var line = String(localized: "Offline · never synced")
            if let at = lastSyncedAt { line = String(localized: "Offline · last synced \(Self.ago(since: at, now: now)) ago") }
            if let offlineReason { line += " · \(offlineReason)" }
            return line
        }
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

extension AgendaEntry {
    /// The row's identity within its day, across workspaces.
    var key: String { "\(docId ?? "")/\(id)" }
    var isDone: Bool { state == .done }
    /// The row shows a clock time: its `when` carries one.
    var isTimed: Bool { (when?.count ?? 0) >= 16 }

    /// Whether the row leads with a checkbox. An event's own row does
    /// not while it is open (spec/events-plan.md: nothing asks for the
    /// tick) or once its day has gone by (it is a record of what was
    /// on): the event pill holds the box's place. One marked done from
    /// today on shows the box, ticked.
    var showsCheck: Bool {
        guard event else { return true }
        return state.isClosed && !whenIsPast
    }

    /// The row's text is muted: a done row, and an event's own row on
    /// a day gone by, done or not (the only rows the past days hold; a
    /// task whose day has passed is in Overdue). The Mac's rule
    /// (`MainWindow.agendaRowIsPast`).
    var isMuted: Bool {
        isDone || (event && whenIsPast)
    }

    /// The `when`'s day is before today.
    private var whenIsPast: Bool {
        guard let when, when.count >= 10 else { return false }
        return String(when.prefix(10)) < WhenText.today()
    }

    /// The quick moves the row's menu offers (`ItemRowMenu`): an
    /// overdue row goes to today or tomorrow, a row on today to
    /// tomorrow, and any other row nowhere. Only an open row. The clock
    /// time is kept, as a pick in the date editor keeps it.
    var quickMoves: [WhenMove] {
        WhenMove.offers(for: self, today: WhenText.today(), tomorrow: WhenText.today(plus: 1))
    }
}

/// One quick move of an item's planned date: where to, and the `when`
/// that puts it there.
struct WhenMove: Equatable {
    enum Day { case today, tomorrow }
    let day: Day
    let when: String

    static func offers(for row: AgendaEntry, today: String, tomorrow: String) -> [WhenMove] {
        guard !row.state.isClosed, let when = row.when, when.count >= 10 else { return [] }
        let day = String(when.prefix(10))
        let time = String(when.dropFirst(10))
        if day == today {
            return [WhenMove(day: .tomorrow, when: tomorrow + time)]
        }
        // A past day in the overdue tone: a task that slipped. A span
        // still running, or an event gone by, has nothing to catch up.
        if day < today, row.tone == .overdue {
            return [WhenMove(day: .today, when: today + time), WhenMove(day: .tomorrow, when: tomorrow + time)]
        }
        return []
    }
}

extension BinEntry {
    /// The row's identity across workspaces.
    var key: String { "\(docId ?? "")/\(id)" }
    var ref: ItemRef { ItemRef(docId: docId, itemId: id, title: text) }
}

extension AssignedEntry {
    /// The row's identity across workspaces.
    var key: String { "\(docId ?? "")/\(item.id)" }
    var ref: ItemRef { ItemRef(docId: docId, itemId: item.id, title: item.text) }
}

extension DoneEntry {
    /// The row's identity across workspaces.
    var key: String { "\(docId ?? "")/\(item.id)" }
    var ref: ItemRef { ItemRef(docId: docId, itemId: item.id, title: item.text) }
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
    /// Account id of the member the item is assigned to, when it is.
    var assignee: String? { item.assignee }
    var when: String? { item.when }
    /// The IANA zone a timed `when` is fixed to; nil is floating
    /// (spec/calendar-plan.md "Fixed instants").
    var whenZone: String? { item.whenZone }
    var whenSuggested: String? { item.whenSuggested }
    /// An event (spec/events-plan.md): it happens on its date, so
    /// the date goes by instead of going overdue.
    var event: Bool { item.event }
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
        if listId == "inbox" { return NoList.label(dated: when != nil) }
        return listName ?? listId
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
    let rows: [DoneEntry]

    var id: Date { day }

    /// Consecutive rows sharing the local day each was ticked become
    /// one group; the input order (newest first) is kept, so the days
    /// come out newest first.
    static func days(from rows: [DoneEntry], calendar: Calendar = .current) -> [DoneDay] {
        var out: [DoneDay] = []
        var day: Date?
        var group: [DoneEntry] = []
        for row in rows {
            let at = Date(timeIntervalSince1970: TimeInterval(row.item.lifecycleAt) / 1000)
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
