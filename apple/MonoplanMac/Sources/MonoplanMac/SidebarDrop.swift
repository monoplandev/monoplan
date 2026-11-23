// Items dragged out of the list on screen and let go on a sidebar row:
// a list or an Inbox (they move there), Starred (they are starred) or
// the Bin (spec/mac-dnd-plan.md phase 3, the item drops). The web's
// `findDropTarget` / `onDndDragEnd` (`js/web/src/Workspace.tsx`), with
// its `data-drop-*` rows.
//
// The drag is `DndListView`'s own (no `NSDraggingSession`), so nothing
// of AppKit's finds the row under it. The rows that take a drop say
// where they are instead: each is a `SidebarDropZone` in the window's
// `SidebarDrops`, asked with the pointer as the drag moves. A SwiftUI
// row carries a `SidebarDropMarker` behind its label; a workspace's
// lists (`SidebarLists`) are one zone for the block.

import AppKit
import SwiftUI

/// What a sidebar row does with items dropped on it.
enum SidebarDropTarget: Equatable {
    case starred
    case bin
    /// A list of a workspace (nil is Home), its Inbox included.
    case list(workspace: String?, list: String)
}

/// A part of the sidebar that takes dropped items.
protocol SidebarDropZone: AnyObject {
    /// The row under `point` (the window's coordinates), if it is one
    /// of this zone's and on screen.
    func dropTarget(at point: NSPoint, in window: NSWindow) -> SidebarDropTarget?
    /// Light the row for `target`; nil puts it out.
    func showDrop(_ target: SidebarDropTarget?)
}

/// The window's drop zones. Main queue.
final class SidebarDrops {
    private struct Entry {
        weak var zone: SidebarDropZone?
    }

    private var zones: [Entry] = []
    private weak var lit: SidebarDropZone?

    func add(_ zone: SidebarDropZone) {
        zones.removeAll { $0.zone == nil || $0.zone === zone }
        zones.append(Entry(zone: zone))
    }

    private func find(at point: NSPoint, in window: NSWindow?) -> (zone: SidebarDropZone, target: SidebarDropTarget)? {
        guard let window else { return nil }
        for entry in zones {
            if let zone = entry.zone, let target = zone.dropTarget(at: point, in: window) {
                return (zone, target)
            }
        }
        return nil
    }

    /// The row a drop at `point` would land on.
    func target(at point: NSPoint, in window: NSWindow?) -> SidebarDropTarget? {
        find(at: point, in: window)?.target
    }

    /// A drag is at `point`: the row under it is lit, and no other.
    func hover(at point: NSPoint, in window: NSWindow?) {
        let found = find(at: point, in: window)
        if let lit, lit !== found?.zone { lit.showDrop(nil) }
        lit = found?.zone
        found?.zone.showDrop(found?.target)
    }

    /// The drag is over.
    func clear() {
        lit?.showDrop(nil)
        lit = nil
    }

    /// Whether `point` is on the part of `rect` (in `view`'s
    /// coordinates) that is on screen in `window`.
    static func shows(_ rect: NSRect, of view: NSView, at point: NSPoint, in window: NSWindow) -> Bool {
        guard view.window === window, !view.isHiddenOrHasHiddenAncestor else { return false }
        let visible = rect.intersection(view.visibleRect)
        return !visible.isEmpty && visible.contains(view.convert(point, from: nil))
    }
}

/// The mark on the row a drop would land on: the web's
/// `.nav-item[data-drop-active]`, the list's selection ground under an
/// outline in the accent colour. Takes no clicks.
class SidebarDropHighlight: NSView {
    var lit = false {
        didSet { if lit != oldValue { needsDisplay = true } }
    }

    override init(frame: NSRect) {
        super.init(frame: frame)
        wantsLayer = true
        layer?.cornerRadius = 8
        layer?.cornerCurve = .continuous
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError("not built from a nib") }

    override var wantsUpdateLayer: Bool { true }

    override func updateLayer() {
        layer?.backgroundColor = lit ? PaneColor.rowSelected.cgColor : nil
        layer?.borderColor = lit ? NSColor.controlAccentColor.cgColor : nil
        layer?.borderWidth = lit ? 1.5 : 0
    }

    override func hitTest(_ point: NSPoint) -> NSView? { nil }
}

/// Behind a SwiftUI sidebar row's label, as large as the row's
/// selection: makes the row a drop zone for `target`.
struct SidebarDropMarker: NSViewRepresentable {
    let drops: SidebarDrops
    let target: SidebarDropTarget

    func makeNSView(context: Context) -> SidebarDropMarkerView {
        let view = SidebarDropMarkerView()
        view.target = target
        drops.add(view)
        return view
    }

    func updateNSView(_ view: SidebarDropMarkerView, context: Context) {
        view.target = target
    }
}

final class SidebarDropMarkerView: SidebarDropHighlight, SidebarDropZone {
    var target: SidebarDropTarget?

    func dropTarget(at point: NSPoint, in window: NSWindow) -> SidebarDropTarget? {
        SidebarDrops.shows(bounds, of: self, at: point, in: window) ? target : nil
    }

    func showDrop(_ target: SidebarDropTarget?) {
        lit = target != nil
    }
}
