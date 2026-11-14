// The bin: `monoplan bin show --json` rows (cli/src/commands/bin.rs
// `BinnedJson`), one doc's binned items in no particular order.

import Foundation

public struct BinnedRow: Decodable, Equatable, Identifiable {
    public let id: String
    public let text: String
    public let listId: String
    /// Unix millis the item was binned.
    public let binnedAt: Int64

    public init(id: String, text: String, listId: String, binnedAt: Int64) {
        self.id = id
        self.text = text
        self.listId = listId
        self.binnedAt = binnedAt
    }
}

public enum Bin {
    public static func parse(_ json: String) throws -> [BinnedRow] {
        let decoder = JSONDecoder()
        decoder.keyDecodingStrategy = .convertFromSnakeCase
        return try decoder.decode([BinnedRow].self, from: Data(json.utf8))
    }
}
