// List names as `monoplan lists ls --json --all` returns them
// (cli/src/commands/lists.rs `ListJson`), reduced to the one thing the
// tray needs: id → display name. The reserved lists (spec/data-model.md)
// have client-side labels, so they resolve even if the engine's list
// view omits them.

import Foundation

public struct ListRow: Decodable, Equatable, Identifiable {
    public let id: String
    public let name: String
    /// The list's emoji, when the user picked one.
    public let icon: String?
    /// Unix millis the list was archived; nil while it is active.
    public let archivedAt: Int64?
    /// How many Open items the list holds; nil from an engine that
    /// does not count them.
    public let open: Int?

    enum CodingKeys: String, CodingKey {
        case id, name, icon, open
        case archivedAt = "archived_at"
    }
}

public enum Lists {
    public static func parse(_ json: String) throws -> [ListRow] {
        try JSONDecoder().decode([ListRow].self, from: Data(json.utf8))
    }

    /// Reserved ids first, so a user-created list can never shadow them
    /// and an empty or failed fetch still names Inbox and Calendar.
    public static func names(from rows: [ListRow]) -> [String: String] {
        var names = ["inbox": "Inbox", "events": "Calendar"]
        for row in rows where names[row.id] == nil {
            names[row.id] = row.name
        }
        return names
    }

    /// The lists a workspace's index shows: active ones, in the doc's
    /// order, without the reserved ids (which have no `ListMeta` row of
    /// their own; a row carrying one is not a second Inbox).
    public static func active(_ rows: [ListRow]) -> [ListRow] {
        rows.filter { $0.archivedAt == nil && $0.id != "inbox" && $0.id != "events" }
    }
}
