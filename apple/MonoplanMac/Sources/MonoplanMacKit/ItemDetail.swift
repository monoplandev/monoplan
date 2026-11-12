// One item as `monoplan item show <id> --json` returns it
// (cli/src/commands/items.rs `ItemDetailJson`): the `item ls --json`
// row plus its notes and its list's name and emoji. What an item's page shows.

import Foundation

public struct PlaceInfo: Decodable, Equatable {
    public let label: String
    public let lat: Double?
    public let lon: Double?
    public let address: String?
}

public struct ItemDetail: Decodable, Equatable {
    public let id: String
    public let text: String
    /// Plain text; empty when the item has none.
    public let notes: String
    public let listId: String
    /// Nil for the reserved lists (no `ListMeta` row): `Lists.names`
    /// labels those.
    public let listName: String?
    /// The list's emoji, when it has one.
    public let listIcon: String?
    /// Workflow state name, as `ItemRow.state`.
    public let state: String
    /// Unix millis the workflow state was entered.
    public let lifecycleAt: Int64
    public let createdAt: Int64
    /// Unix millis of the last entry into Done; never cleared, so only
    /// meaningful while `state` is `done`.
    public let doneAt: Int64?
    /// Unix millis the item was binned; nil while it is not.
    public let binnedAt: Int64?
    /// `YYYY-MM-DD`, when set.
    public let deadline: String?
    /// `YYYY-MM-DD` or `YYYY-MM-DDTHH:MM`, when set.
    public let when: String?
    /// Whole minutes, when set.
    public let duration: Int?
    public let place: PlaceInfo?

    public var isTimed: Bool { (when?.count ?? 0) >= 16 }

    /// The list's display name: the doc's, else the reserved label.
    public var listLabel: String {
        listName ?? Lists.names(from: [])[listId] ?? listId
    }

    /// When the item was closed, while it is: the completion stamp for
    /// Done (falling back to the register's transition), the register's
    /// own instant for Cancelled, which stamps nothing.
    public var closedAt: Int64? {
        switch state {
        case "done": return doneAt ?? lifecycleAt
        case "cancelled": return lifecycleAt
        default: return nil
        }
    }

    public static func parse(_ json: String) throws -> ItemDetail {
        let decoder = JSONDecoder()
        decoder.keyDecodingStrategy = .convertFromSnakeCase
        return try decoder.decode(ItemDetail.self, from: Data(json.utf8))
    }
}
