// The Map page, pushed from Home, with no title or bar surface over
// the map, only the back button and a filter button: the Open items
// that carry a place with coordinates (`places --items`, every
// workspace, events included), one marker per spot, over a sheet that
// stays up for as long as the page does. The filter (top right) is a
// menu of two ticks, Open and Closed: Open alone to start with, on
// every visit; Closed adds the Done and Cancelled items (never the
// binned) and the events that have gone by, their markers grey where
// a spot has nothing open. The sheet lists the items nearest first: from
// where the phone is when location is allowed, else from the middle of
// the map; it has a search field (item text, place label, address) and
// three heights (a strip, half, full), and the map behind it stays
// live up to half. A marker narrows the list to the items at that
// spot. A row opens the item's page inside the sheet, pushed over the
// list, and moves the map to its marker; the page's "x" comes back to
// the list. Items whose place has no coordinates cannot be pinned and
// are only counted, under the list.
//
// Location never leaves the phone: the distances are worked out here.
// Map tiles come from Apple, so the page tells Apple roughly what
// region is being looked at (as `PlaceMapCard` does for one place).
// Home's bottom bar is hidden while the page is up; the sheet has the
// search.

import CoreLocation
import MapKit
import MonoplanCore
import SwiftUI

/// Where the phone is, while the Map page is up. Asks for when-in-use
/// permission the first time; without it `location` stays nil and the
/// page does without distances.
final class LocationSource: NSObject, ObservableObject, CLLocationManagerDelegate {
    @Published private(set) var location: CLLocation?
    private let manager = CLLocationManager()
    private var wanted = false

    override init() {
        super.init()
        manager.delegate = self
        manager.desiredAccuracy = kCLLocationAccuracyHundredMeters
        manager.distanceFilter = 50
    }

    func start() {
        wanted = true
        switch manager.authorizationStatus {
        case .notDetermined: manager.requestWhenInUseAuthorization()
        case .authorizedWhenInUse, .authorizedAlways: manager.startUpdatingLocation()
        default: break
        }
    }

    func stop() {
        wanted = false
        manager.stopUpdatingLocation()
    }

    func locationManagerDidChangeAuthorization(_ manager: CLLocationManager) {
        switch manager.authorizationStatus {
        case .authorizedWhenInUse, .authorizedAlways:
            if wanted { manager.startUpdatingLocation() }
        default:
            location = nil
        }
    }

    func locationManager(_ manager: CLLocationManager, didUpdateLocations locations: [CLLocation]) {
        if let last = locations.last { location = last }
    }

    func locationManager(_ manager: CLLocationManager, didFailWithError error: Error) {}
}

extension LocatedItem {
    /// Closed as the Map page's filter has it: Done or Cancelled, or
    /// an event that has gone by (its whole span ended before today,
    /// the agenda's rule for a `when` that is over).
    var isClosedOnMap: Bool { item.state.isClosed || hasGoneBy }

    /// An event whose last day is before today.
    var hasGoneBy: Bool {
        guard item.event, let when = item.when,
            let first = WhenText.date(String(when.prefix(10)))
        else { return false }
        let length = max(0, Int(item.duration ?? 0))
        // Days past the first that the span reaches: a timed one up
        // to the day its end falls on (an end at midnight belongs to
        // the day before), an all-day one its whole days.
        let extra = when.count >= 16
            ? (length > 0 ? (ItemDateEditor.startClock(when) + length - 1) / 1440 : 0)
            : max(1, length / 1440) - 1
        let last = WhenText.calendar.date(byAdding: .day, value: extra, to: first) ?? first
        return WhenText.day(last) < WhenText.today()
    }
}

/// An item that can be pinned: its place has coordinates.
private struct Pinned: Equatable {
    let item: LocatedItem
    let place: Place
    let coordinate: CLLocationCoordinate2D
    /// The spot's identity: items at the same coordinates share a
    /// marker (five decimals is about a metre).
    let spot: String

    init?(_ item: LocatedItem) {
        guard let place = item.item.place, let lat = place.lat, let lon = place.lon else { return nil }
        self.item = item
        self.place = place
        coordinate = CLLocationCoordinate2D(latitude: lat, longitude: lon)
        spot = String(format: "%.5f,%.5f", lat, lon)
    }

    static func == (a: Pinned, b: Pinned) -> Bool { a.item == b.item }

