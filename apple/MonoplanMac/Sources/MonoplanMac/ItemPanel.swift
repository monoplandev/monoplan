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
// The place row is edited in place (`PlaceField.swift`). No other
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
    /// Every other open workspace with its lists, offered under the
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
                                .font(.title2.weight(.semibold))
                                .foregroundStyle(.secondary)
                                .textSelection(.enabled)
                        }
                        fields(item)
                            // The place list floats over the rows below.
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

    /// List, planned date, deadline (only when set) and place. The date
    /// and place rows are always there, muted when empty.
    private func fields(_ item: ItemInfo) -> some View {
        VStack(alignment: .leading, spacing: 12) {
            listField(item)
            if let when = item.item.when {
                PanelField(text: Self.whenText(when), note: Self.lengthText(item)) {
                    PanelGlyph(image: NavArt.calendar)
                }
            } else {
                PanelField(text: String(localized: "Date"), empty: true) { PanelGlyph(image: NavArt.calendar) }
            }
            if let deadline = item.item.deadline {
                PanelField(text: String(localized: "Due \(Self.dayText(deadline))")) { Image(systemName: "timer") }
            }
            if item.item.binnedAt == nil {
                PlaceField(place: item.item.place, knownPlaces: knownPlaces) { setPlace(item, $0, $1) }
                    .id(item.item.id)
                    // Its list floats over the map card under it.
                    .zIndex(1)
            } else if let place = item.item.place {
                PanelField(text: place.label, note: place.address) { PanelGlyph(image: NavArt.pin) }
            } else {
                PanelField(text: String(localized: "Place"), empty: true) { PanelGlyph(image: NavArt.pin) }
            }
            if let place = item.item.place, let lat = place.lat, let lon = place.lon {
                PlaceMapCard(label: place.label, coordinate: CLLocationCoordinate2D(latitude: lat, longitude: lon))
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

    /// The item's list, drawn as a field as on the iOS page
    /// (`ItemView.listField`): full width, a faint fill, outset by its
    /// own padding so its icon and text stay in line with the plain
    /// rows under it. A click opens a menu of the workspace's lists
    /// with the current one ticked, then every other workspace's
    /// under its name, as the web's list picker; a pick moves the
    /// item, across workspaces when the list is another one's
    /// (spec/sharing-plan.md "Cross-doc move"). A binned item stays
    /// put.
    ///
    /// A plain button that pops an `NSMenu`, not a SwiftUI `Menu`: a
    /// `Menu`'s label is the system's to draw, and it does not keep a
    /// box of our own.
    private func listField(_ item: ItemInfo) -> some View {
        Button {
            ListMenu.pop(
                listOptions(item), current: item.item.listId, others: otherOptions(item),
                pick: { move(item, $0) }, pickAcross: { moveAcross(item, $0, $1) })
        } label: {
            HStack(alignment: .firstTextBaseline, spacing: 10) {
                Group {
                    if let emoji = item.listIcon {
                        Text(verbatim: emoji)
                    } else {
                        Self.listIcon(item.item.listId, dated: item.item.when != nil)
                    }
                }
                .foregroundStyle(.secondary)
                .frame(width: 20)
                Text(verbatim: item.listLabel).lineLimit(1)
                // A shared workspace's name after the list's, as on
                // iOS (`SharedWorkspaceBadge`): the shared glyph, then
                // the name in the same font, muted. First to truncate.
                if let workspaceName {
                    HStack(alignment: .firstTextBaseline, spacing: 4) {
                        PanelGlyph(image: NavArt.shared, size: 13).accessibilityHidden(true)
                        Text(verbatim: workspaceName).lineLimit(1)
                    }
                    .foregroundStyle(.secondary)
                    .layoutPriority(-1)
                }
                Spacer(minLength: 8)
                Image(systemName: "chevron.up.chevron.down")
                    .font(.footnote)
                    .foregroundStyle(.secondary)
            }
            .padding(.horizontal, Self.fieldInset)
            .frame(maxWidth: .infinity, minHeight: 32, alignment: .leading)
            .background(.quaternary.opacity(0.5), in: RoundedRectangle(cornerRadius: 8, style: .continuous))
            .contentShape(RoundedRectangle(cornerRadius: 8, style: .continuous))
        }
        .buttonStyle(.plain)
        .disabled(item.item.binnedAt != nil)
        .padding(.horizontal, -Self.fieldInset)
        .accessibilityHint(Text("Moves the item to another list"))
    }

    private static let fieldInset: CGFloat = 8

    /// The menu's rows: the workspace's lists, plus the item's own
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
                workspace: group.workspace, name: group.name,
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
    private static func listIcon(_ listId: String, dated: Bool) -> some View {
        switch listId {
        case MainWindowModel.inbox: PanelGlyph(image: dated ? NavArt.calendar : NavArt.inbox)
        default: Image(systemName: "list.bullet")
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

/// The list field's menu: the item's workspace's lists, the current
/// one ticked, then each other workspace's under its name.
private final class ListMenu: NSObject {
    private let pick: (String) -> Void
    private let pickAcross: (String?, String) -> Void
    /// The other workspaces' rows, by the menu item's tag.
    private var across: [(workspace: String?, list: String)] = []

    private init(pick: @escaping (String) -> Void, pickAcross: @escaping (String?, String) -> Void) {
        self.pick = pick
        self.pickAcross = pickAcross
    }

    /// Open it at the pointer; `pick` gets the id of a list other
    /// than the current one, `pickAcross` another workspace and a
    /// list of it. Returns once the menu has shut.
    static func pop(
        _ lists: [PanelList], current: String, others: [PanelListGroup],
        pick: @escaping (String) -> Void, pickAcross: @escaping (String?, String) -> Void
    ) {
        let handler = ListMenu(pick: pick, pickAcross: pickAcross)
        let menu = NSMenu()
        for list in lists {
            let item = NSMenuItem(title: title(list), action: #selector(picked(_:)), keyEquivalent: "")
            item.target = handler
            item.representedObject = list.id
            item.state = list.id == current ? .on : .off
            menu.addItem(item)
        }
        for group in others {
            menu.addItem(.separator())
            menu.addItem(.sectionHeader(title: group.name))
            for list in group.lists {
                let item = NSMenuItem(
                    title: title(list), action: #selector(pickedAcross(_:)), keyEquivalent: "")
                item.target = handler
                item.tag = handler.across.count
                handler.across.append((group.workspace, list.id))
                menu.addItem(item)
            }
        }
        let ticked = menu.items.first { $0.state == .on }
        // The ticked row lands under the pointer, as a pop-up button's.
        menu.popUp(positioning: ticked, at: NSEvent.mouseLocation, in: nil)
        // `handler` is the items' (weak) target: kept until here.
        withExtendedLifetime(handler) {}
    }

    private static func title(_ list: PanelList) -> String {
        [list.icon, list.name].compactMap { $0 }.joined(separator: " ")
    }

    @objc private func picked(_ sender: NSMenuItem) {
        guard let id = sender.representedObject as? String, sender.state != .on else { return }
        pick(id)
    }

    @objc private func pickedAcross(_ sender: NSMenuItem) {
        guard across.indices.contains(sender.tag) else { return }
        let row = across[sender.tag]
        pickAcross(row.workspace, row.list)
    }
}

/// A list the panel's item can be moved to.
struct PanelList: Identifiable {
    let id: String
    let name: String
    let icon: String?
}

/// Another workspace's lists, for the list menu: what a typed call's
/// `workspace` takes (nil is Home) and the name its heading shows.
struct PanelListGroup {
    let workspace: String?
    let name: String
    let lists: [PanelList]
}

/// One field of the panel: its icon, its value, and a secondary line
/// under it when there is one. `empty` mutes a field that is unset.
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
        .font(.title2.weight(.semibold))
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
