// The Appearance preference (spec/mac-window-plan.md "Settings"): the
// first of the device preferences the app owns for its window. Pure
// data here, the `NSAppearance` it maps to in the app target. The
// window's page needs nothing from it: the app sets `NSApp.appearance`
// and the webview's `prefers-color-scheme` follows its effective
// appearance, so the page stays on "auto".

import Foundation

public enum AppearancePreference: String, CaseIterable, Equatable {
    /// Follow the system (`NSApp.appearance = nil`).
    case system
    case light
    case dark

    /// The `UserDefaults` key; absent or unknown reads as `.system`.
    public static let defaultsKey = "appearance"

    public static func load(from defaults: UserDefaults = .standard) -> AppearancePreference {
        guard let raw = defaults.string(forKey: defaultsKey) else { return .system }
        return AppearancePreference(rawValue: raw) ?? .system
    }

    /// `.system` removes the key rather than writing it, so a default
    /// that changes later is picked up.
    public func save(to defaults: UserDefaults = .standard) {
        if self == .system {
            defaults.removeObject(forKey: Self.defaultsKey)
        } else {
            defaults.set(rawValue, forKey: Self.defaultsKey)
        }
    }
}
