// Search, as Spotlight's bar: a floating panel over the window's top
// with one field (at the system's text size, not Spotlight's large
// type) and, once something is typed, the results under
// it. The toolbar's magnifying glass and Edit › Find… (⌘F) open it;
// Escape, a click anywhere else, or a pick shuts it.
//
// It works as iOS's search does (spec/search.md; `Engine.find`): the
// built-in pages whose localized names match (the core's
// `matchesName`), then lists, then items, ranked across every open
// workspace, every word matching and the last one as a prefix. With
// nothing typed there is only the field, as in Spotlight. ↑ and ↓ move
// the highlight, Return opens it; a page or list shows in the window,
// an item opens its panel over where its row lives
// (`MainWindowModel.open`).
//
// The panel is a borderless child window of the main one, so it moves
// with it and is gone when it closes; the system's glass behind it.

import AppKit
import Combine
import MonoplanCore
import SwiftUI

/// What one result row opens.
enum SearchRow: Identifiable {
    case lens(Lens)
    /// Home's Inbox: like the lenses, a built-in page with a label.
    case inbox
    case hit(FindHit)

    var id: String {
        switch self {
        case .lens(let lens): return "lens/\(lens)"
        case .inbox: return "inbox"
        case .hit(let hit): return "\(hit.docId ?? "")/\(hit.kind)/\(hit.id)"
        }
    }
}

/// The field's text and what it found.
final class SearchModel: ObservableObject {
    @Published var query = "" {
        didSet { if query != oldValue { refresh() } }
    }
    @Published private(set) var rows: [SearchRow] = []
    @Published var highlighted = 0
    /// The engine has answered for what is typed: "no results" waits
    /// for it, so typing does not flicker the message.
    @Published private(set) var answered = true

    private let window: MainWindowModel
    /// The hits of the last answer, kept until the next one lands.
    private var hits: [FindHit] = []

    static let limit: UInt32 = 50

    init(window: MainWindowModel) {
        self.window = window
    }

    var trimmed: String { query.trimmingCharacters(in: .whitespacesAndNewlines) }

    /// The built-in pages whose label matches: not doc entities, so the
    /// index never has them and they are matched here with its rule.
    /// The Bin only while it has something, as in the sidebar.
    private func builtIns(_ query: String) -> [SearchRow] {
        var pages: [(String, SearchRow)] = [
            (String(localized: "Starred"), .lens(.starred)),
            (String(localized: "Calendar"), .lens(.agenda)),
            (String(localized: "Done"), .lens(.done)),
        ]
        // Assigned only while something is, as in the sidebar.
        if !window.assigned.isEmpty { pages.insert((String(localized: "Assigned"), .lens(.assigned)), at: 2) }
        if !window.binned.isEmpty { pages.append((String(localized: "Bin"), .lens(.bin))) }
        pages.append((String(localized: "Inbox"), .inbox))
        return pages.filter { matchesName(name: $0.0, query: query) }.map(\.1)
    }

    private func refresh() {
        let query = trimmed
        guard !query.isEmpty else {
            hits = []
            answered = true
            publish(query, keep: false)
            return
        }
        answered = false
        // The pages at once; the old hits stay under them until the
        // engine answers for this query.
        publish(query, keep: false)
        window.find(query, limit: Self.limit) { [weak self] found in
            guard let self, query == self.trimmed else { return }
            self.hits = found ?? []
            self.answered = true
            self.publish(query, keep: true)
        }
    }

    /// The rows for `query`. The highlight goes back to the top as the
    /// query changes, the best match being what Return should open;
    /// when the engine's answer lands for the same query (`keep`), a
    /// row moved to by hand stays highlighted.
    private func publish(_ query: String, keep: Bool) {
        let old = keep && rows.indices.contains(highlighted) ? rows[highlighted].id : nil
        rows = query.isEmpty ? [] : builtIns(query) + hits.map(SearchRow.hit)
        highlighted = rows.firstIndex { $0.id == old } ?? 0
    }

    func move(_ by: Int) {
        guard !rows.isEmpty else { return }
        highlighted = min(max(highlighted + by, 0), rows.count - 1)
    }

    var picked: SearchRow? { rows.indices.contains(highlighted) ? rows[highlighted] : nil }
}

/// Opens, places and shuts the panel. One per window.
final class SearchPanelController: NSObject, NSWindowDelegate {
    private let model: MainWindowModel
    private var panel: SearchPanel?
    private var search: SearchModel?
    private var sizing: AnyCancellable?
    private weak var parent: NSWindow?

