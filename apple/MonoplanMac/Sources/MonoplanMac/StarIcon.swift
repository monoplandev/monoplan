// The app's star: the web's glyph (`js/web/src/icons/star-filled.svg`
// and its outline), the same three drawings the iOS asset catalog has
// as `Star`, `StarFill` and `StarStroke`. This target is a bare SwiftPM
// executable with no asset catalog of its own, so the SVGs are here as
// source and AppKit rasterises them; all three are templates, coloured
// by the foreground style.

import AppKit
import SwiftUI

enum StarArt {
    /// The outline alone, for a label (the sidebar's Starred row).
    static let outline = template(#"""
        <svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 256 256" fill="currentColor"><path d="M239.18,97.26A16.38,16.38,0,0,0,224.92,86l-59-4.76L143.14,26.15a16.36,16.36,0,0,0-30.27,0L90.11,81.23,31.08,86a16.46,16.46,0,0,0-9.37,28.86l45,38.83L53,211.75a16.38,16.38,0,0,0,24.5,17.82L128,198.49l50.53,31.08A16.4,16.4,0,0,0,203,211.75l-13.76-58.07,45-38.83A16.43,16.43,0,0,0,239.18,97.26Zm-15.34,5.47-48.7,42a8,8,0,0,0-2.56,7.91l14.88,62.8a.37.37,0,0,1-.17.48c-.18.14-.23.11-.38,0l-54.72-33.65a8,8,0,0,0-8.38,0L69.09,215.94c-.15.09-.19.12-.38,0a.37.37,0,0,1-.17-.48l14.88-62.8a8,8,0,0,0-2.56-7.91l-48.7-42c-.12-.1-.23-.19-.13-.5s.18-.27.33-.29l63.92-5.16A8,8,0,0,0,103,91.86l24.62-59.61c.08-.17.11-.25.35-.25s.27.08.35.25L153,91.86a8,8,0,0,0,6.75,4.92l63.92,5.16c.15,0,.24,0,.33.29S224,102.63,223.84,102.73Z"/></svg>
        """#)
    /// The solid star, to be tinted.
    static let fill = template(#"""
        <svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 256 256" fill="currentColor"><path d="M234.29,114.85l-45,38.83L203,211.75a16.4,16.4,0,0,1-24.5,17.82L128,198.49,77.47,229.57A16.4,16.4,0,0,1,53,211.75l13.76-58.07-45-38.83A16.46,16.46,0,0,1,31.08,86l59-4.76,22.76-55.08a16.36,16.36,0,0,1,30.27,0l22.75,55.08,59,4.76a16.46,16.46,0,0,1,9.37,28.86Z"/></svg>
        """#)
    /// A thin outline to lay over the fill.
    static let stroke = template(#"""
        <svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 256 256" fill="none" stroke="#000" stroke-width="6"><path d="M234.29,114.85l-45,38.83L203,211.75a16.4,16.4,0,0,1-24.5,17.82L128,198.49,77.47,229.57A16.4,16.4,0,0,1,53,211.75l13.76-58.07-45-38.83A16.46,16.46,0,0,1,31.08,86l59-4.76,22.76-55.08a16.36,16.36,0,0,1,30.27,0l22.75,55.08,59,4.76a16.46,16.46,0,0,1,9.37,28.86Z"/></svg>
        """#)

    /// The star for a toolbar button: the outline as a template (the
    /// bar tints it), and when starred the yellow fill under an
    /// outline in the appearance's own ink, as `StarGlyph`.
    static func toolbarImage(starred: Bool) -> NSImage {
        let size = NSSize(width: 18, height: 18)
        guard starred else {
            let image = stroke.copy() as? NSImage ?? stroke
            image.size = size
            return image
        }
        let yellow = tinted(fill, NSColor(red: 1.0, green: 0.906, blue: 0.522, alpha: 1), size: size)
        let dark = tinted(stroke, .white, size: size)
        let light = tinted(stroke, .black, size: size)
        return NSImage(size: size, flipped: false) { rect in
            yellow.draw(in: rect)
            let isDark = NSAppearance.currentDrawing().bestMatch(from: [.darkAqua, .aqua]) == .darkAqua
            (isDark ? dark : light).draw(in: rect, from: .zero, operation: .sourceOver, fraction: 0.7)
            return true
        }
    }

    /// `image`'s shape in one colour, as a bitmap.
    private static func tinted(_ image: NSImage, _ color: NSColor, size: NSSize) -> NSImage {
        let scale = 3
        let out = NSImage(size: size)
        guard let rep = NSBitmapImageRep(
            bitmapDataPlanes: nil, pixelsWide: Int(size.width) * scale, pixelsHigh: Int(size.height) * scale,
            bitsPerSample: 8, samplesPerPixel: 4, hasAlpha: true, isPlanar: false,
            colorSpaceName: .deviceRGB, bytesPerRow: 0, bitsPerPixel: 0),
            let context = NSGraphicsContext(bitmapImageRep: rep)
        else { return out }
        rep.size = size
        NSGraphicsContext.saveGraphicsState()
        NSGraphicsContext.current = context
        let rect = NSRect(origin: .zero, size: size)
        image.draw(in: rect)
        color.set()
        rect.fill(using: .sourceIn)
        NSGraphicsContext.restoreGraphicsState()
        out.addRepresentation(rep)
        return out
    }

    private static func template(_ svg: String) -> NSImage {
        let image = NSImage(data: Data(svg.utf8)) ?? NSImage()
        image.isTemplate = true
        return image
    }
}

/// The outline star at a label's size.
struct StarOutline: View {
    var size: CGFloat = 16

    var body: some View {
        Image(nsImage: StarArt.outline)
            .resizable()
            .frame(width: size, height: size)
    }
}

/// An item's star, as on its iOS page (`StarButton` in `ItemView.swift`)
/// and the web dialog: a thin outline that fills yellow when starred.
struct StarGlyph: View {
    let starred: Bool
    var size: CGFloat = 16

    /// The web's `#ffe785`.
    static let fill = Color(red: 1.0, green: 0.906, blue: 0.522)

    var body: some View {
        ZStack {
            Image(nsImage: StarArt.fill)
                .resizable()
                .foregroundStyle(starred ? Self.fill : .clear)
            Image(nsImage: StarArt.stroke)
                .resizable()
                .foregroundStyle(Color.primary.opacity(0.7))
        }
        .frame(width: size, height: size)
    }
}
