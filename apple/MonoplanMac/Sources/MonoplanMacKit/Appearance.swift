// The Appearance preference: the first of the device preferences the
// app owns. Pure data here, the `NSAppearance` it maps to in the app
// target.

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