    static let width: CGFloat = 640
    /// From the window's top edge to the panel's: just under the bar.
    static let drop: CGFloat = 96

    init(model: MainWindowModel) {
        self.model = model
    }

    var isShown: Bool { panel != nil }

    /// When a click elsewhere last shut it. A click on the toolbar's
    /// button shuts it that way just before the button's action, which
    /// should then leave it shut rather than open it again.
    private var resignedAt = Date.distantPast

    func toggle(over window: NSWindow) {
        if isShown {
            close()
        } else if Date().timeIntervalSince(resignedAt) > 0.3 {
            show(over: window)
        }
    }

    func show(over window: NSWindow) {
        guard panel == nil else {
            panel?.makeKeyAndOrderFront(nil)
            return
        }
        let search = SearchModel(window: model)
        let panel = SearchPanel()
        panel.delegate = self
        let view = SearchOverlay(search: search, pick: { [weak self] in self?.pick($0) }, close: { [weak self] in self?.close() })
        let host = NSHostingView(rootView: view)
        host.sizingOptions = []
        panel.contentView = SearchPanel.backdrop(around: host)
        self.panel = panel
        self.search = search
        parent = window
        let width = min(Self.width, window.frame.width - 40)
        place(panel, width: width, height: SearchOverlay.height(search), over: window)
        window.addChildWindow(panel, ordered: .above)
        panel.makeKeyAndOrderFront(nil)
        sizing = search.objectWillChange
            .receive(on: DispatchQueue.main)
            .sink { [weak self, weak search] in
                guard let self, let search, let panel = self.panel, let window = self.parent else { return }
                self.place(panel, width: panel.frame.width, height: SearchOverlay.height(search), over: window)
            }
    }

    /// Centred over the window, its top a fixed drop under the window's
    /// top, growing downwards.
    private func place(_ panel: NSPanel, width: CGFloat, height: CGFloat, over window: NSWindow) {
        let frame = window.frame
        let rect = NSRect(
            x: frame.midX - width / 2, y: frame.maxY - Self.drop - height, width: width, height: height)
        guard rect != panel.frame else { return }
        panel.setFrame(rect, display: true)
        panel.invalidateShadow()
    }

    private func pick(_ row: SearchRow) {
        close()
        switch row {
        case .lens(let lens): model.select(lens: lens)
        case .inbox: model.select(workspace: nil, list: MainWindowModel.inbox)
        case .hit(let hit): model.open(hit)
        }
    }

    func close() {
        guard let panel else { return }
        self.panel = nil
        search = nil
        sizing = nil
        panel.delegate = nil
        parent?.removeChildWindow(panel)
        panel.orderOut(nil)
        parent?.makeKey()
    }

    /// A click anywhere else, or another app, shuts it.
    func windowDidResignKey(_ notification: Notification) {
        resignedAt = Date()
        close()
    }
}

/// A borderless panel that still takes the keyboard.
final class SearchPanel: NSPanel {
    init() {
        super.init(contentRect: .zero, styleMask: [.borderless], backing: .buffered, defer: false)
        isOpaque = false
        backgroundColor = .clear
        hasShadow = true
        isReleasedWhenClosed = false
        hidesOnDeactivate = true
        animationBehavior = .utilityWindow
    }

    override var canBecomeKey: Bool { true }
    override var canBecomeMain: Bool { false }

    static let radius: CGFloat = 16

    /// The system's glass behind `content`, rounded: Liquid Glass on
    /// macOS 26, the popover material before it.
    static func backdrop(around content: NSView) -> NSView {
        if #available(macOS 26.0, *) {
            let glass = NSGlassEffectView()
            glass.cornerRadius = radius
            glass.contentView = content
            return glass
        }
        let effect = NSVisualEffectView()
        effect.material = .popover
        effect.blendingMode = .behindWindow
        effect.state = .active
        effect.maskImage = roundedMask(radius)
        content.translatesAutoresizingMaskIntoConstraints = false
        effect.addSubview(content)
        NSLayoutConstraint.activate([
            content.leadingAnchor.constraint(equalTo: effect.leadingAnchor),
            content.trailingAnchor.constraint(equalTo: effect.trailingAnchor),
            content.topAnchor.constraint(equalTo: effect.topAnchor),
            content.bottomAnchor.constraint(equalTo: effect.bottomAnchor),
        ])
        return effect
    }

    /// A stretchable rounded rectangle, for the effect view's mask.
    private static func roundedMask(_ radius: CGFloat) -> NSImage {
        let side = radius * 2 + 1
        let image = NSImage(size: NSSize(width: side, height: side), flipped: false) { rect in
            NSColor.black.setFill()
            NSBezierPath(roundedRect: rect, xRadius: radius, yRadius: radius).fill()
            return true
        }
        image.capInsets = NSEdgeInsets(top: radius, left: radius, bottom: radius, right: radius)
        image.resizingMode = .stretch
        return image
    }
}

