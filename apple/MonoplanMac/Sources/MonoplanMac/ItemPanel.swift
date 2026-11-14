// The item panel: the selected row's item, at the right of the window.
// The iOS item page (apple/MonoplanIOS/Monoplan/ItemView.swift) laid
// out the same way, top to bottom: status, title, list, planned date,
// deadline, place, notes, then the activity log. Read from
// `Engine.item` in the window's snapshot, so it is reread on every doc
// change like the rest.
//
// Read only but for the two writes the window already had: the
// checkbox ticks and reopens the item, the star stars and unstars it.
// No field editors, no state menu, no bin: those come with the views
// shared with iOS.

import MapKit
import MonoplanCore
import MonoplanMacKit
import SwiftUI

struct ItemPanel: View {
    /// Nil until the read for a new selection lands.
    let item: ItemInfo?
    /// The item's workspace when it is a shared one.
    let workspaceName: String?
    let setDone: (Bool) -> Void
    let setStarred: (Bool) -> Void
    let close: () -> Void

    var body: some View {
        VStack(spacing: 0) {
            header
            Divider()
            if let item {
                ScrollView {
                    VStack(alignment: .leading, spacing: 16) {
                        Text(verbatim: item.item.text)
                            .font(.title2.weight(.semibold))
                            .foregroundStyle(item.closedAt == nil ? .primary : .secondary)
                            .textSelection(.enabled)
                        fields(item)
                        Divider()
                        notes(item)
                        Divider()
                        activity(item)
                    }
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .padding(16)
                }
            } else {
                Spacer()
            }
        }
        .frame(maxHeight: .infinity)
    }

    /// The status at the left (the checkbox and the workflow state; a
    /// binned item just says so, the bin masking its state), the star
    /// and the close button at the right. Only an Open item can be
    /// starred (spec/focus.md).
    private var header: some View {
        HStack(spacing: 8) {
            if let item {
                if item.item.binnedAt != nil {
                    Text("Binned").foregroundStyle(.secondary)
                } else {
                    let closed = item.item.state.isClosed
                    Button {
                        setDone(!closed)
                    } label: {
                        Image(systemName: closed ? "checkmark.circle.fill" : "circle")
                    }
                    .buttonStyle(.plain)
                    .accessibilityLabel(closed ? Text("Mark as not done") : Text("Mark as done"))
                    Text(verbatim: Self.statusLabel(item))
                        .font(.subheadline.weight(.medium))
                        .foregroundStyle(.secondary)
                }
            }
            Spacer()
            if let item, item.item.binnedAt == nil, item.closedAt == nil {
                Button {
                    setStarred(!item.starred)
                } label: {
                    Image(systemName: item.starred ? "star.fill" : "star")
                        .foregroundStyle(item.starred ? Color.yellow : Color.secondary)
                }
                .buttonStyle(.plain)
                .accessibilityLabel(item.starred ? Text("Remove star") : Text("Star"))
            }
            Button(action: close) {
                Image(systemName: "xmark")
            }
            .buttonStyle(.plain)
            .foregroundStyle(.secondary)
            .keyboardShortcut(.cancelAction)
            .accessibilityLabel(Text("Close"))
        }
        .padding(.horizontal, 12)
        .padding(.vertical, 8)
    }

    /// List, planned date, deadline (only when set) and place. The date
    /// and place rows are always there, muted when empty.
    private func fields(_ item: ItemInfo) -> some View {
        VStack(alignment: .leading, spacing: 12) {
            PanelField(text: item.listLabel, note: workspaceName) {
                if let emoji = item.listIcon {
                    Text(verbatim: emoji)
                } else {
                    Image(systemName: Self.listSymbol(item.item.listId))
                }
            }
            if let when = item.item.when {
                PanelField(text: Self.whenText(when), note: Self.lengthText(item)) {
                    Image(systemName: "calendar")
                }
            } else {
                PanelField(text: String(localized: "Date"), empty: true) { Image(systemName: "calendar") }
            }
            if let deadline = item.item.deadline {
                PanelField(text: String(localized: "Due \(Self.dayText(deadline))")) { Image(systemName: "timer") }
            }
            if let place = item.item.place {
                PanelField(text: place.label, note: place.address) { Image(systemName: "mappin.and.ellipse") }
                if let lat = place.lat, let lon = place.lon {
                    PlaceMapCard(label: place.label, coordinate: CLLocationCoordinate2D(latitude: lat, longitude: lon))
                }
            } else {
                PanelField(text: String(localized: "Place"), empty: true) { Image(systemName: "mappin.and.ellipse") }
            }
        }
    }

