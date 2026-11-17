// The item panel: the selected row's item, at the right of the window.
// The iOS item page (apple/MonoplanIOS/Monoplan/ItemView.swift) laid
// out the same way, top to bottom: status, title, list, planned date,
// deadline, place, notes, then the activity log. Read from
// `Engine.item` in the window's snapshot, so it is reread on every doc
// change like the rest.
//
// Read only but for the two writes the window already had: the
// checkbox ticks and reopens the item, the star stars and unstars it.
// Both are toolbar items with the close button, over the panel in the
// window's bar and on the system's glass, as the iOS page's navigation
// bar (`MainSplit.swift`).
// The list row moves the item (`ListField.swift`); the planned date
// (`DateField.swift`) and the place (`PlaceField.swift`) are edited in
// place. No other
// field editors, no state menu, no bin: those come with the views
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
    /// The lists the item can be moved to: its workspace's, Inbox
    /// first.
    let lists: [PanelList]
    /// Every other open workspace with its lists, offered after the
    /// item's own: a pick there moves the item across workspaces.
    let otherLists: [PanelListGroup]
    /// The title was edited: the new one, trimmed, not empty and not
    /// the old one.
    let retitle: (ItemInfo, String) -> Void
    /// Another list was picked for the item: its id.
    let move: (ItemInfo, String) -> Void
    /// A list of another workspace was picked: the workspace (as a
    /// typed call takes it, nil being Home) and the list's id.
    let moveAcross: (ItemInfo, String?, String) -> Void
    /// The account's known places matching the text.
    let knownPlaces: (String, @escaping ([PlaceSuggestion]) -> Void) -> Void
    /// A place was picked for the item, or cleared (nil); `exact` for
    /// a known place, written as it is.
    let setPlace: (ItemInfo, Place?, Bool) -> Void
    /// A date was picked for the item, or cleared (nil), with what
    /// becomes of its duration.
    let setWhen: (ItemInfo, String?, DurationChange) -> Void

    var body: some View {
        Group {
            if let item {
                ScrollView {
                    VStack(alignment: .leading, spacing: 16) {
                        // The bin masks an item: nothing of it is
                        // edited until it is restored.
                        if item.item.binnedAt == nil {
                            PanelTitle(text: item.item.text, closed: item.closedAt != nil) { retitle(item, $0) }
                                .id(item.item.id)
                        } else {
                            Text(verbatim: item.item.text)
                                .font(.title2.weight(.medium))
                                .foregroundStyle(.secondary)
                                .textSelection(.enabled)
                        }
                        fields(item)
                            // The list and place lists float over the rows below.
                            .zIndex(1)
                        PanelRule()
                        notes(item)
                        PanelRule()
                        activity(item)
                    }
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .padding(16)
                }
            } else {
                Color.clear
            }
        }
        .frame(maxHeight: .infinity)
    }

    /// List, a rule, planned date, its time under it when timed,
    /// deadline (only when set), a rule, then the place. The date and
    /// place rows are always there, muted when empty. Each row is a
    /// box (`panelBox`): the list's is always filled, the others' only
    /// under the pointer.
    private func fields(_ item: ItemInfo) -> some View {
        VStack(alignment: .leading, spacing: 4) {
            ListField(
                item: item, workspaceName: workspaceName,
                lists: listOptions(item), others: otherOptions(item),
                pick: { move(item, $0) }, pickAcross: { moveAcross(item, $0, $1) }
            )
            .id(item.item.id)
            // Its list floats over every row under it, the date and
            // place fields' included.
            .zIndex(3)
            // The list, the date rows and the place are set apart: a
            // rule between each, with 16 pt between it and a row's
            // text (the stack's 4, 4 more, and the 8 inside a box).
            PanelRule().padding(.vertical, 4)
            if item.item.binnedAt == nil {
                DateField(when: item.item.when, duration: item.item.duration.map(Int.init)) {
                    setWhen(item, $0, $1)
                }
                .id(item.item.id)
                // Its offers and month float over the rows under it.
                .zIndex(2)
            } else if let when = item.item.when {
                PanelField(
                    text: Self.dayText(String(when.prefix(10))),
                    note: Self.timeText(when) == nil ? Self.lengthText(item) : nil
                ) {
                    PanelGlyph(image: NavArt.calendar)
                }
            } else {
                PanelField(text: String(localized: "Date"), empty: true) { PanelGlyph(image: NavArt.calendar) }
            }
            // A timed date has its times on a row of their own under
            // the day, in two columns: when it starts, then when it
            // ends (a muted "End" while it has no length).
            if let time = item.item.when.flatMap(Self.timeText) {
                // Each box is outset by its inset, so 4 pt shows
                // between the two.
                HStack(spacing: PanelBox.inset * 2 + 4) {
                    PanelField(text: time) { Image(systemName: "clock") }
                    let end = Self.endTimeText(item)
                    PanelField(text: end ?? String(localized: "End"), empty: end == nil) {
                        Image(systemName: "arrow.right")
                    }
                }
            }
            if let deadline = item.item.deadline {
                PanelField(text: String(localized: "Due \(Self.dayText(deadline))")) { Image(systemName: "timer") }
            }
            PanelRule().padding(.vertical, 4)
            if item.item.binnedAt == nil {
                PlaceField(place: item.item.place, knownPlaces: knownPlaces) { setPlace(item, $0, $1) }
                    .id(item.item.id)
                    // Its list floats over the map card under it.
                    .zIndex(1)
            } else if let place = item.item.place {
                PanelField(text: place.label) { PanelGlyph(image: NavArt.pin) }
            } else {
                PanelField(text: String(localized: "Place"), empty: true) { PanelGlyph(image: NavArt.pin) }
            }
            if let place = item.item.place, let lat = place.lat, let lon = place.lon {
                PlaceMapCard(label: place.label, coordinate: CLLocationCoordinate2D(latitude: lat, longitude: lon))
                    .padding(.top, 4)
            }
            // The address whole, under the map (under the place's row
            // when it has no map): every line of it, never cut short.
            if let address = item.item.place?.address, !address.isEmpty {
                Text(verbatim: address)
                    .font(.footnote)
                    .foregroundStyle(.secondary)
                    .fixedSize(horizontal: false, vertical: true)
                    .textSelection(.enabled)
                    .padding(.top, 4)
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

    /// The list picker's rows: the workspace's lists, plus the item's own
    /// when it is not among them (archived, or the calendar's), so
    /// there is always a row to tick.
    private func listOptions(_ item: ItemInfo) -> [PanelList] {
        // For an item with a date, "no list" is the Calendar, not Inbox.
        let lists = lists.map { Self.noList($0, dated: item.item.when != nil) }
        if lists.contains(where: { $0.id == item.item.listId }) { return lists }
        return lists + [PanelList(id: item.item.listId, name: item.listLabel, icon: item.listIcon)]
    }

    /// The other workspaces' rows, "no list" named the same way.
    private func otherOptions(_ item: ItemInfo) -> [PanelListGroup] {
        otherLists.map { group in
            PanelListGroup(
                workspace: group.workspace, name: group.name, shared: group.shared,
                lists: group.lists.map { Self.noList($0, dated: item.item.when != nil) })
        }
    }

    private static func noList(_ list: PanelList, dated: Bool) -> PanelList {
        dated && list.id == MainWindowModel.inbox
            ? PanelList(id: list.id, name: MainWindowModel.noListLabel(dated: true), icon: nil)
            : list
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

    /// The workflow state's name. An event has no lanes, so its open
    /// states all read "Event" (spec/events-plan.md).
    static func statusLabel(_ item: ItemInfo) -> String {
        statusLabel(item.item.state, event: item.item.event)
    }

    static func statusLabel(_ state: ItemState, event: Bool) -> String {
        if event, !state.isClosed { return String(localized: "Event") }
        switch state {
        case .backlog: return String(localized: "Backlog")
        case .todo: return String(localized: "To do")
        case .inProgress: return String(localized: "In progress")
        case .review: return String(localized: "Review")
        case .done: return String(localized: "Done")
        case .cancelled: return String(localized: "Cancelled")
        }
    }

    /// The states a status menu offers, as on iOS. A task gets all
    /// six, in ladder order. An event has no lanes, so its four open
    /// states are one choice, "Event", standing in as `backlog` (what
    /// reopening writes), beside the two closed ones
    /// (spec/events-plan.md).
    static let taskStates: [ItemState] = [.backlog, .todo, .inProgress, .review, .done, .cancelled]
    static let eventStates: [ItemState] = [.backlog, .done, .cancelled]

    /// A list's glyph when it has no emoji: the sidebar's for the
    /// Inbox (the Calendar's when the item has a date and so lives
    /// there), a bullet list for any other.
    @ViewBuilder
    static func listIcon(_ listId: String, dated: Bool) -> some View {
        switch listId {
        case MainWindowModel.inbox: PanelGlyph(image: dated ? NavArt.calendar : NavArt.inbox)
        default: Image(systemName: "list.bullet")
        }
    }

    /// A planned date's clock time, "14:00"; nil when it has none.
    static func timeText(_ when: String) -> String? {
        when.count >= 16 ? String(when.dropFirst(11).prefix(5)) : nil
    }

    /// When a timed date ends, "15:30": its start and its length, on
    /// the clock alone (an end past midnight is the next day's time).
    /// Nil without a time or a length.
    static func endTimeText(_ item: ItemInfo) -> String? {
        guard let when = item.item.when, let start = timeText(when),
              let minutes = item.item.duration.map(Int.init), minutes > 0,
              let hour = Int(start.prefix(2)), let minute = Int(start.suffix(2))
        else { return nil }
        let end = (hour * 60 + minute + minutes) % 1440
        return String(format: "%02d:%02d", end / 60, end % 60)
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
        case MainWindowModel.inbox: return MainWindowModel.noListLabel(dated: item.when != nil)
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

/// A list the panel's item can be moved to.
struct PanelList: Identifiable {
    let id: String
    let name: String
    let icon: String?
}

/// Another workspace's lists, for the list picker: what a typed call's
/// `workspace` takes (nil is Home), its name, and whether it is a
/// shared one (its rows name it only then).
struct PanelListGroup {
    let workspace: String?
    let name: String
    let shared: Bool
    let lists: [PanelList]
}

/// One of `NavArt`'s glyphs as a panel row's icon, where a symbol
/// would be: an image's baseline is its bottom edge, so it is lowered
/// to sit on the text's baseline the way a symbol does.
struct PanelGlyph: View {
    let image: NSImage
    var size: CGFloat = 15

    var body: some View {
        NavIcon(image: image, size: size)
            .alignmentGuide(.firstTextBaseline) { $0[.bottom] - size * 0.2 }
    }
}

/// One field of the panel, in the panel's box (`panelBox`): its icon,
/// its value, and a muted note at the trailing edge when there is one.
/// `empty` mutes a field that is unset.
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
            Text(verbatim: text)
                .lineLimit(1)
                .foregroundStyle(empty ? .tertiary : .primary)
                .textSelection(.enabled)
            if let note {
                Spacer(minLength: 8)
                Text(verbatim: note).font(.footnote).foregroundStyle(.secondary).lineLimit(1)
            }
        }
        .panelBox(filled: false)
    }
}

/// The box every field of the panel sits in: full width, its icon and
/// text inset, outset by that inset so they stay in line with the
/// title over them and the notes under. Its faint fill shows always
/// when `filled` (the list's, a search box), else only under the
/// pointer.
enum PanelBox {
    static let inset: CGFloat = 8
    static let height: CGFloat = 32
    static let radius: CGFloat = 8
}

private struct PanelBoxModifier: ViewModifier {
    let filled: Bool
    @State private var hovered = false

    func body(content: Content) -> some View {
        let shape = RoundedRectangle(cornerRadius: PanelBox.radius, style: .continuous)
        content
            .padding(.horizontal, PanelBox.inset)
            .frame(maxWidth: .infinity, minHeight: PanelBox.height, alignment: .leading)
            .background(.quaternary.opacity(filled || hovered ? 0.5 : 0), in: shape)
            .contentShape(shape)
            .onHover { hovered = $0 }
            .padding(.horizontal, -PanelBox.inset)
    }
}

extension View {
    func panelBox(filled: Bool = true) -> some View {
        modifier(PanelBoxModifier(filled: filled))
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

/// The item's title, edited in place: it reads as the heading it was
/// and takes the caret on a click. Return or the focus leaving commits;
/// an empty title is not one, and puts the old one back. Escape is
/// the panel's (it shuts), and the edit so far is committed as the
/// focus goes.
private struct PanelTitle: View {
    /// The item's title as the engine has it.
    let text: String
    let closed: Bool
    let commit: (String) -> Void
    @State private var draft: String
    @FocusState private var focused: Bool

    init(text: String, closed: Bool, commit: @escaping (String) -> Void) {
        self.text = text
        self.closed = closed
        self.commit = commit
        _draft = State(initialValue: text)
    }

    var body: some View {
        // Wraps like the heading it replaces. Unfocused, a vertical
        // field sizes itself for every line but can draw one short
        // (macOS 26, probed 2026-11-15), so the title is drawn by a
        // `Text` under it and the field's own text is clear until it
        // has the focus. The field stays opaque and on top: at opacity
        // 0 it takes no clicks. The two wrap and sit identically.
        ZStack(alignment: .topLeading) {
            Text(verbatim: draft)
                .foregroundStyle(ink)
                .opacity(focused ? 0 : 1)
            field
        }
        .font(.title2.weight(.medium))
    }

    private var ink: Color { closed ? .secondary : .primary }

    private var field: some View {
        TextField("Title", text: $draft, axis: .vertical)
            .textFieldStyle(.plain)
            .foregroundStyle(focused ? ink : .clear)
            .focused($focused)
            .onSubmit { focused = false }
            .onChange(of: draft) {
                // A title is one line: a break typed or pasted in is a
                // space, and a typed one is Return.
                guard draft.contains(where: \.isNewline) else { return }
                draft = draft.split(whereSeparator: \.isNewline).joined(separator: " ")
                focused = false
            }
            .onChange(of: focused) { if !focused { finish() } }
            // Changed elsewhere (another device, an undo): shown,
            // unless this one is being typed in.
            .onChange(of: text) { if !focused { draft = text } }
    }

    private func finish() {
        let trimmed = draft.trimmingCharacters(in: .whitespacesAndNewlines)
        if trimmed.isEmpty || trimmed == text {
            draft = text
        } else {
            draft = trimmed
            commit(trimmed)
        }
    }
}
