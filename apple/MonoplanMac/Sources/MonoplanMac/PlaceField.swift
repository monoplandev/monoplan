// The item panel's place row (`place`, spec/place-plan.md), and its
// lookup: the list row's way of working (`ListField.swift`), in the
// same box (`panelBox`), filled only under the pointer and as a
// search box. At rest it is
// the place's label on one line (cut with an ellipsis when it
// is longer; the panel puts the address under the map), or a muted
// "Location" when there is none. Under the pointer a set place shows a ✕
// at the box's end, which takes it off the item. A click turns the row
// into a search box with a list under it: one run of rows, no
// headings, in the order of the iOS place editor
// (apple/MonoplanIOS/Monoplan/ItemPlaceEditor.swift): the typed text
// as a bare label, the account's known places that match (from the
// engine, nothing leaves the device), then Apple's search results from
// three characters on (MapKit; the query goes to Apple, as the web's
// goes to Photon). The list floats under the box in a window of its
// own (`FloatingList`), not in the panel's column and not an overlay
// on it: the place row is the panel's last, so an overlay ran off the
// window's bottom edge where no scrolling reached it, and a window can
// pass that edge (and sits over the map). It is as tall as its rows up
// to a cap, scrolling past it.
// A click on a row writes it at once. Return takes the
// highlighted row, else the typed text; the arrow keys move the
// highlight; Escape, or the caret leaving the box, shuts it with the
// place as it was.

import MapKit
import MonoplanCore
import SwiftUI

struct PlaceField: View {
    let place: Place?
    /// Known places matching the text (all for none).
    let knownPlaces: (String, @escaping ([PlaceSuggestion]) -> Void) -> Void
    /// Bottom padding for the row at rest (negative when it ends the
    /// panel's block, see `ItemPanel.fields`); the search box and its
    /// list keep their full height.
    let trim: CGFloat
    /// Write the place, or clear it with nil. `exact` for a known
    /// place, which is already whole.
    let set: (Place?, _ exact: Bool) -> Void

    @State private var query = ""
    /// The search box is open.
    @State private var picking = false
    /// The search box has had the caret since it opened: only then is
    /// the caret leaving it a reason to shut.
    @State private var armed = false
    @State private var known: [PlaceSuggestion] = []
    @State private var highlight: Int?
    /// The pointer is over the box at rest: the ✕ shows.
    @State private var hovered = false
    @StateObject private var search = MacPlaceSearch()
    @FocusState private var focused: Bool

    private var typed: String { query.trimmingCharacters(in: .whitespacesAndNewlines) }

    var body: some View {
        Group {
            if picking {
                // The list hangs under the box in its own window, so
                // it takes no room in the panel.
                searchBox.background {
                    if !candidates.isEmpty {
                        FloatingList { dropdown }
                    }
                }
            } else {
                shown
            }
        }
        .onChange(of: query) {
            highlight = nil
            loadKnown()
            search.update(typed)
        }
        .onChange(of: focused) {
            if focused {
                armed = true
                loadKnown()
            } else if armed {
                finish()
            }
        }
    }

    // MARK: Rows

