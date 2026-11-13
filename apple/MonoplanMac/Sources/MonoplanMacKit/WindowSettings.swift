// The device settings the app owns for its window and the page cannot
// read for itself (spec/mac-window-plan.md "Settings"): built from
// `UserDefaults` and the system (the app's language, the locale's
// hour cycle), delivered to the page two ways by the window
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
    /// Whether the sidebar counts each list's open items
    /// (`listCounts.ts`); off by default, as in the browser. Inbox and
    /// Starred are counted either way.
    public var listCounts: Bool
    /// The page's language (`i18n.tsx`): the app's own, "es" when the
    /// app runs in Spanish, "en" otherwise (System Settings › Language
    /// & Region › Applications; no setting of the app's own).
    public var language: String
    /// "12h" or "24h" (`format.tsx`): the system's, from the locale
    /// (the 24-Hour Time switch included).
    public var hourCycle: String

    public static let typedDatesKey = "typedDates"
    public static let listCountsKey = "listCounts"
    /// The global the user script sets.
    public static let global = "window.__monoplanSettings"

    public init(typedDates: Bool, listCounts: Bool = false, language: String, hourCycle: String) {
        self.typedDates = typedDates
        self.listCounts = listCounts
        self.language = language
        self.hourCycle = hourCycle
    }

    /// `localizations` is `Bundle.main.preferredLocalizations`: the
    /// bundle's languages ordered by the user's preference, so its
    /// first is the one the app's own strings show in.
    public static func load(
        from defaults: UserDefaults = .standard, locale: Locale = .current,
        localizations: [String] = Bundle.main.preferredLocalizations
    ) -> WindowSettings {
        WindowSettings(
            typedDates: defaults.object(forKey: typedDatesKey) as? Bool ?? true,
            listCounts: defaults.object(forKey: listCountsKey) as? Bool ?? false,
            language: language(of: localizations),
            hourCycle: hourCycle(of: locale))
    }

    public static func language(of localizations: [String]) -> String {
        let code = localizations.first.flatMap { Locale(identifier: $0).language.languageCode?.identifier }
        return code == "es" ? "es" : "en"
    }

    public static func hourCycle(of locale: Locale) -> String {
        switch locale.hourCycle {
        case .oneToTwelve, .zeroToEleven: return "12h"
        default: return "24h"
        }
    }

    /// Writes what is the app's own; the language and the hour cycle
    /// are the system's and never written.
    public func save(to defaults: UserDefaults = .standard) {
        defaults.set(typedDates, forKey: Self.typedDatesKey)
        defaults.set(listCounts, forKey: Self.listCountsKey)
    }

    /// The object literal, keys sorted so equal settings give equal
    /// text. `HostSettings` on the JS side.
    public var json: String {
        let object: [String: Any] = [
            "typedDates": typedDates, "listCounts": listCounts, "language": language, "hourCycle": hourCycle,
        ]
        let data = try! JSONSerialization.data(withJSONObject: object, options: [.sortedKeys])
        return String(decoding: data, as: UTF8.self)
    }

    /// The document-start user script.
    public var userScript: String {
        "\(Self.global) = \(json);"
    }
}
