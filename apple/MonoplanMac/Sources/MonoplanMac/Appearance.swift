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
/// its `--bg` / `--surface`, #1a1a1d (`js/web/src/styles.css`), and
/// the list a step darker in the same cool grey, #161619; light is the
/// system's. The item's panel shares the list's ground.
enum PaneColor {
    /// The web's dark `--bg` / `--surface`.
    private static let webDark = 0x1a1a1d

    static let list = dynamic(dark: 0x161619, light: .textBackgroundColor)
    static let panel = list
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

    /// A rule across the item's panel, just visible: in dark, half the
    /// contrast of the web's `--row-divider` (#2a2a2a) against the
    /// panel's ground; in light, the system separator at a quarter of
    /// its alpha (half the panes' divider).
    static let panelRule = NSColor(name: nil) { appearance in
        guard appearance.bestMatch(from: [.aqua, .darkAqua]) != .darkAqua else { return grey(0x22) }
        var resolved = NSColor.separatorColor
        appearance.performAsCurrentDrawingAppearance {
            resolved = NSColor.separatorColor.usingColorSpace(.sRGB) ?? .separatorColor
        }
        return resolved.withAlphaComponent(resolved.alphaComponent * 0.25)
    }

    /// An item row's title: in dark, the system label (white at 85%)
    /// dimmed about 15%; light is the system label.
    static let rowText = NSColor(name: nil) { appearance in
        guard appearance.bestMatch(from: [.aqua, .darkAqua]) == .darkAqua else { return .labelColor }
        return NSColor(white: 1, alpha: 0.72)
    }

    /// A sidebar row's label and glyph: the item rows' text, so the nav
    /// reads as the lists do.
    static let navText = rowText

    /// The same in a window that is not key: dimmer, white at 45% in
    /// dark (from 72%), black at 50% in light (from the label's 85%).
    static let navTextInactive = NSColor(name: nil) { appearance in
        appearance.bestMatch(from: [.aqua, .darkAqua]) == .darkAqua
            ? NSColor(white: 1, alpha: 0.45)
            : NSColor(white: 0, alpha: 0.5)
    }

    /// A sidebar row's count, active and not.
    static let navCount = NSColor.secondaryLabelColor
    static let navCountInactive = NSColor.tertiaryLabelColor

    /// The hairline between two item rows, just visible: half the
    /// contrast of the web's `--row-divider` (`js/web/src/styles.css`,
    /// #2a2a2a / #f5f5f5) against the list's ground.
    static let rowDivider = dynamic(dark: 0x212121, light: grey(0xfa))

    /// The ground of a selected row in the key window: the web's
    /// `--dnd-select-bg` (`js/web/src/styles.css`), #dbeafe in light
    /// and #293a5b in dark.
    static let rowSelected = rgb(dark: 0x293a5b, light: 0xdbeafe)

    /// The same in a window that is not key: the web's `--bg-active`,
    /// #e5e7eb in light and #2a2a2e in dark.
    static let rowSelectedInactive = rgb(dark: 0x2a2a2e, light: 0xe5e7eb)

    /// Where dragged rows will land: the web's
    /// `--dnd-placeholder-color`, #cbd5e1 in light and #475569 in dark.
    static let dropSlot = rgb(dark: 0x475569, light: 0xcbd5e1)

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
