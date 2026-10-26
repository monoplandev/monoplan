// List names as `monoplan lists ls --json --all` returns them
// (cli/src/commands/lists.rs `ListJson`), reduced to the one thing the
// tray needs: id → display name. The reserved lists (spec/data-model.md)
// have client-side labels, so they resolve even if the daemon's list
// view omits them.

import Foundation

public struct ListRow: Decodable, Equatable {
    public let id: String
    public let name: String
}

public enum Lists {
    public static func parse(_ json: String) throws -> [ListRow] {
        try JSONDecoder().decode([ListRow].self, from: Data(json.utf8))
    }

    /// Reserved ids first, so a user-created list can never shadow them
    /// and an empty or failed fetch still names Inbox and Events.
    public static func names(from rows: [ListRow]) -> [String: String] {
        var names = ["inbox": "Inbox", "events": "Events"]
        for row in rows where names[row.id] == nil {
            names[row.id] = row.name
        }
        return names
    }
}
