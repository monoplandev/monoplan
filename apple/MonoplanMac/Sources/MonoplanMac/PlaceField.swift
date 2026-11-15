// The item panel's place row (`place`, spec/place-plan.md), edited in
// place. Unset, it is a text field that reads as the muted "Place" row
// until clicked. Typing drops a list under it, as the iOS place editor
// (apple/MonoplanIOS/Monoplan/ItemPlaceEditor.swift) lists them: the
// typed text as a bare label, the account's known places that match
// (from the engine, nothing leaves the device), then Apple's search
// results from three characters on (MapKit; the query goes to Apple,
// as the web's goes to Photon). A click on a row writes it at once.
// Return takes the highlighted row, else the typed text; the arrow
// keys move the highlight; Escape leaves the place as it was. The
// list floats over the panel's rows rather than pushing them down.
//
// Set, the row shows the place; a click on it searches for another,
// and the cross beside it clears it.

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
    /// Searching for another place over a set one.
    @State private var replacing = false
    @State private var known: [PlaceSuggestion] = []
    @State private var highlight: Int?
    @State private var hovering = false
    @StateObject private var search = MacPlaceSearch()
    @FocusState private var focused: Bool

    private var typed: String { query.trimmingCharacters(in: .whitespacesAndNewlines) }

    var body: some View {
        Group {
            if let place, !replacing {
                shown(place)
            } else {
                // The list floats over the rows under the field: an
                // overlay takes no room. It hangs from a zero-height
                // strip on the field's bottom edge, so it grows
                // downwards at its own height. The panel puts the field
                // above its later rows (`zIndex`) so the list draws,
                // and takes clicks, over them.
                field.overlay(alignment: .bottomLeading) {
                    Color.clear
                        .frame(height: 0)
                        .overlay(alignment: .topLeading) {
                            if focused || !typed.isEmpty, !candidates.isEmpty {
                                dropdown
                                    .fixedSize(horizontal: false, vertical: true)
                                    .padding(.top, 6)
                            }
                        }
                }
            }
        }
        .onChange(of: query) {
            highlight = nil
            loadKnown()
            search.update(typed)
        }
        .onChange(of: focused) {
            if focused {
                loadKnown()
            } else if typed.isEmpty {
                replacing = false
            }
        }
    }

    // MARK: Rows

    /// The set place: its label and address, the cross on hover.
    private func shown(_ place: Place) -> some View {
        HStack(alignment: .firstTextBaseline, spacing: 10) {
            PanelGlyph(image: NavArt.pin)
                .foregroundStyle(.secondary)
                .frame(width: 20)
            VStack(alignment: .leading, spacing: 2) {
                Text(verbatim: place.label)
                if let address = place.address {
                    Text(verbatim: address).font(.footnote).foregroundStyle(.secondary)
                }
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            .contentShape(Rectangle())
            .onTapGesture {
                replacing = true
                focused = true
            }
            .accessibilityAddTraits(.isButton)
            .accessibilityHint(Text("Searches for another place"))
            Button {
                set(nil, false)
            } label: {
                Image(systemName: "xmark.circle.fill").foregroundStyle(.tertiary)
            }
            .buttonStyle(.plain)
            .opacity(hovering ? 1 : 0)
            .help(String(localized: "Clear place"))
            .accessibilityLabel(Text("Clear place"))
        }
        .onHover { hovering = $0 }
    }

    private var field: some View {
        HStack(alignment: .firstTextBaseline, spacing: 10) {
            PanelGlyph(image: NavArt.pin)
                .foregroundStyle(focused ? .secondary : .tertiary)
                .frame(width: 20)
            TextField(place?.label ?? String(localized: "Place"), text: $query)
                .textFieldStyle(.plain)
                .autocorrectionDisabled()
                .focused($focused)
                .onSubmit(submit)
                .onExitCommand(perform: finish)
                .onKeyPress(.downArrow) { step(1) }
                .onKeyPress(.upArrow) { step(-1) }
        }
        // A new one opens with the caret in it.
        .onAppear { if replacing { focused = true } }
    }

    /// The candidates, in a box under the field, lined up with its text.
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
        .padding(.leading, 30)
    }

    // MARK: Candidates

    private var candidates: [Candidate] {
        var rows: [Candidate] = []
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
        case .typed: return nil
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

    /// Back to the row as it was: no text, no list.
    private func finish() {
        query = ""
        highlight = nil
        replacing = false
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

    enum Kind { case typed, known, result }

    var kind: Kind {
        switch self {
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
