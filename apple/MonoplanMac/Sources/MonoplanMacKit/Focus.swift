// The Starred lens as `monoplan focus --json` returns it
// (cli/src/commands/focus.rs `FocusItemJson`): Open referenced items in
// curated order. "Starred" is the UI label; "Focus" is the data-model
// name (spec/focus.md).

import Foundation

public struct FocusRow: Decodable, Equatable {
    public let pos: Int
    public let id: String
    public let text: String
    public let listId: String
    public let state: String
    /// `--all` only: the row's workspace (`docName` nil for Home) and
    /// its list's name there.
    public let docId: String?
    public let docName: String?
    public let listName: String?
}

public enum Focus {
    public static func parse(_ json: String) throws -> [FocusRow] {
        let decoder = JSONDecoder()
        decoder.keyDecodingStrategy = .convertFromSnakeCase
        return try decoder.decode([FocusRow].self, from: Data(json.utf8))
    }
}