    func metres(from origin: CLLocation) -> CLLocationDistance {
        origin.distance(from: CLLocation(latitude: coordinate.latitude, longitude: coordinate.longitude))
    }
}

/// One marker: a spot and the items at it.
private struct Spot: Identifiable {
    let id: String
    let label: String
    let coordinate: CLLocationCoordinate2D
    let count: Int
    /// Nothing open here: every item at the spot is closed.
    let allClosed: Bool
    /// Grey with nothing open, else red, a marker's own colour.
    var tint: Color { allClosed ? .gray : .red }
}

struct PlacesMapView: View {
    @EnvironmentObject private var model: AppModel
    /// Home's navigation path, which this page sits on: it empties the
    /// moment the back button is tapped, before the pop has run.
    @Binding var homePath: NavigationPath
    @StateObject private var location = LocationSource()
    /// Follows the phone when it may; else frames every marker.
    @State private var camera: MapCameraPosition = .userLocation(fallback: .automatic)
    /// The middle of the map as last left: where "nearest" is measured
    /// from without a location.
    @State private var centre: CLLocationCoordinate2D?
    /// The marker picked, if one is: the list shows that spot only.
    @State private var spot: String?
    @State private var sheetUp = false
    @State private var detent: PresentationDetent = .medium
    /// The item page pushed in the sheet, if one is.
    @State private var path: [ItemRef] = []
    @State private var query = ""
    /// The filter: which items show, by whether they are open or
    /// closed. Open alone on every visit.
    @State private var showOpen = true
    @State private var showClosed = false
    @FocusState private var searching: Bool
    /// Ties the controls drawn over the map to it.
    @Namespace private var mapScope

    private static let strip = PresentationDetent.height(120)

    /// The filter's pick of what the model read (which holds closed
    /// items only while the filter asks for them, give or take a read
    /// in flight).
    private var placed: [LocatedItem] {
        model.placed.filter { $0.isClosedOnMap ? showClosed : showOpen }
    }

    private var pinned: [Pinned] { placed.compactMap(Pinned.init) }

    /// Not the filter a visit starts with.
    private var filtered: Bool { !showOpen || showClosed }

    private var spots: [Spot] {
        var order: [String] = []
        var bySpot: [String: [Pinned]] = [:]
        for pin in pinned {
            if bySpot[pin.spot] == nil { order.append(pin.spot) }
            bySpot[pin.spot, default: []].append(pin)
        }
        return order.compactMap { id in
            guard let pins = bySpot[id], let first = pins.first else { return nil }
            return Spot(
                id: id, label: first.place.label, coordinate: first.coordinate, count: pins.count,
                allClosed: pins.allSatisfy { $0.item.isClosedOnMap })
        }
    }

    /// The list: the picked spot's items, or all of them, matching the
    /// search, nearest first.
    private var rows: [Pinned] {
        let trimmed = query.trimmingCharacters(in: .whitespaces)
        let shown = pinned.filter { pin in
            if let spot, pin.spot != spot { return false }
            if trimmed.isEmpty { return true }
            let place = pin.place
            return [pin.item.text, place.label, place.address ?? ""]
                .contains { $0.localizedStandardContains(trimmed) }
        }
        let origin = location.location
            ?? centre.map { CLLocation(latitude: $0.latitude, longitude: $0.longitude) }
        guard let origin else { return shown }
        // Stable for equal distances: the engine's order holds at one spot.
        return shown.enumerated()
            .map { (offset: $0.offset, pin: $0.element, metres: $0.element.metres(from: origin)) }
            .sorted { ($0.metres, $0.offset) < ($1.metres, $1.offset) }
            .map(\.pin)
    }

    /// Items with a place that has no coordinates.
    private var unpinned: Int { placed.count - pinned.count }

