// One item as `monoplan item show <id> --json` returns it
// (cli/src/commands/items.rs `ItemDetailJson`): the `item ls --json`
// row plus its notes and its list's name and emoji. What an item's page shows.

import Foundation

public struct PlaceInfo: Decodable, Equatable {
    public let label: String
    public let lat: Double?
    public let lon: Double?
    public let address: String?

    public init(label: String, lat: Double? = nil, lon: Double? = nil, address: String? = nil) {
        self.label = label
        self.lat = lat
        self.lon = lon
        self.address = address
    }
}

/// A known place as `monoplan places --json` returns it
/// (cli/src/commands/items.rs `KnownPlaceJson`): a place already on
/// some item of the account, with how often and how recently it is used.
public struct KnownPlace: Decodable, Hashable {
    public let label: String
    public let lat: Double?
    public let lon: Double?
    public let address: String?
    public let count: Int
    /// Unix millis of the newest use.
    public let lastUsed: Int64

    public var place: PlaceInfo { PlaceInfo(label: label, lat: lat, lon: lon, address: address) }

    public static func parse(_ json: String) throws -> [KnownPlace] {
        let decoder = JSONDecoder()
        decoder.keyDecodingStrategy = .convertFromSnakeCase
        return try decoder.decode([KnownPlace].self, from: Data(json.utf8))
    }
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
    /// A pending date suggestion (`when`-shaped): a date read out of
    /// the title as it was typed, waiting on the user's yes or no
    /// (spec/calendar-plan.md "Typed date phrases").
    public let whenSuggested: String?
    /// Whole minutes, when set.
    public let duration: Int?
    public let place: PlaceInfo?
    /// The item shows in Starred: Open, and referenced by the Home
    /// doc's lens (spec/focus.md), whichever workspace it lives in.
    public let starred: Bool

    public var isTimed: Bool { (when?.count ?? 0) >= 16 }

    /// The suggestion to ask about: only on an open item with no date
    /// (a date is a plan; a closed or binned item has none to make, and
    /// a suggestion left beside a set `when` is ignored).
    public var pendingSuggestion: String? {
        when == nil && binnedAt == nil && closedAt == nil ? whenSuggested : nil
    }

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
