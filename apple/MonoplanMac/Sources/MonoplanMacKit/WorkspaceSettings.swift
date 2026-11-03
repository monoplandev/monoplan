// The account-wide UI settings kept in the Home doc, as the
// `DocCmd::Settings` frame reports them (cli/src/commands/settings.rs
// `SettingsJson`). Doc content, synced to every device: the Settings
// window shows and sets `show_list_counts` because that is where a
// user looks for it, not because the app owns it
// (spec/mac-window-plan.md "Settings").

import Foundation

public struct WorkspaceSettings: Decodable, Equatable {
    public let showListCounts: Bool

    enum CodingKeys: String, CodingKey {
        case showListCounts = "show_list_counts"
    }

    public init(showListCounts: Bool) {
        self.showListCounts = showListCounts
    }

    public static func parse(_ json: String) throws -> WorkspaceSettings {
        try JSONDecoder().decode(WorkspaceSettings.self, from: Data(json.utf8))
    }
}