/// The panel's content: the field, then the results.
struct SearchOverlay: View {
    @ObservedObject var search: SearchModel
    let pick: (SearchRow) -> Void
    let close: () -> Void

    static let fieldHeight: CGFloat = 40
    static let rowHeight: CGFloat = 40
    static let visibleRows = 8
    static let listPadding: CGFloat = 8
    static let messageHeight: CGFloat = 52

    /// The panel's height for what `search` shows: the field alone with
    /// nothing typed, else up to `visibleRows` rows (scrolling beyond),
    /// or the no-results line once the engine has said so.
    static func height(_ search: SearchModel) -> CGFloat {
        if search.trimmed.isEmpty { return fieldHeight }
        if search.rows.isEmpty {
            return search.answered ? fieldHeight + 1 + messageHeight : fieldHeight
        }
        let rows = CGFloat(min(search.rows.count, visibleRows))
        return fieldHeight + 1 + rows * rowHeight + listPadding * 2
    }

    var body: some View {
        VStack(spacing: 0) {
            HStack(spacing: 8) {
                Image(systemName: "magnifyingglass")
                    .foregroundStyle(.secondary)
                SearchField(
                    text: $search.query,
                    placeholder: String(localized: "Search"),
                    onMove: { search.move($0) },
                    onSubmit: { if let row = search.picked { pick(row) } },
                    onCancel: close)
            }
            .padding(.horizontal, 14)
            .frame(height: Self.fieldHeight)
            if !search.trimmed.isEmpty {
                if !search.rows.isEmpty {
                    Divider()
                    results
                } else if search.answered {
                    Divider()
                    Text("No Results")
                        .foregroundStyle(.secondary)
                        .frame(maxWidth: .infinity, minHeight: Self.messageHeight)
                }
            }
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .top)
    }

    /// The rows, identified by position alone: that is also what the
    /// highlight and `scrollTo` go by. A second identity (the hit's id
    /// on the `ForEach`, the index as `.id` for scrolling) left a lazy
    /// stack holding stale rows that did not redraw as the highlight
    /// reached them, once the engine's hits shifted the rows under the
    /// pages shown first. A plain stack: at most `limit` hits plus the
    /// pages, nothing to gain from laziness.
    private var results: some View {
        ScrollViewReader { proxy in
            ScrollView {
                VStack(spacing: 0) {
                    ForEach(search.rows.indices, id: \.self) { index in
                        let row = search.rows[index]
                        SearchRowView(row: row, highlighted: index == search.highlighted)
                            .frame(height: Self.rowHeight)
                            .contentShape(Rectangle())
                            .onTapGesture { pick(row) }
                    }
                }
                .padding(.vertical, Self.listPadding)
                .padding(.horizontal, 8)
            }
            .onChange(of: search.highlighted) { _, index in
                proxy.scrollTo(index)
            }
        }
    }
}

/// One result: a page or list under its glyph, or an item under a
/// checkbox showing its state (display only). Under the title, where it
/// lives: the item's list, the workspace when it is not Personal, the
/// Bin, and "Archived" for an archived list or an item in one. A binned
/// item is struck through. The highlighted row is the accent, as in
/// Spotlight.
private struct SearchRowView: View {
    let row: SearchRow
    let highlighted: Bool

    var body: some View {
        HStack(spacing: 10) {
            icon
                .frame(width: 20, height: 20)
            VStack(alignment: .leading, spacing: 1) {
                title
                    .lineLimit(1)
                if !note.isEmpty {
                    Text(verbatim: note)
                        .font(.caption)
                        .foregroundStyle(highlighted ? AnyShapeStyle(.white.opacity(0.8)) : AnyShapeStyle(.secondary))
                        .lineLimit(1)
                }
            }
            Spacer(minLength: 0)
        }
        .padding(.horizontal, 10)
        .frame(maxHeight: .infinity)
        .foregroundStyle(highlighted ? AnyShapeStyle(.white) : AnyShapeStyle(.primary))
        .background {
            if highlighted {
                RoundedRectangle(cornerRadius: 10, style: .continuous).fill(Color.accentColor)
            }
        }
    }