    var body: some View {
        GeometryReader { geo in
            // How much of the page the sheet covers, measured up from
            // the bottom safe area: the strip's height, or half the
            // screen (the full sheet covers the controls anyway).
            let covered = detent == Self.strip
                ? 120 : (geo.size.height + geo.safeAreaInsets.top + geo.safeAreaInsets.bottom) / 2
            Map(position: $camera, selection: $spot, scope: mapScope) {
                UserAnnotation()
                ForEach(spots) { spot in
                    marker(spot)
                }
            }
            // The map's own controls would sit under the status bar
            // (the map ignores the safe area): none there, and ours at
            // the bottom right instead.
            .mapControls {}
            .onMapCameraChange(frequency: .onEnd) { centre = $0.region.center }
            // The sheet covers the foot of the map: keep what the
            // camera frames, and the logo and legal link, above it.
            // Less a little, set by eye on a phone: at `covered` the
            // logo sits level with the top of the location button, and
            // this brings it down into the button's row.
            .safeAreaPadding(.bottom, covered - 23)
            // The map runs to the top of the screen, under the back
            // button.
            .ignoresSafeArea()
            // The location button, and the compass while the map is
            // turned, at the bottom right just above the sheet.
            .overlay(alignment: .bottomTrailing) {
                VStack(spacing: 8) {
                    MapCompass(scope: mapScope)
                    MapUserLocationButton(scope: mapScope)
                }
                .buttonBorderShape(.circle)
                .padding(.trailing, 12)
                .padding(.bottom, covered + 12)
                .animation(.default, value: detent)
            }
        }
        .mapScope(mapScope)
        // No title and no bar surface: the map says what the page is.
        // The back button stays.
        .navigationTitle("")
        .navigationBarTitleDisplayMode(.inline)
        .toolbarBackground(.hidden, for: .navigationBar)
        .toolbar {
            ToolbarItem(placement: .topBarTrailing) { filterMenu }
        }
        .sheet(isPresented: $sheetUp) { sheet }
        .onAppear {
            model.refreshPlaced(closed: showClosed)
            location.start()
            sheetUp = true
        }
        .onDisappear {
            sheetUp = false
            location.stop()
        }
        .onChange(of: model.changeCount) { model.refreshPlaced(closed: showClosed) }
        .onChange(of: showClosed) { model.refreshPlaced(closed: showClosed) }
        // A picked spot the filter has emptied is no longer on the map.
        .onChange(of: spots.map(\.id)) {
            if let spot, !spots.contains(where: { $0.id == spot }) { self.spot = nil }
        }
        // The sheet is over the whole screen and cannot slide away
        // with the page, so it goes down as the pop starts instead of
        // staying up until the page has gone (`onDisappear`).
        .onChange(of: homePath.isEmpty) { if homePath.isEmpty { sheetUp = false } }
    }

    /// A spot's marker: its count on it when it holds several items.
    @MapContentBuilder
    private func marker(_ spot: Spot) -> some MapContent {
        if spot.count > 1 {
            Marker(spot.label, monogram: Text("\(spot.count)"), coordinate: spot.coordinate)
                .tint(spot.tint)
                .tag(spot.id)
        } else {
            Marker(spot.label, coordinate: spot.coordinate)
                .tint(spot.tint)
                .tag(spot.id)
        }
    }

    /// The filter button: a menu of two ticks that stays open while
    /// they are set. Filled while it is not the filter a visit starts
    /// with.
    private var filterMenu: some View {
        Menu {
            Toggle("Open", isOn: $showOpen)
            Toggle("Closed", isOn: $showClosed)
        } label: {
            Image(systemName: filtered
                ? "line.3.horizontal.decrease.circle.fill" : "line.3.horizontal.decrease")
        }
        .menuActionDismissBehavior(.disabled)
        .accessibilityLabel("Filter")
    }

    private var sheet: some View {
        NavigationStack(path: $path) {
            list
                .toolbar(.hidden, for: .navigationBar)
                .navigationDestination(for: ItemRef.self) { ref in
                    // The page's own "x" pops back to the list.
                    ItemView(ref: ref).navigationBarBackButtonHidden()
                }
        }
        .environmentObject(model)
        .presentationDetents([Self.strip, .medium, .large], selection: $detent)
        .presentationBackgroundInteraction(.enabled(upThrough: .medium))
        // The sheet is the page's other half: it goes when the page does.
        .interactiveDismissDisabled()
        // The keyboard would cover a shorter sheet's list.
        .onChange(of: searching) { if searching { detent = .large } }
    }

    private var list: some View {
        // The field sits above the list, not over it (a top inset): the
        // field has no surface of its own, so rows scrolled under an
        // inset show through and above it.
        VStack(spacing: 0) {
            searchField
            rowList
        }
    }

