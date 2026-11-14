// The Open items that carry a place, as `monoplan places --items
// --json` returns them (cli/src/commands/items.rs `PlacedItemJson`):
// the `item ls --json` row with its place, plus its list's name and
// its workspace. Every open workspace, each in its own item order.
// What the iOS map reads; a place may have no coordinates, which the
// reader leaves off the map.

import Foundation

public struct PlacedItem: Decodable, Equatable {
    public let id: String
    public let text: String
    public let listId: String
    /// Workflow state name, as `ItemRow.state`; never a closed one.
    public let state: String
    /// `YYYY-MM-DD` or `YYYY-MM-DDTHH:MM`, when set.
    public let when: String?
    /// `YYYY-MM-DD`, when set.
    public let deadline: String?
    public let place: PlaceInfo
    /// Nil for a list the engine could not name.
    public let listName: String?
    /// The row's workspace; `docName` is nil for Personal.
    public let docId: String?
    public let docName: String?

    /// The row's identity across workspaces: an item id alone can be
    /// in two docs for a moment while it moves between them.
    public var key: String { "\(docId ?? "")/\(id)" }

    public init(id: String, text: String, listId: String, state: String, when: String? = nil,
                deadline: String? = nil, place: PlaceInfo, listName: String? = nil,
                docId: String? = nil, docName: String? = nil) {
        self.id = id
        self.text = text
        self.listId = listId
        self.state = state
        self.when = when
        self.deadline = deadline
        self.place = place
        self.listName = listName
        self.docId = docId
        self.docName = docName
    }
}

public enum Placed {
    public static func parse(_ json: String) throws -> [PlacedItem] {
        let decoder = JSONDecoder()
        decoder.keyDecodingStrategy = .convertFromSnakeCase
        return try decoder.decode([PlacedItem].self, from: Data(json.utf8))
    }
}
