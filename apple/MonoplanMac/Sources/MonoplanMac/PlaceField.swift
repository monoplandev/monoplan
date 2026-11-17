// The item panel's place row (`place`, spec/place-plan.md), and its
// lookup: the list row's way of working (`ListField.swift`), in the
// same box (`panelBox`), filled only under the pointer and as a
// search box. At rest it is
// the place's label on one line (cut with an ellipsis when it
// is longer; the panel puts the address under the map), or a muted
// "Place" when there is none. A click turns the row into a search
// box and drops a list under it, floating over the panel's rows, as
// the iOS place editor
// (apple/MonoplanIOS/Monoplan/ItemPlaceEditor.swift) lists them: the
// typed text as a bare label, the account's known places that match
// (from the engine, nothing leaves the device), then Apple's search
// results from three characters on (MapKit; the query goes to Apple,
// as the web's goes to Photon). Over a set place, until something is
// typed, the list's first row is "Remove place", which takes it off
// the item. A click on a row writes it at once. Return takes the
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
    @StateObject private var search = MacPlaceSearch()
    @FocusState private var focused: Bool

    private var typed: String { query.trimmingCharacters(in: .whitespacesAndNewlines) }

    var body: some View {
        Group {
            if picking {
                // The list floats over the rows under the box: an
                // overlay takes no room. It hangs from a zero-height
                // strip on the box's bottom edge, so it grows
                // downwards at its own height. The panel puts the field
                // above its later rows (`zIndex`) so the list draws,
                // and takes clicks, over them.
                searchBox.overlay(alignment: .bottomLeading) {
                    Color.clear
                        .frame(height: 0)
                        .overlay(alignment: .topLeading) {
                            if !candidates.isEmpty {
                                dropdown
                                    .fixedSize(horizontal: false, vertical: true)
                                    .padding(.top, 6)
                            }
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

    /// The place at rest: its label on one line, or a muted "Place".
    private var shown: some View {
        Button {
            picking = true
        } label: {
            HStack(alignment: .firstTextBaseline, spacing: 10) {
                PanelGlyph(image: NavArt.pin)
                    .foregroundStyle(place == nil ? .tertiary : .secondary)
                    .frame(width: 20)
                Text(verbatim: place?.label ?? String(localized: "Place"))
                    .lineLimit(1)
                    .truncationMode(.tail)
                    .foregroundStyle(place == nil ? .tertiary : .primary)
            }
            // The whole box takes the click, not just its text.
            .panelBox(filled: false)
        }
        .buttonStyle(.plain)
        .help(place?.label ?? "")
        .accessibilityHint(Text("Searches for another place"))
    }

    private var searchBox: some View {
        HStack(alignment: .firstTextBaseline, spacing: 10) {
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

    /// The candidates, in a box under the search box and as wide.
    private var dropdown: some View {
        VStack(alignment: .leading, spacing: 0) {
            ForEach(Array(candidates.enumerated()), id: \.offset) { index, candidate in
                if let header = header(at: index) {
                    Text(header)
                        .font(.caption)
                        .foregroundStyle(.secondary)
                        .padding(.horizontal, 8)
                        .padding(.top, index == 0 ? 2 : 8)
                        .padding(.bottom, 2)
                }
                CandidateRow(candidate: candidate, highlighted: highlight == index) {
                    pick(candidate)
                }
                .onHover { if $0 { highlight = index } }
            }
        }
        .padding(4)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(Color(nsColor: .controlBackgroundColor), in: RoundedRectangle(cornerRadius: 8, style: .continuous))
        .overlay(RoundedRectangle(cornerRadius: 8, style: .continuous).strokeBorder(.separator))
        .shadow(color: .black.opacity(0.18), radius: 10, y: 4)
    }

    // MARK: Candidates

    private var candidates: [Candidate] {
        var rows: [Candidate] = []
        // Over a set place the list opens on the way to take it off.
        if place != nil, typed.isEmpty { rows.append(.remove) }
        if !typed.isEmpty { rows.append(.typed(typed)) }
        rows += known.map(Candidate.known)
        rows += search.results.map(Candidate.result)
        return rows
    }

    /// A section's title over its first row.
    private func header(at index: Int) -> String? {
        let rows = candidates
        let kind = rows[index].kind
        guard index == 0 || rows[index - 1].kind != kind else { return nil }
        switch kind {
        case .remove, .typed: return nil
        case .known: return typed.isEmpty ? String(localized: "Recent") : String(localized: "Your places")
        case .result: return String(localized: "Search")
        }
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
        case .remove:
            set(nil, false)
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
    /// Take the set place off the item.
    case remove
    case typed(String)
    case known(PlaceSuggestion)
    case result(MKLocalSearchCompletion)

    enum Kind { case remove, typed, known, result }

    var kind: Kind {
        switch self {
        case .remove: .remove
        case .typed: .typed
        case .known: .known
        case .result: .result
        }
    }
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
        case .remove: Image(systemName: "xmark")
        case .typed: Image(systemName: "character.cursor.ibeam")
        case .known: Image(systemName: "clock")
        case .result: PanelGlyph(image: NavArt.pin, size: 13)
        }
    }

    @ViewBuilder
    private var title: some View {
        switch candidate {
        case .remove: Text("Remove place").lineLimit(1)
        case .typed(let label): Text("Use “\(label)”").lineLimit(1)
        case .known(let row): Text(verbatim: row.place.label).lineLimit(1)
        case .result(let result): Text(verbatim: result.title).lineLimit(1)
        }
    }

    private var subtitle: String? {
        switch candidate {
        case .remove, .typed: nil
        case .known(let row): row.place.address
        case .result(let result): result.subtitle
        }
    }
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
