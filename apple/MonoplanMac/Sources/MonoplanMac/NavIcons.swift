// The sidebar's glyphs for Inbox, Calendar, Done and Bin: the web nav's
// (`js/web/src/icons/archive.svg`, `calendar.svg`, `check.svg`,
// `trash.svg`). As with
// the star (`StarIcon.swift`), this target has no asset catalog, so the
// SVGs are here as source and AppKit rasterises them as templates,
// coloured by the foreground style.

import AppKit
import SwiftUI

enum NavArt {
    static let inbox = template(#"""
        <svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 256 256"><rect width="256" height="256" fill="none"/><rect x="40" y="40" width="176" height="176" rx="8" fill="none" stroke="currentColor" stroke-linecap="round" stroke-linejoin="round" stroke-width="16"/><path d="M40,160H76.69a8,8,0,0,1,5.65,2.34l19.32,19.32a8,8,0,0,0,5.65,2.34h41.38a8,8,0,0,0,5.65-2.34l19.32-19.32a8,8,0,0,1,5.65-2.34H216" fill="none" stroke="currentColor" stroke-linecap="round" stroke-linejoin="round" stroke-width="16"/></svg>
        """#)
    static let calendar = template(#"""
        <svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 256 256"><rect width="256" height="256" fill="none"/><rect x="40" y="40" width="176" height="176" rx="8" fill="none" stroke="currentColor" stroke-linecap="round" stroke-linejoin="round" stroke-width="16"/><line x1="176" y1="24" x2="176" y2="56" fill="none" stroke="currentColor" stroke-linecap="round" stroke-linejoin="round" stroke-width="16"/><line x1="80" y1="24" x2="80" y2="56" fill="none" stroke="currentColor" stroke-linecap="round" stroke-linejoin="round" stroke-width="16"/><line x1="40" y1="88" x2="216" y2="88" fill="none" stroke="currentColor" stroke-linecap="round" stroke-linejoin="round" stroke-width="16"/></svg>
        """#)
    static let done = template(#"""
        <svg width="15" height="15" viewBox="0 0 16 16" fill="none" xmlns="http://www.w3.org/2000/svg"><path d="M3.5 8.5l3 3 6-6" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"/></svg>
        """#)
    static let bin = template(#"""
        <svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 256 256"><rect width="256" height="256" fill="none"/><line x1="216" y1="56" x2="40" y2="56" fill="none" stroke="currentColor" stroke-linecap="round" stroke-linejoin="round" stroke-width="16"/><line x1="104" y1="104" x2="104" y2="168" fill="none" stroke="currentColor" stroke-linecap="round" stroke-linejoin="round" stroke-width="16"/><line x1="152" y1="104" x2="152" y2="168" fill="none" stroke="currentColor" stroke-linecap="round" stroke-linejoin="round" stroke-width="16"/><path d="M200,56V208a8,8,0,0,1-8,8H64a8,8,0,0,1-8-8V56" fill="none" stroke="currentColor" stroke-linecap="round" stroke-linejoin="round" stroke-width="16"/><path d="M168,56V40a16,16,0,0,0-16-16H104A16,16,0,0,0,88,40V56" fill="none" stroke="currentColor" stroke-linecap="round" stroke-linejoin="round" stroke-width="16"/></svg>
        """#)

    private static func template(_ svg: String) -> NSImage {
        let image = NSImage(data: Data(svg.utf8)) ?? NSImage()
        image.isTemplate = true
        return image
    }
}

/// One of `NavArt`'s glyphs at a label's size: 16 pt, the web's
/// `.nav-item-icon svg`.
struct NavIcon: View {
    let image: NSImage
    var size: CGFloat = 16

    var body: some View {
        Image(nsImage: image)
            .resizable()
            .frame(width: size, height: size)
    }
}