    /// The place at rest: its label on one line, or a muted "Location".
    private var shown: some View {
        Button {
            picking = true
        } label: {
            HStack(alignment: .firstTextBaseline, spacing: 5) {
                PanelGlyph(image: NavArt.pin)
                    .foregroundStyle(place == nil ? .tertiary : .secondary)
                    .frame(width: 20)
                Text(verbatim: place?.label ?? String(localized: "Location"))
                    .lineLimit(1)
                    .truncationMode(.tail)
                    .foregroundStyle(place == nil ? .tertiary : .primary)
                    // Room for the ✕.
                    .padding(.trailing, place == nil ? 0 : 20)
            }
            // The whole box takes the click, not just its text.
            .panelBox(filled: false)
        }
        .buttonStyle(.plain)
        .help(place?.label ?? "")
        .accessibilityHint(Text("Searches for another place"))
        .overlay(alignment: .trailing) {
            if place != nil, hovered {
                Button {
                    set(nil, false)
                } label: {
                    Image(systemName: "xmark")
                        .font(.caption.weight(.semibold))
                        .foregroundStyle(.secondary)
                        .frame(width: 20, height: 20)
                        .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
                .hoverCircle()
                .help("Remove place")
                .accessibilityLabel("Remove place")
            }
        }
        .onHover { hovered = $0 }
        .padding(.bottom, trim)
    }

    private var searchBox: some View {
        HStack(alignment: .firstTextBaseline, spacing: 5) {
            Image(systemName: "magnifyingglass")
                .foregroundStyle(.secondary)
                .frame(width: 20)
            TextField("Search places", text: $query)
                .textFieldStyle(.plain)
                .autocorrectionDisabled()
                .focused($focused)
                .onSubmit(submit)
                .onExitCommand(perform: finish)
                .onKeyPress(.downArrow) { step(1) }
                .onKeyPress(.upArrow) { step(-1) }
        }
        .panelBox()
        // The caret, a turn after the box appears: asked for while the
        // field is still being installed, the window hands the caret to
        // its first field instead (the item's title).
        .onAppear { DispatchQueue.main.async { focused = true } }
    }

    /// The candidates, in a box under the search box and as wide: as
    /// tall as its rows up to a cap, scrolling past it.
    private var dropdown: some View {
        let rows = candidates
        return ScrollViewReader { proxy in
            ScrollView {
                VStack(alignment: .leading, spacing: 0) {
                    ForEach(Array(rows.enumerated()), id: \.offset) { index, candidate in
                        CandidateRow(candidate: candidate, highlighted: highlight == index) {
                            pick(candidate)
                        }
                        .onHover { if $0 { highlight = index } }
                        .id(index)
                    }
                }
                .padding(4)
                .frame(maxWidth: .infinity, alignment: .leading)
            }
            .frame(maxHeight: 220)
            .fixedSize(horizontal: false, vertical: true)
            // The arrow keys can walk the highlight out of sight.
            .onChange(of: highlight) {
                guard let highlight, rows.indices.contains(highlight) else { return }
                proxy.scrollTo(highlight)
            }
        }
        .background(Color(nsColor: .controlBackgroundColor), in: RoundedRectangle(cornerRadius: 8, style: .continuous))
        .overlay(RoundedRectangle(cornerRadius: 8, style: .continuous).strokeBorder(.separator))
    }

    // MARK: Candidates

    private var candidates: [Candidate] {
        var rows: [Candidate] = []
        if !typed.isEmpty { rows.append(.typed(typed)) }
        rows += known.map(Candidate.known)
        rows += search.results.map(Candidate.result)
        return rows
    }

    private func step(_ by: Int) -> KeyPress.Result {
        let count = candidates.count
        guard count > 0 else { return .ignored }
        highlight = max(0, min(count - 1, (highlight.map { $0 + by }) ?? (by > 0 ? 0 : count - 1)))
        return .handled
    }

    private func submit() {
        let rows = candidates
        if let highlight, rows.indices.contains(highlight) {
            pick(rows[highlight])
        } else if !typed.isEmpty {
            pick(.typed(typed))
        } else {
            finish()
        }
    }

    private func pick(_ candidate: Candidate) {
        switch candidate {
        case .typed(let label):
            // The engine takes a known place of exactly that label
            // whole (what `item set --place <label>` does).
            set(Place(label: label), false)
        case .known(let row):
            set(row.place, true)
        case .result(let result):
            search.resolve(result) { set($0, false) }
        }
        finish()
    }

    /// Back to the field as it was: no text, no list.
    private func finish() {
        query = ""
        highlight = nil
        picking = false
        armed = false
        focused = false
    }

    private func loadKnown() {
        let asked = typed
        knownPlaces(asked) { rows in
            // A reply for text since changed is stale.
            if asked == typed { known = rows }
        }
    }
}

private enum Candidate {
    case typed(String)
    case known(PlaceSuggestion)
    case result(MKLocalSearchCompletion)
}

/// One row of the list: an icon, the name, the address under it.
private struct CandidateRow: View {
    let candidate: Candidate
    let highlighted: Bool
    let action: () -> Void

    var body: some View {
        Button(action: action) {
            HStack(alignment: .firstTextBaseline, spacing: 8) {
                icon
                    .foregroundStyle(highlighted ? .white : .secondary)
                    .frame(width: 16)
                VStack(alignment: .leading, spacing: 1) {
                    title
                    if let subtitle, !subtitle.isEmpty {
                        Text(verbatim: subtitle)
                            .font(.footnote)
                            .foregroundStyle(highlighted ? .white.opacity(0.8) : .secondary)
                            .lineLimit(1)
                    }
                }
                .foregroundStyle(highlighted ? .white : .primary)
                Spacer(minLength: 0)
            }
            .padding(.horizontal, 8)
            .padding(.vertical, 4)
            .frame(maxWidth: .infinity, alignment: .leading)
            .background(
                highlighted ? Color.accentColor : .clear,
                in: RoundedRectangle(cornerRadius: 5, style: .continuous))
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
    }

    @ViewBuilder
    private var icon: some View {
        switch candidate {
        case .typed: Image(systemName: "character.cursor.ibeam")
        case .known: Image(systemName: "clock")
        case .result: PanelGlyph(image: NavArt.pin, size: 13)
        }
    }

    @ViewBuilder
    private var title: some View {
        switch candidate {
        case .typed(let label): Text("Use “\(label)”").lineLimit(1)
        case .known(let row): Text(verbatim: row.place.label).lineLimit(1)
        case .result(let result): Text(verbatim: result.title).lineLimit(1)
        }
    }

    private var subtitle: String? {
        switch candidate {
        case .typed: nil
        case .known(let row): row.place.address
        case .result(let result): result.subtitle
        }
    }
}

/// A list floating under the view it is the background of, as wide as
/// it: a borderless child window of the view's own, so it can pass the
/// window's edges and sits over AppKit views (the map). The window
/// never becomes key, so the caret stays in the search box while a row
/// is clicked. It follows the view as the panel scrolls, hides while
/// the view is scrolled out of sight, and goes above the view when the
/// screen has no room under it.
private struct FloatingList<Content: View>: NSViewRepresentable {
    @ViewBuilder let content: Content

