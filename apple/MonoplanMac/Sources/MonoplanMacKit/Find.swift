// Search results as `monoplan find <query> --json` returns them
// (cli/src/commands/find.rs `HitJson`): the search index of every open
// doc (spec/search.md), merged and ranked, lists above items, each row
// tagged with its workspace.

import Foundation

public struct SearchHit: Decodable, Equatable, Identifiable {
    public enum Kind: String, Decodable {
        case item, list
    }

    public let kind: Kind
    /// The item's or the list's id; unique only within its doc.
    public let itemId: String
    public let title: String
    /// The row's workspace; `docName` is nil for Personal and for an
    /// unnamed doc.
    public let docId: String?
    public let docName: String?
    /// Items: the list the item is in, and its name there.
    public let listId: String?
    public let listName: String?
    /// Items: the lifecycle's name, the workflow states plus `binned`.
    public let state: String?
    /// Lists: the list's emoji, when it has one.
    public let icon: String?
    /// The list, or the item's list, is archived.
    public let archived: Bool

    public var id: String { "\(docId ?? "")/\(kind.rawValue)/\(itemId)" }

    enum CodingKeys: String, CodingKey {
        case kind, title, state, icon, archived
        case itemId = "id"
        case docId = "doc_id"
        case docName = "doc_name"
        case listId = "list_id"
        case listName = "list_name"
    }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        kind = try c.decode(Kind.self, forKey: .kind)
        itemId = try c.decode(String.self, forKey: .itemId)
        title = try c.decode(String.self, forKey: .title)
        docId = try c.decodeIfPresent(String.self, forKey: .docId)
        docName = try c.decodeIfPresent(String.self, forKey: .docName)
        listId = try c.decodeIfPresent(String.self, forKey: .listId)
        listName = try c.decodeIfPresent(String.self, forKey: .listName)
        state = try c.decodeIfPresent(String.self, forKey: .state)
        icon = try c.decodeIfPresent(String.self, forKey: .icon)
        // Absent means false on the wire.
        archived = try c.decodeIfPresent(Bool.self, forKey: .archived) ?? false
    }
}

public enum Find {
    public static func parse(_ json: String) throws -> [SearchHit] {
        try JSONDecoder().decode([SearchHit].self, from: Data(json.utf8))
    }
}
