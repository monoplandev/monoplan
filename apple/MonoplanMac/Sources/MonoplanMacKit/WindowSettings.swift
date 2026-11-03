// The device settings the app owns for its window and the page cannot
// read for itself (spec/mac-window-plan.md "Settings"): built from
// `UserDefaults`, delivered to the page two ways by the window
// controller. At document start as a user script that sets
// `window.__monoplanSettings`, so the page's module-level readers see
// the app's values on first paint; and live as
// `HostInbound.settings` whenever they change. Appearance is not in
// it: the page follows `NSApp.appearance` through `prefers-color-scheme`.

import Foundation

public struct WindowSettings: Equatable {
    /// Whether the editors recognise a date typed into an item
    /// (`whenToken.ts`); on by default, as in the browser.
    public var typedDates: Bool

    public static let typedDatesKey = "typedDates"
    /// The global the user script sets.
    public static let global = "window.__monoplanSettings"

    public init(typedDates: Bool) {
        self.typedDates = typedDates
    }

    public static func load(from defaults: UserDefaults = .standard) -> WindowSettings {
        WindowSettings(typedDates: defaults.object(forKey: typedDatesKey) as? Bool ?? true)
    }

    public func save(to defaults: UserDefaults = .standard) {
        defaults.set(typedDates, forKey: Self.typedDatesKey)
    }

    /// The object literal, keys sorted so equal settings give equal
    /// text. `HostSettings` on the JS side.
    public var json: String {
        let data = try! JSONSerialization.data(withJSONObject: ["typedDates": typedDates], options: [.sortedKeys])
        return String(decoding: data, as: UTF8.self)
    }

    /// The document-start user script.
    public var userScript: String {
        "\(Self.global) = \(json);"
    }
}