    @ViewBuilder
    private func notes(_ item: ItemInfo) -> some View {
        if item.notes.isEmpty {
            Text("Notes").foregroundStyle(.tertiary)
        } else {
            Text(verbatim: item.notes).textSelection(.enabled)
        }
    }

    /// The item's timeline as plain sentences: created, and once closed,
    /// when and how long it took.
    private func activity(_ item: ItemInfo) -> some View {
        VStack(alignment: .leading, spacing: 4) {
            Text("Created \(Self.stamp(item.item.createdAt))")
            if let closedAt = item.closedAt {
                if item.item.state == .done {
                    Text("Completed \(Self.stamp(closedAt))")
                } else {
                    Text("Cancelled \(Self.stamp(closedAt))")
                }
                if let took = Self.elapsed(from: item.item.createdAt, to: closedAt) {
                    Text("Took \(took)")
                }
            }
        }
        .font(.footnote)
        .foregroundStyle(.secondary)
    }

    // MARK: Formatting

    /// The workflow state's name. An event (an item of the reserved
    /// `events` list) has no lanes, so its open states all read
    /// "Event" (spec/events-plan.md).
    static func statusLabel(_ item: ItemInfo) -> String {
        let state = item.item.state
        if item.item.listId == MainWindowModel.events, !state.isClosed { return String(localized: "Event") }
        switch state {
        case .backlog: return String(localized: "Backlog")
        case .todo: return String(localized: "To do")
        case .inProgress: return String(localized: "In progress")
        case .review: return String(localized: "Review")
        case .done: return String(localized: "Done")
        case .cancelled: return String(localized: "Cancelled")
        }
    }

    static func listSymbol(_ listId: String) -> String {
        switch listId {
        case MainWindowModel.inbox: return "tray"
        case MainWindowModel.events: return "calendar"
        default: return "list.bullet"
        }
    }

    /// A planned date: "Saturday 12 Sep" (with the year once it is not
    /// this one), and "at 14:00" after it when timed.
    static func whenText(_ when: String) -> String {
        let day = dayText(String(when.prefix(10)))
        guard when.count >= 16 else { return day }
        return String(localized: "\(day) at \(String(when.dropFirst(11).prefix(5)))")
    }

    /// A `YYYY-MM-DD` spelled out in the local calendar; the raw text if
    /// it does not parse.
    static func dayText(_ day: String, now: Date = Date(), calendar: Calendar = .current) -> String {
        guard let date = Agenda.start(of: day, calendar: calendar) else { return day }
        if calendar.isDate(date, equalTo: now, toGranularity: .year) {
            return date.formatted(.dateTime.weekday(.wide).day().month(.abbreviated))
        }
        return date.formatted(.dateTime.weekday(.wide).day().month(.abbreviated).year())
    }

    /// How long it runs: the duration beside a timed date ("1h 30m"),
    /// whole days beside an all-day one once it spans more than one
    /// (spec/calendar-plan.md).
    static func lengthText(_ item: ItemInfo) -> String? {
        guard let minutes = item.item.duration.map(Int.init), minutes > 0 else { return nil }
        if (item.item.when?.count ?? 0) >= 16 { return TodayMenu.span(TimeInterval(minutes * 60)) }
        let days = (minutes + 1439) / 1440
        return days >= 2 ? String(localized: "\(days) days") : nil
    }