    private var rowList: some View {
        List {
            if let spot, let picked = spots.first(where: { $0.id == spot }) {
                HStack {
                    Label { Text(picked.label) } icon: { PinIcon(size: 15) }
                        .lineLimit(1)
                    Spacer()
                    Button("Show All") { self.spot = nil }
                        .buttonStyle(.borderless)
                }
                .font(.subheadline)
            }
            ForEach(rows, id: \.item.key) { pin in
                Button { open(pin) } label: { PlacedRowView(pin: pin, from: location.location) }
                    .buttonStyle(.plain)
                    .itemListRow()
            }
            if rows.isEmpty {
                emptyState.listRowSeparator(.hidden)
            }
            if unpinned > 0 {
                Text("^[\(unpinned) more item](inflect: true) with a place that has no location on the map.")
                    .font(.footnote)
                    .foregroundStyle(.secondary)
                    .listRowSeparator(.hidden)
            }
        }
        .listStyle(.plain)
        .animation(.default, value: rows)
    }

    private var searchField: some View {
        HStack(spacing: 8) {
            Image(systemName: "magnifyingglass")
                .foregroundStyle(.secondary)
            TextField("Search places and items", text: $query)
                .textInputAutocapitalization(.never)
                .autocorrectionDisabled()
                .submitLabel(.search)
                .focused($searching)
            if !query.isEmpty {
                Button {
                    query = ""
                } label: {
                    Image(systemName: "xmark.circle.fill").foregroundStyle(.secondary)
                }
                .buttonStyle(.plain)
                .accessibilityLabel("Clear search")
            }
        }
        .padding(.horizontal, 12)
        .frame(height: 40)
        .background(.quaternary.opacity(0.5), in: RoundedRectangle(cornerRadius: 10, style: .continuous))
        .padding(.horizontal, 16)
        .padding(.top, 20)
        .padding(.bottom, 8)
    }

    @ViewBuilder
    private var emptyState: some View {
        if let error = model.placedError {
            ContentUnavailableView(
                "Map unavailable", systemImage: "exclamationmark.triangle", description: Text(error))
        } else if !query.trimmingCharacters(in: .whitespaces).isEmpty {
            ContentUnavailableView.search(text: query)
        } else {
            ContentUnavailableView(
                "Nothing on the map", systemImage: "map",
                description: Text(
                    filtered
                        ? String(localized: "No items with a place match the filter.")
                        : String(localized: "Open items with a place show here, nearest first.")))
        }
    }

    /// Show the item's page in the sheet, and its marker on the map
    /// above it.
    private func open(_ pin: Pinned) {
        searching = false
        withAnimation {
            camera = .region(MKCoordinateRegion(
                center: pin.coordinate, latitudinalMeters: 1500, longitudinalMeters: 1500))
        }
        path = [ItemRef(docId: pin.item.docId, itemId: pin.item.id, title: pin.item.text)]
    }
}

/// One row of the map's list: the item's text; under it, muted, where
/// its date when it has one, said short (`ItemView.shortWhenText`), and
/// where it is (the place's label, and how far away when the phone's
/// location is known); and on a third line the list it lives in, with the
/// workspace after it when that is not Personal, as the other lenses'
/// rows have them.
private struct PlacedRowView: View {
    let pin: Pinned
    let from: CLLocation?

    var body: some View {
        VStack(alignment: .leading, spacing: 2) {
            Text(pin.item.text)
                .itemTextSlot()
                // A gone-by event was not done: muted, not struck.
                .strikethrough(pin.item.item.state.isClosed)
                .foregroundStyle(closed ? .secondary : .primary)
            Text(note)
                .lineLimit(1)
                .font(.caption)
                .foregroundStyle(.secondary)
            if let home {
                Text(home)
                    .lineLimit(1)
                    .font(.caption)
                    .foregroundStyle(.tertiary)
            }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .contentShape(Rectangle())
    }

    private var closed: Bool { pin.item.isClosedOnMap }

    private var note: String {
        var parts: [String] = []
        if let when = pin.item.item.when { parts.append(ItemView.shortWhenText(when)) }
        parts.append(pin.place.label)
        if let from {
            let metres = Measurement(value: pin.metres(from: from), unit: UnitLength.meters)
            parts.append(metres.formatted(.measurement(width: .abbreviated, usage: .road)))
        }
        return parts.joined(separator: " · ")
    }

    private var home: String? {
        let list = NoList.display(pin.item.listName, listId: pin.item.item.listId)
        let parts = [list, pin.item.docName].compactMap { $0 }
        return parts.isEmpty ? nil : parts.joined(separator: " · ")
    }
}
