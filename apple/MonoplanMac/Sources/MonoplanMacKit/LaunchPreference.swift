// Whether launching the app opens the window (spec/mac-window-plan.md
// "Settings"). On by default: the app is a tray process, but a user who
// double-clicks it, or has it open at login, is nearly always after
// the window; off makes a launch tray-only, as the first pass was. The
// tray opens the window once the host has settled rather than at once,
// so the page does not boot against no host and flash its error
// before the reload. Pure data here, applied in the app target.

import Foundation

public enum LaunchPreference {
    /// The `UserDefaults` key; absent reads as on.
    public static let openWindowKey = "openWindowAtLaunch"

    public static func openWindow(from defaults: UserDefaults = .standard) -> Bool {
        defaults.object(forKey: openWindowKey) as? Bool ?? true
    }

    public static func setOpenWindow(_ on: Bool, to defaults: UserDefaults = .standard) {
        defaults.set(on, forKey: openWindowKey)
    }
}
