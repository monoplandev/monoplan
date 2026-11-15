// The sidebar's glyphs for Inbox, Calendar, Done and Bin: the web nav's
// (`js/web/src/icons/archive.svg`, `calendar.svg`, `check.svg`,
// `trash.svg`), and the place pin (`map-pin.svg`, iOS's `MapPin`) the
// item panel uses beside them. As with
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

    static let pin = template(#"""
        <svg width="15" height="15" viewBox="0 0 15 15" fill="none" xmlns="http://www.w3.org/2000/svg"><path d="M7.5 0.875C4.73858 0.875 2.5 3.11358 2.5 5.875C2.5 7.6287 3.44486 9.3395 4.52533 10.7146C5.60872 12.0934 6.85183 13.1738 7.44627 13.6632C7.47955 13.6906 7.52045 13.6906 7.55373 13.6632C8.14817 13.1738 9.39128 12.0934 10.4747 10.7146C11.5551 9.3395 12.5 7.6287 12.5 5.875C12.5 3.11358 10.2614 0.875 7.5 0.875ZM1.5 5.875C1.5 2.5613 4.1863 -0.125 7.5 -0.125C10.8137 -0.125 13.5 2.5613 13.5 5.875C13.5 7.9507 12.3892 9.8856 11.2611 11.3324C10.1296 12.7813 8.83152 13.9079 8.18927 14.4365C7.78559 14.7688 7.21441 14.7688 6.81073 14.4365C6.16848 13.9079 4.87039 12.7813 3.73889 11.3324C2.61076 9.8856 1.5 7.9507 1.5 5.875ZM7.5 4.125C6.53350 4.125 5.75 4.9085 5.75 5.875C5.75 6.8415 6.53350 7.625 7.5 7.625C8.4665 7.625 9.25 6.8415 9.25 5.875C9.25 4.9085 8.4665 4.125 7.5 4.125ZM4.75 5.875C4.75 4.35622 5.98122 3.125 7.5 3.125C9.01878 3.125 10.25 4.35622 10.25 5.875C10.25 7.39378 9.01878 8.625 7.5 8.625C5.98122 8.625 4.75 7.39378 4.75 5.875Z" fill="currentColor" fill-rule="evenodd" clip-rule="evenodd"/></svg>
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
