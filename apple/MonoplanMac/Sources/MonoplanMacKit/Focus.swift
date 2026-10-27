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
}

public enum Focus {
    public static func parse(_ json: String) throws -> [FocusRow] {
        let decoder = JSONDecoder()
        decoder.keyDecodingStrategy = .convertFromSnakeCase
        return try decoder.decode([FocusRow].self, from: Data(json.utf8))
    }
}

public enum StarredMenu {
    /// How many starred rows the tray shows: the top of the list is the
    /// point of curating it.
    public static let limit = 3

    /// `listNames` is id → name; the item's list is shown as the muted
    /// note, or nothing when the id is unknown (a list the fetch missed
    /// is not worth showing a uuid for).
    public static func lines(from rows: [FocusRow], listNames: [String: String] = [:], limit: Int = limit) -> [MenuLine] {
        rows.prefix(limit).map { row in
            MenuLine(
                id: row.id,
                title: TodayMenu.clipped(row.text),
                note: listNames[row.listId].map(TodayMenu.clipped),
                done: false,
                overdue: false
            )
        }
    }
}
