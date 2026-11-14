// What the app derives from the engine's generated records
// (`MonoplanCore`'s `Item`, `ListInfo`, `ItemState`;
// spec/swift-bindings-plan.md): the counterparts of the Kit's `Lists`
// helpers for the pages that have moved onto the typed calls.

import MonoplanCore

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
    /// The state's name as the engine's JSON carries it, for the row
    /// pieces still shared with pages on the old wire (`CheckboxRow`,
    /// `ItemRowMenu`, `AppModel.setLifecycle`). Goes when they take the
    /// enum (phase 3).
    var name: String {
        switch self {
        case .backlog: return "backlog"
        case .todo: return "todo"
        case .inProgress: return "in_progress"
        case .review: return "review"
        case .done: return "done"
        case .cancelled: return "cancelled"
        }
    }
}
