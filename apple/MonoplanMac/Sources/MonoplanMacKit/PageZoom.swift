// The window's page zoom (View › Zoom In / Zoom Out / Actual Size, ⌘+
// ⌘- ⌘0): Safari's fixed ladder of levels, applied as the webview's
// `pageZoom` so the page reflows at the new size rather than scaling
// like an image (`magnification`). One level for the app, not per
// site: there is one page. Pure data here, applied in the app target.
// `WKWebView` starts every page at 1, so the app re-applies the saved
// level when it makes the webview.

import Foundation

public enum PageZoom {
    /// The `UserDefaults` key; absent or out of range reads as 1.
    public static let defaultsKey = "pageZoom"

    /// 10% steps, two either side of 1: the page's layout is tuned for
    /// its own size and the window's chrome (the glass under the
    /// sidebar) tracks it, so the range stays small.
    public static let levels: [Double] = [0.8, 0.9, 1, 1.1, 1.2]

    public static func load(from defaults: UserDefaults = .standard) -> Double {
        guard let v = defaults.object(forKey: defaultsKey) as? Double, levels.contains(v) else { return 1 }
        return v
    }

    /// 1 removes the key rather than writing it, like the other
    /// preferences' defaults.
    public static func save(_ zoom: Double, to defaults: UserDefaults = .standard) {
        if zoom == 1 {
            defaults.removeObject(forKey: defaultsKey)
        } else {
            defaults.set(zoom, forKey: defaultsKey)
        }
    }

    /// The next level up from `zoom`, or `zoom` itself at the top. A
    /// value off the ladder snaps to the first level above it.
    public static func stepIn(from zoom: Double) -> Double {
        levels.first { $0 > zoom + 0.001 } ?? zoom
    }

    /// The next level down from `zoom`, or `zoom` itself at the bottom.
    public static func stepOut(from zoom: Double) -> Double {
        levels.last { $0 < zoom - 0.001 } ?? zoom
    }
}
