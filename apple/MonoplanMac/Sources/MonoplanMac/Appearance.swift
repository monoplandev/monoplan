// `AppearancePreference` applied: the whole app's appearance, so the
// tray menu, the windows and their sheets all agree. Set at launch from the saved preference and
// again from the General pane.

import AppKit
import SwiftUI
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

/// The panes' backgrounds. Dark is the web's (a trial): the sidebar
/// and the item's panel its `--bg` / `--surface`, #1a1a1d
/// (`js/web/src/styles.css`), and the list a step darker, its
/// `--bg-board-col`; light is the system's.
enum PaneColor {
    /// The web's dark `--bg` / `--surface`.
    private static let webDark = 0x1a1a1d

    static let list = dynamic(dark: 0x171718, light: .textBackgroundColor)
    static let panel = dynamic(dark: webDark, light: .windowBackgroundColor)
    /// The sidebar, opaque: off-white in light, the panes' colour in
    /// dark.
    static let sidebar = dynamic(dark: webDark, light: grey(0xf7))

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

    /// A rule across the item's panel: the rows' hairline
    /// (`rowDivider`) in dark; in light, the panes' divider (the system
    /// separator at half its alpha, lighter than `Divider()`'s).
    static let panelRule = NSColor(name: nil) { appearance in
        guard appearance.bestMatch(from: [.aqua, .darkAqua]) != .darkAqua else { return rowDivider }
        var resolved = NSColor.separatorColor
        appearance.performAsCurrentDrawingAppearance {
            resolved = NSColor.separatorColor.usingColorSpace(.sRGB) ?? .separatorColor
        }
        return resolved.withAlphaComponent(resolved.alphaComponent * 0.5)
    }

    /// The hairline between two item rows, just visible: the web's
    /// `--row-divider` (`js/web/src/styles.css`).
    static let rowDivider = dynamic(dark: 0x2a2a2a, light: grey(0xf5))

    /// The ground of a selected row in the key window: the web's
    /// `--dnd-select-bg` (`js/web/src/styles.css`), #dbeafe in light
    /// and #293a5b in dark.
    static let rowSelected = rgb(dark: 0x293a5b, light: 0xdbeafe)

    /// The same in a window that is not key: the web's `--bg-active`,
    /// #e5e7eb in light and #2a2a2e in dark.
    static let rowSelectedInactive = rgb(dark: 0x2a2a2e, light: 0xe5e7eb)

    /// `0xRRGGBB` for each appearance.
    private static func rgb(dark: Int, light: Int) -> NSColor {
        NSColor(name: nil) { appearance in
            let hex = appearance.bestMatch(from: [.aqua, .darkAqua]) == .darkAqua ? dark : light
            return NSColor(
                srgbRed: CGFloat((hex >> 16) & 0xff) / 255, green: CGFloat((hex >> 8) & 0xff) / 255,
                blue: CGFloat(hex & 0xff) / 255, alpha: 1)
        }
    }

    private static func grey(_ value: Int) -> NSColor {
        let level = CGFloat(value) / 255
        return NSColor(srgbRed: level, green: level, blue: level, alpha: 1)
    }

    /// `0xRRGGBB` of `dark` under a dark appearance, `light` otherwise.
    private static func dynamic(dark: Int, light: NSColor) -> NSColor {
        NSColor(name: nil) { appearance in
            guard appearance.bestMatch(from: [.aqua, .darkAqua]) == .darkAqua else { return light }
            return NSColor(
                srgbRed: CGFloat((dark >> 16) & 0xff) / 255, green: CGFloat((dark >> 8) & 0xff) / 255,
                blue: CGFloat(dark & 0xff) / 255, alpha: 1)
        }
    }
}

/// A horizontal rule in the item's panel, `PaneColor.panelRule`.
struct PanelRule: View {
    var body: some View {
        Rectangle().fill(Color(nsColor: PaneColor.panelRule)).frame(height: 1)
    }
}
