// A list's items as `monoplan item ls --json` returns them
// (cli/src/commands/items.rs `ItemJson`), in list order. Only the
// fields a row shows are decoded; the rest (place, duration, the
// per-state stamps) wait for a view that needs them.

import Foundation

public struct ItemRow: Decodable, Equatable, Identifiable {
    public let id: String
    public let text: String
    public let listId: String
    /// Workflow state name (`spec/data-model.md` "Lifecycle"):
    /// `backlog`, `todo`, `in_progress`, `review`, `done`, `cancelled`.
    public let state: String
    /// Unix millis the workflow state was entered: for a Done item, the
    /// moment it was ticked.
    public let lifecycleAt: Int64
    /// Unix millis.
    public let createdAt: Int64
    /// `YYYY-MM-DD`, when set.
    public let deadline: String?
    /// `YYYY-MM-DD` or `YYYY-MM-DDTHH:MM`, when set.
    public let when: String?

    public init(id: String, text: String, listId: String, state: String, lifecycleAt: Int64 = 0,
                createdAt: Int64, deadline: String? = nil, when: String? = nil) {
        self.id = id
        self.text = text
        self.listId = listId
        self.state = state
        self.lifecycleAt = lifecycleAt
        self.createdAt = createdAt
        self.deadline = deadline
        self.when = when
    }
}

public enum Items {
    public static func parse(_ json: String) throws -> [ItemRow] {
        let decoder = JSONDecoder()
        decoder.keyDecodingStrategy = .convertFromSnakeCase
        return try decoder.decode([ItemRow].self, from: Data(json.utf8))
    }
}