    /// An instant for the activity log: the time alone today, otherwise
    /// the date and time.
    static func stamp(_ millis: Int64, now: Date = Date(), calendar: Calendar = .current) -> String {
        let date = Date(timeIntervalSince1970: Double(millis) / 1000)
        if calendar.isDate(date, inSameDayAs: now) {
            return String(localized: "today \(date.formatted(date: .omitted, time: .shortened))")
        }
        return date.formatted(date: .abbreviated, time: .shortened)
    }

    /// "2 days, 3 hours": two units at most, the larger first.
    static func elapsed(from start: Int64, to end: Int64) -> String? {
        let formatter = DateComponentsFormatter()
        formatter.unitsStyle = .full
        formatter.maximumUnitCount = 2
        formatter.allowedUnits = [.year, .month, .day, .hour, .minute, .second]
        return formatter.string(from: TimeInterval(max(0, end - start)) / 1000)
    }
}

extension ItemState {
    /// Done or Cancelled: nothing left to do.
    var isClosed: Bool { self == .done || self == .cancelled }
}

extension ItemInfo {
    /// The list's display name: the doc's, else the reserved label.
    var listLabel: String {
        if let listName { return listName }
        switch item.listId {
        case MainWindowModel.inbox: return String(localized: "Inbox")
        case MainWindowModel.events: return String(localized: "Calendar")
        default: return item.listId
        }
    }

    /// When the item was closed, while it is: the completion stamp for
    /// Done (falling back to the register's transition), the register's
    /// own instant for Cancelled, which stamps nothing.
    var closedAt: Int64? {
        switch item.state {
        case .done: return item.doneAt ?? item.lifecycleAt
        case .cancelled: return item.lifecycleAt
        default: return nil
        }
    }
}

/// One field of the panel: its icon, its value, and a secondary line
/// under it when there is one. `empty` mutes a field that is unset.
private struct PanelField<Icon: View>: View {
    let text: String
    var note: String? = nil
    var empty = false
    @ViewBuilder let icon: Icon

    var body: some View {
        HStack(alignment: .firstTextBaseline, spacing: 10) {
            icon
                .foregroundStyle(empty ? .tertiary : .secondary)
                .frame(width: 20)
            VStack(alignment: .leading, spacing: 2) {
                Text(verbatim: text)
                    .foregroundStyle(empty ? .tertiary : .primary)
                    .textSelection(.enabled)
                if let note {
                    Text(verbatim: note).font(.footnote).foregroundStyle(.secondary)
                }
            }
        }
    }
}

/// A place with coordinates as a small map: a pin at the spot, a few
/// streets around it. A picture, not a control; a click opens the spot
/// in Apple Maps under the place's own label. Map tiles come from
/// Apple, so showing the card tells Apple roughly where the place is.
private struct PlaceMapCard: View {
    @Environment(\.openURL) private var openURL
    let label: String
    let coordinate: CLLocationCoordinate2D

    var body: some View {
        Map(
            initialPosition: .region(MKCoordinateRegion(
                center: coordinate, latitudinalMeters: 600, longitudinalMeters: 600)),
            interactionModes: []
        ) {
            Marker(label, coordinate: coordinate)
        }
        // A new place is a new map: `initialPosition` is read once.
        .id("\(coordinate.latitude),\(coordinate.longitude)")
        .allowsHitTesting(false)
        .frame(height: 140)
        .clipShape(RoundedRectangle(cornerRadius: 10, style: .continuous))
        .contentShape(Rectangle())
        .onTapGesture { if let url = mapsURL { openURL(url) } }
        .accessibilityElement()
        .accessibilityLabel(Text("Map of \(label)"))
        .accessibilityAddTraits(.isButton)
    }

    /// Apple Maps at the coordinates, the pin named by our label.
    private var mapsURL: URL? {
        var parts = URLComponents(string: "https://maps.apple.com/")
        parts?.queryItems = [
            URLQueryItem(name: "ll", value: "\(coordinate.latitude),\(coordinate.longitude)"),
            URLQueryItem(name: "q", value: label),
        ]
        return parts?.url
    }
}