    func makeNSView(context: Context) -> FloatingListAnchor { FloatingListAnchor() }

    func updateNSView(_ view: FloatingListAnchor, context: Context) {
        view.show(AnyView(content))
    }

    static func dismantleNSView(_ view: FloatingListAnchor, coordinator: ()) {
        view.close()
    }
}

private final class FloatingListAnchor: NSView {
    /// The gap between the view and the list.
    private static let gap: CGFloat = 6

    private var content = AnyView(EmptyView())
    private var panel: NSPanel?
    private var scrolled: NSObjectProtocol?

    func show(_ content: AnyView) {
        self.content = content
        place()
    }

    func close() {
        if let scrolled { NotificationCenter.default.removeObserver(scrolled) }
        scrolled = nil
        hide()
        panel = nil
    }

    private func hide() {
        guard let panel else { return }
        panel.parent?.removeChildWindow(panel)
        panel.orderOut(nil)
    }

    override func viewDidMoveToWindow() {
        super.viewDidMoveToWindow()
        guard window != nil else { return close() }
        // The panel's scroll view moves this view without laying it
        // out: follow it.
        if scrolled == nil, let clip = enclosingScrollView?.contentView {
            clip.postsBoundsChangedNotifications = true
            scrolled = NotificationCenter.default.addObserver(
                forName: NSView.boundsDidChangeNotification, object: clip, queue: .main
            ) { [weak self] _ in self?.place() }
        }
        // Once the view has its frame.
        DispatchQueue.main.async { [weak self] in self?.place() }
    }

    override func layout() {
        super.layout()
        place()
    }

    private func place() {
        guard let window, bounds.width > 0 else { return }
        guard !visibleRect.isEmpty else { return hide() }
        let panel = self.panel ?? makePanel()
        self.panel = panel
        guard let host = panel.contentView as? FloatingListHost else { return }
        host.rootView = AnyView(content.frame(width: bounds.width))
        let height = host.fittingSize.height
        guard height > 0 else { return hide() }
        let anchor = window.convertToScreen(convert(bounds, to: nil))
        var y = anchor.minY - Self.gap - height
        // No room under the view on this screen: above it, when there
        // is room there.
        if let screen = window.screen?.visibleFrame, y < screen.minY,
           anchor.maxY + Self.gap + height <= screen.maxY {
            y = anchor.maxY + Self.gap
        }
        panel.setFrame(NSRect(x: anchor.minX, y: y, width: bounds.width, height: height), display: true)
        if panel.parent == nil { window.addChildWindow(panel, ordered: .above) }
        panel.invalidateShadow()
    }

    private func makePanel() -> NSPanel {
        let panel = NSPanel(
            contentRect: .zero, styleMask: [.borderless, .nonactivatingPanel], backing: .buffered, defer: true)
        panel.isOpaque = false
        panel.backgroundColor = .clear
        panel.hasShadow = true
        panel.contentView = FloatingListHost(rootView: AnyView(EmptyView()))
        return panel
    }
}

/// The floating list's content. Its window is never key, so a click on
/// a row has to count as it lands.
private final class FloatingListHost: NSHostingView<AnyView> {
    override func acceptsFirstMouse(for event: NSEvent?) -> Bool { true }
}

/// Apple's place search as-you-type (`MKLocalSearchCompleter`), and the
/// follow-up search that turns a picked suggestion into coordinates,
/// as on iOS (`PlaceSearch`). No location permission: results are not
/// biased to where the Mac is.
final class MacPlaceSearch: NSObject, ObservableObject, MKLocalSearchCompleterDelegate {
    @Published private(set) var results: [MKLocalSearchCompletion] = []
    private let completer = MKLocalSearchCompleter()

    override init() {
        super.init()
        completer.delegate = self
        completer.resultTypes = [.pointOfInterest, .address]
    }

    /// Nothing is asked of Apple under three characters, as on the web.
    func update(_ query: String) {
        guard query.count >= 3 else {
            completer.cancel()
            results = []
            return
        }
        completer.queryFragment = query
    }

    func completerDidUpdateResults(_ completer: MKLocalSearchCompleter) {
        results = Array(completer.results.prefix(6))
    }

    func completer(_ completer: MKLocalSearchCompleter, didFailWithError error: Error) {
        results = []
    }

    /// A suggestion as a place: its name, where it is, and its address.
    /// If the follow-up search fails the suggestion's own text is kept,
    /// without coordinates.
    func resolve(_ result: MKLocalSearchCompletion, completion: @escaping (Place) -> Void) {
        let address = result.subtitle.isEmpty ? nil : result.subtitle
        MKLocalSearch(request: MKLocalSearch.Request(completion: result)).start { response, _ in
            let place: Place
            if let item = response?.mapItems.first {
                let at = item.placemark.coordinate
                place = Place(
                    label: item.name ?? result.title, lat: at.latitude, lon: at.longitude,
                    address: address ?? item.placemark.title)
            } else {
                place = Place(label: result.title, address: address)
            }
            DispatchQueue.main.async { completion(place) }
        }
    }
}
