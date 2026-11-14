// `AppearancePreference` applied: the whole app's appearance, so the
// tray menu, the windows and their sheets all agree. Set at launch from the saved preference and
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

/// The panes' backgrounds. Dark has its own two greys, the item's
/// panel a step lighter than the list; light is the system's.
enum PaneColor {
    static let list = dynamic(dark: 0x12, light: .textBackgroundColor)
    static let panel = dynamic(dark: 0x18, light: .windowBackgroundColor)

    /// The line between two panes: near-black in dark, `system` (the
    /// split view's own) in light.
    static func divider(_ system: NSColor) -> NSColor { dynamic(dark: 0x08, light: system) }

    /// The hairline between two item rows, just visible: the web's
    /// `--row-divider` (`js/web/src/styles.css`).
    static let rowDivider = dynamic(dark: 0x2a, light: grey(0xf5))

    private static func grey(_ value: Int) -> NSColor {
        let level = CGFloat(value) / 255
        return NSColor(srgbRed: level, green: level, blue: level, alpha: 1)
    }

    /// A grey of `dark` in every channel under a dark appearance,
    /// `light` otherwise.
    private static func dynamic(dark: Int, light: NSColor) -> NSColor {
        NSColor(name: nil) { appearance in
            guard appearance.bestMatch(from: [.aqua, .darkAqua]) == .darkAqua else { return light }
            return grey(dark)
        }
    }
}
