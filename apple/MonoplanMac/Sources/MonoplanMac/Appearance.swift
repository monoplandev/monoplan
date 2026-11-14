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

    /// The line between two panes: near-black in dark; in light,
    /// `system` (the split view's own, black at 10%) at half its alpha.
    static func divider(_ system: NSColor) -> NSColor {
        NSColor(name: nil) { appearance in
            guard appearance.bestMatch(from: [.aqua, .darkAqua]) != .darkAqua else { return grey(0x08) }
            var resolved = system
            appearance.performAsCurrentDrawingAppearance {
                resolved = system.usingColorSpace(.sRGB) ?? system
            }
            return resolved.withAlphaComponent(resolved.alphaComponent * 0.5)
        }
    }

    /// A selected row's gray, `system` (the row's own) brightened by
    /// half: in light halfway to white (#dcdcdc to #eeeeee), in dark
    /// half as bright again (#464646 to #696969).
    static func rowSelection(_ system: NSColor) -> NSColor {
        NSColor(name: nil) { appearance in
            var resolved = system
            appearance.performAsCurrentDrawingAppearance {
                resolved = system.usingColorSpace(.sRGB) ?? system
            }
            guard appearance.bestMatch(from: [.aqua, .darkAqua]) == .darkAqua else {
                return resolved.blended(withFraction: 0.5, of: .white) ?? resolved
            }
            return NSColor(
                srgbRed: min(resolved.redComponent * 1.5, 1), green: min(resolved.greenComponent * 1.5, 1),
                blue: min(resolved.blueComponent * 1.5, 1), alpha: resolved.alphaComponent)
        }
    }

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