    @ViewBuilder private var icon: some View {
        switch row {
        case .lens(.starred): NavIcon(image: StarArt.outline)
        case .lens(.agenda): NavIcon(image: NavArt.calendar)
        case .lens(.assigned): NavIcon(image: NavArt.person)
        case .lens(.done): NavIcon(image: NavArt.done)
        case .lens(.bin): NavIcon(image: NavArt.bin)
        case .inbox: NavIcon(image: NavArt.inbox)
        case .hit(let hit):
            switch hit.kind {
            case .list:
                if let icon = hit.icon, !icon.isEmpty {
                    Text(verbatim: icon)
                } else {
                    NavIcon(image: NavArt.list)
                }
            case .item:
                ItemCheckbox(state: hit.state?.itemState ?? .backlog)
            }
        }
    }

    @ViewBuilder private var title: some View {
        switch row {
        case .lens(.starred): Text("Starred")
        case .lens(.agenda): Text("Calendar")
        case .lens(.assigned): Text("Assigned")
        case .lens(.done): Text("Done")
        case .lens(.bin): Text("Bin")
        case .inbox: Text("Inbox")
        case .hit(let hit):
            Text(verbatim: hit.title)
                .strikethrough(hit.state == .binned)
        }
    }

    private var note: String {
        guard case .hit(let hit) = row else { return "" }
        var parts = [hit.listName, hit.docName].compactMap { $0 }
        if hit.state == .binned { parts.append(String(localized: "Bin")) }
        if hit.archived { parts.append(String(localized: "Archived")) }
        return parts.joined(separator: " · ")
    }
}

extension HitState {
    /// The workflow state a hit's checkbox shows; a binned item's is
    /// masked by the bin, so it reads as not done.
    var itemState: ItemState {
        switch self {
        case .backlog, .binned: return .backlog
        case .todo: return .todo
        case .inProgress: return .inProgress
        case .review: return .review
        case .done: return .done
        case .cancelled: return .cancelled
        }
    }
}

/// The field: an `NSTextField`, so ↑, ↓, Return and Escape reach
/// the panel rather than the field editor. Focused as it lands in the
/// panel.
private struct SearchField: NSViewRepresentable {
    @Binding var text: String
    let placeholder: String
    let onMove: (Int) -> Void
    let onSubmit: () -> Void
    let onCancel: () -> Void

    func makeNSView(context: Context) -> NSTextField {
        let field = FocusingField()
        field.isBordered = false
        field.drawsBackground = false
        field.focusRingType = .none
        field.font = .systemFont(ofSize: NSFont.systemFontSize)
        field.placeholderString = placeholder
        field.cell?.usesSingleLineMode = true
        field.cell?.lineBreakMode = .byTruncatingTail
        field.delegate = context.coordinator
        return field
    }

    func updateNSView(_ field: NSTextField, context: Context) {
        context.coordinator.parent = self
        if field.stringValue != text { field.stringValue = text }
    }

    func makeCoordinator() -> Coordinator { Coordinator(self) }

    final class Coordinator: NSObject, NSTextFieldDelegate {
        var parent: SearchField

        init(_ parent: SearchField) { self.parent = parent }

        func controlTextDidChange(_ notification: Notification) {
            guard let field = notification.object as? NSTextField else { return }
            parent.text = field.stringValue
        }

        func control(_ control: NSControl, textView: NSTextView, doCommandBy selector: Selector) -> Bool {
            switch selector {
            case #selector(NSResponder.moveUp(_:)): parent.onMove(-1)
            case #selector(NSResponder.moveDown(_:)): parent.onMove(1)
            case #selector(NSResponder.insertNewline(_:)): parent.onSubmit()
            case #selector(NSResponder.cancelOperation(_:)): parent.onCancel()
            default: return false
            }
            return true
        }
    }

    private final class FocusingField: NSTextField {
        override func viewDidMoveToWindow() {
            super.viewDidMoveToWindow()
            guard let window else { return }
            DispatchQueue.main.async { [weak self] in
                guard let self else { return }
                window.makeFirstResponder(self)
            }
        }
    }
}
