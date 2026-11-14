// The item sheet's place editor (`place`, spec/place-plan.md): like the
// date editor it takes the sheet over. One text box; under it, in
// order: the typed text as a bare label, the account's known places
// that match (from the engine, nothing leaves the device), then Apple's
// search results from three characters on (MapKit; the query goes to
// Apple, as the web's goes to Photon). A pick is written at once and
// returns to the item. The cross leaves the place as it was; the tick
// takes the typed text, if any, as the label.

import MapKit
import MonoplanCore
import SwiftUI

struct ItemPlaceEditor: View {
    @EnvironmentObject private var model: AppModel
    let ref: ItemRef
    let item: ItemInfo
    let done: () -> Void

    @State private var query = ""
    @State private var known: [PlaceSuggestion] = []
    @StateObject private var search = PlaceSearch()
    @FocusState private var focused: Bool

    private var typed: String { query.trimmingCharacters(in: .whitespacesAndNewlines) }

    var body: some View {
        VStack(spacing: 0) {
            searchBox
            List {
                if typed.isEmpty, let place = item.place {
                    Section("Current") {
                        PlaceRow(title: place.label, subtitle: place.address)
                        Button("Clear place", role: .destructive) {
                            model.setPlace(ref, place: nil)
                            done()
                        }
                    }
                }
                if !typed.isEmpty {
                    Section {
                        Button(action: useTyped) {
                            PlaceRow(title: "Use \u{201C}\(typed)\u{201D}", subtitle: nil, systemImage: "character.cursor.ibeam")
                        }
                    }
                }
                if !known.isEmpty {
                    Section(typed.isEmpty ? "Recent" : "Your places") {
                        ForEach(known, id: \.self) { row in
                            Button {
                                // As it is: a known place is already whole.
                                model.setPlace(ref, place: row.place, exact: true)
                                done()
                            } label: {
                                PlaceRow(title: row.place.label, subtitle: row.place.address, systemImage: "clock")
                            }
                        }
                    }
                }
                if !search.results.isEmpty {
                    Section("Search") {
                        ForEach(search.results, id: \.self) { result in
                            Button {
                                search.resolve(result) { place in
                                    model.setPlace(ref, place: place)
                                    done()
                                }
                            } label: {
                                PlaceRow(title: result.title, subtitle: result.subtitle)
                            }
                        }
                    }
                }
            }
            .listStyle(.plain)
            .buttonStyle(.plain)
            .scrollDismissesKeyboard(.interactively)
        }
        .editorHeader(title: "Place", itemTitle: item.text, cancel: done, done: useTyped)
        .onAppear {
            focused = true
            loadKnown()
        }
        .onChange(of: query) {
            loadKnown()
            search.update(typed)
        }
    }

    private var searchBox: some View {
        HStack(spacing: 8) {
            Image(systemName: "magnifyingglass").foregroundStyle(.secondary)
            TextField("Search or type a place", text: $query)
                .focused($focused)
                .submitLabel(.done)
                .autocorrectionDisabled()
                .onSubmit(useTyped)
            if !query.isEmpty {
                Button {
                    query = ""
                } label: {
                    Image(systemName: "xmark.circle.fill").foregroundStyle(.tertiary)
                }
                .buttonStyle(.plain)
                .accessibilityLabel("Clear text")
            }
        }
        .padding(.horizontal, 12)
        .frame(minHeight: 40)
        .background(.quaternary.opacity(0.5), in: RoundedRectangle(cornerRadius: 10, style: .continuous))
        .padding()
    }

    /// The typed text as the place's label. The engine takes a known
    /// place of exactly that label whole, coordinates included (what
    /// `item set --place <label>` does); otherwise it is a bare label.
    /// Nothing typed: just close.
    private func useTyped() {
        if !typed.isEmpty { model.setPlace(ref, place: Place(label: typed)) }
        done()
    }

    private func loadKnown() {
        let asked = typed
        model.knownPlaces(matching: asked) { rows in
            // A reply for text since changed is stale.
            if asked == typed { known = rows }
        }
    }
}

/// One candidate place: an icon, its name, and its address underneath.
private struct PlaceRow: View {
    let title: String
    let subtitle: String?
    /// The row's symbol; nil is a place, which has the web's pin.
    var systemImage: String? = nil

    var body: some View {
        HStack(alignment: .firstTextBaseline, spacing: 12) {
            Group {
                if let systemImage { Image(systemName: systemImage) } else { PinIcon() }
            }
            .foregroundStyle(.secondary)
            .frame(width: 24)
            VStack(alignment: .leading, spacing: 2) {
                Text(title)
                if let subtitle, !subtitle.isEmpty {
                    Text(subtitle).font(.footnote).foregroundStyle(.secondary)
                }
            }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .contentShape(Rectangle())
    }
}

/// Apple's place search as-you-type (`MKLocalSearchCompleter`), and the
/// follow-up search that turns a picked suggestion into coordinates.
/// No location permission: results are not biased to where the device
/// is.
final class PlaceSearch: NSObject, ObservableObject, MKLocalSearchCompleterDelegate {
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
        results = completer.results
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
