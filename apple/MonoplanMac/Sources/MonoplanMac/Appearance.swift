// `AppearancePreference` applied: the whole app's appearance, so the
// tray menu, the Settings window, sheets, the traffic lights and the
// window's webview (whose `prefers-color-scheme` follows its effective
// appearance) all agree. Set at launch from the saved preference and
// again from the General pane.

import AppKit
import MonoplanMacKit

extension AppearancePreference {
    var nsAppearance: NSAppearance? {
        switch self {
        case .system: return nil
        case .light: return NSAppearance(named: .aqua)
        case .dark: return NSAppearance(named: .darkAqua)
        }
    }

    /// Save and apply.
    func activate() {
        save()
        NSApp.appearance = nsAppearance
    }

    static func applySaved() {
        NSApp.appearance = load().nsAppearance
    }
}
