// An item's page: the web client's task dialog (js/web/src/TaskDialog.tsx)
// laid out top to bottom in the same order: status, title, list, planned
// date, deadline, place, notes, then the activity log. Read from
// `item show --json` and refetched on every doc change while shown.
// It comes up as a sheet over whatever list the row was in (half height,
// draggable to full), as the dialog floats over the web's lists, rather
// than as a pushed page. The checkbox ticks and reopens the item, the
// state label beside it opens a menu of the workflow states, a tap on
// the title hands the sheet to the title editor (`ItemTitleEditor`), the
// star at the top right stars and unstars it,
// the list field moves it, the date and place rows hand the sheet to their editors
// (`ItemDateEditor`, `ItemPlaceEditor`);
// everything else is display only.

import MapKit
import MonoplanMacKit
import SwiftUI

/// One item in one workspace: what a row opens. `docId` nil is the
/// Personal workspace; `title` is what the row already shows, so the
/// page has its heading before the fetch lands.
struct ItemRef: Hashable, Identifiable {
    let docId: String?
    let itemId: String
    let title: String

    var key: String { "\(docId ?? "")/\(itemId)" }
    var id: String { key }
}

private struct OpenItemKey: EnvironmentKey {
    static let defaultValue: (ItemRef) -> Void = { _ in }
}

extension EnvironmentValues {
    /// Open an item's sheet; set by `opensItems()`.
    var openItem: (ItemRef) -> Void {
        get { self[OpenItemKey.self] }
        set { self[OpenItemKey.self] = newValue }
    }
}

extension View {
    /// Item rows anywhere under this view (a tab's whole navigation
    /// stack, pushed pages included) open the item's sheet over it.
    func opensItems() -> some View { modifier(ItemSheet()) }
}

private struct ItemSheet: ViewModifier {
    @EnvironmentObject private var model: AppModel
    @State private var open: ItemRef?
    /// The sheet's height: half to start, full once a field expands
    /// (`ItemView` raises it) or the user drags it up.
    @State private var detent: PresentationDetent = .medium

    func body(content: Content) -> some View {
        content
            .environment(\.openItem) {
                detent = .medium
                open = $0
            }
            .sheet(item: $open) { ref in
                ItemView(ref: ref, detent: $detent)
                    .environmentObject(model)
                    .presentationDetents([.medium, .large], selection: $detent)
                    .presentationDragIndicator(.visible)
            }
    }
}

private struct ItemRefKey: EnvironmentKey {
    static let defaultValue: ItemRef? = nil
}

extension EnvironmentValues {
    /// The item a row stands for; set by `ItemLink`, read by the row's
    /// `CheckboxRow` for its two taps.
    var itemRef: ItemRef? {
        get { self[ItemRefKey.self] }
        set { self[ItemRefKey.self] = newValue }
    }
}

/// Names the item a list row stands for. The row itself (`CheckboxRow`)
/// does the work: its checkbox ticks the item, the rest of it opens the
/// item's sheet. No disclosure chevron: nothing is pushed.
struct ItemLink<Label: View>: View {
    let ref: ItemRef
    @ViewBuilder let label: Label

    init(_ ref: ItemRef, @ViewBuilder label: () -> Label) {
        self.ref = ref
        self.label = label()
    }

    var body: some View {
        label.environment(\.itemRef, ref)
    }
}

struct ItemView: View {
    @EnvironmentObject private var model: AppModel
    let ref: ItemRef
    @Binding var detent: PresentationDetent
    /// The field editor that has the sheet, if one does.
    @State private var editing: Editor?
    @State private var detentBeforeEditing: PresentationDetent = .medium

    private enum Editor {
        case title
        case date
        case place
    }

    /// An editor takes the sheet over, at full height; closing it gives
    /// the sheet back at the height it had.
    private func edit(_ editor: Editor) {
        detentBeforeEditing = detent
        detent = .large
        withAnimation(.easeOut(duration: 0.15)) { editing = editor }
    }

    private func endEditing() {
        detent = detentBeforeEditing
        withAnimation(.easeOut(duration: 0.15)) { editing = nil }
    }

    private var item: ItemDetail? { model.itemDetails[ref.key] }

    var body: some View {
        ZStack {
            if let editing, let item {
                Group {
                    switch editing {
                    case .title: ItemTitleEditor(ref: ref, item: item, done: endEditing)
                    case .date: ItemDateEditor(ref: ref, item: item, done: endEditing)
                    case .place: ItemPlaceEditor(ref: ref, item: item, done: endEditing)
                    }
                }
                .transition(.opacity)
            } else {
                page.transition(.opacity)
            }
        }
        .onAppear { model.refreshItem(ref) }
        .onChange(of: model.changeCount) { model.refreshItem(ref) }
    }

    private var page: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 20) {
                status
                Text(item?.text ?? ref.title)
                    .font(.title2.weight(.semibold))
                    .strikethrough(item?.state == "done")
                    .foregroundStyle(item?.closedAt == nil ? .primary : .secondary)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .contentShape(Rectangle())
                    // A tap edits the title (a binned item's stays as
                    // it is, like its other fields).
                    .onTapGesture {
                        if let item, item.binnedAt == nil { edit(.title) }
                    }
                    .accessibilityHint("Edits the title")
                if let item {
                    fields(item)
                    Divider()
                    notes(item)
                    Divider()
                    activity(item)
                } else if let error = model.itemErrors[ref.key] {
                    Text(error).foregroundStyle(.red)
                } else {
                    ProgressView().frame(maxWidth: .infinity)
                }
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            .padding()
            // Clear of the sheet's drag indicator.
            .padding(.top, 12)
        }
        // No pull-to-refresh: the page refetches on every doc change by
        // itself, and in a sheet a pull at the top has to mean "drag the
        // sheet down". Content that fits does not scroll at all, so the
        // same drag works from anywhere on it.
        .scrollBounceBehavior(.basedOnSize)
    }

    /// The checkbox and the workflow state beside it, drawn as one
    /// status control with two halves: the checkbox ticks and reopens,
    /// the label opens a menu of the states (the web dialog's status
    /// badge). A binned item just says so (the bin masks the state;
    /// Restore is the way out), and one that has not loaded shows an
    /// inert box.
    private var status: some View {
        HStack(spacing: 10) {
            if let item, item.binnedAt == nil {
                HStack(spacing: 0) {
                    ItemCheckButton(ref: ref, state: item.state)
                        .padding(.leading, 8)
                        .padding(.trailing, 8)
                    stateMenu(item)
                }
                .frame(minHeight: 36)
                .background(.quaternary.opacity(0.5), in: Capsule())
                // The capsule's own inset, taken back so the checkbox
                // stays in line with the title under it.
                .padding(.leading, -8)
            } else {
                ItemCheckbox(state: item?.state ?? "backlog")
                if item != nil {
                    Text("Binned")
                        .font(.subheadline.weight(.medium))
                        .foregroundStyle(.secondary)
                }
            }
            Spacer(minLength: 8)
            // Only an Open item can be starred (spec/focus.md): a
            // closed or binned one has no star to show or set.
            if let item, item.binnedAt == nil, item.closedAt == nil {
                StarButton(starred: item.starred) { model.setStarred(ref, starred: !item.starred) }
            }
        }
    }

    /// List, planned date, deadline (only when set, as on the web) and
    /// place. The date and place rows are always there, muted when
    /// empty: they are where setting one will happen.
    private func fields(_ item: ItemDetail) -> some View {
        VStack(alignment: .leading, spacing: 14) {
            listField(item)
            ItemDateField(item: item) { edit(.date) }
            if let deadline = item.deadline {
                FieldRow("timer", text: "Due \(Self.dayText(deadline))")
            }
            Divider()
            // As the date: a display row with the round button that
            // opens the editor; the map card stays under it.
            HStack(spacing: 8) {
                if let place = item.place {
                    FieldRow("map", text: place.label, note: place.address)
                } else {
                    FieldRow("map", text: "Place", empty: true)
                }
                Spacer(minLength: 8)
                EditBubble(adding: item.place == nil, what: "place") { edit(.place) }
                    .disabled(item.binnedAt != nil)
            }
            .frame(maxWidth: .infinity, minHeight: 36, alignment: .leading)
            if let place = item.place, let lat = place.lat, let lon = place.lon {
                PlaceMapCard(label: place.label, coordinate: CLLocationCoordinate2D(latitude: lat, longitude: lon))
            }
        }
    }

    /// The states a status menu offers. A task gets all six, in ladder
    /// order. An event (an item of the reserved `events` list) has no
    /// lanes, so its four open states are one choice, "Event", standing
    /// in as `backlog` (what reopening writes), beside the two closed
    /// ones (spec/events-plan.md).
    private static let taskStates = ["backlog", "todo", "in_progress", "review", "done", "cancelled"]
    private static let eventStates = ["backlog", "done", "cancelled"]

    private static func isClosed(_ state: String) -> Bool { state == "done" || state == "cancelled" }

    private static func statusLabel(_ state: String, event: Bool) -> String {
        event && !isClosed(state) ? "Event" : ItemRowView.stateLabel(state)
    }

    /// The state label as a pull-down menu: the current state ticked, a
    /// pick moves the item there in one write. On an Open event the
    /// single open choice is ticked whatever open state the item holds
    /// underneath, and picking it changes nothing, so a state the item
    /// brought in from a list is kept.
    private func stateMenu(_ item: ItemDetail) -> some View {
        let event = item.listId == "events"
        let picked = event && !Self.isClosed(item.state) ? "backlog" : item.state
        return Menu {
            Picker("Status", selection: Binding(
                get: { picked },
                set: { if $0 != picked { model.setLifecycle(ref, state: $0) } }
            )) {
                ForEach(event ? Self.eventStates : Self.taskStates, id: \.self) { state in
                    Text(Self.statusLabel(state, event: event)).tag(state)
                }
            }
        } label: {
            HStack(spacing: 4) {
                Text(Self.statusLabel(item.state, event: event))
                    .font(.subheadline.weight(.medium))
                Image(systemName: "chevron.up.chevron.down")
                    .font(.caption2)
            }
            .foregroundStyle(.secondary)
            .padding(.trailing, 12)
            // The capsule's full height, and out to its trailing edge.
            .frame(minHeight: 36)
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .sensoryFeedback(.selection, trigger: item.state)
        .accessibilityLabel("Status")
        .accessibilityValue(Self.statusLabel(item.state, event: event))
    }

    /// The menu's rows: the workspace's lists, plus the item's own list
    /// when it is not among them (archived, or the calendar's), so the
    /// picker always has a row to tick.
    private func listOptions(_ item: ItemDetail) -> [ListRef] {
        let lists = model.lists(inDoc: ref.docId)
        if lists.contains(where: { $0.listId == item.listId }) { return lists }
        let own = ListRef(docId: ref.docId ?? "", listId: item.listId, name: item.listLabel, icon: item.listIcon)
        return lists + [own]
    }

    /// The item's list, as a pull-down menu of the workspace's lists
    /// (the web dialog's list picker): the current one ticked, a pick
    /// moves the item there. Lists of other workspaces are not offered:
    /// the engine has no cross-doc move yet. A binned item stays put.
    private func listField(_ item: ItemDetail) -> some View {
        Menu {
            // A `Picker` in a menu is the system's single-choice menu:
            // it draws the tick for the current list itself, which
            // leaves each row's image slot for the list's icon.
            Picker("List", selection: Binding(
                get: { item.listId },
                set: { if $0 != item.listId { model.moveItem(ref, toList: $0) } }
            )) {
                ForEach(listOptions(item), id: \.listId) { list in
                    Label {
                        Text(list.name)
                    } icon: {
                        ListMenuIcon.image(listId: list.listId, emoji: list.icon)
                    }
                    .tag(list.listId)
                }
            }
        } label: {
            // Drawn as a field: full width, a faint fill, the picker
            // chevrons at its trailing edge, and a full-height (44pt)
            // tap target. Outset by its own padding so its icon and
            // text stay in line with the plain rows under it.
            HStack(spacing: 6) {
                FieldRow(text: item.listLabel) { ListIcon(listId: item.listId, emoji: item.listIcon) }
                Spacer(minLength: 8)
                Image(systemName: "chevron.up.chevron.down")
                    .font(.footnote)
                    .foregroundStyle(.secondary)
            }
            .itemField()
        }
        .buttonStyle(.plain)
        .disabled(item.binnedAt != nil)
        .itemFieldOutset()
    }

    @ViewBuilder
    private func notes(_ item: ItemDetail) -> some View {
        if item.notes.isEmpty {
            Text("Notes").foregroundStyle(.tertiary)
        } else {
            Text(item.notes).textSelection(.enabled)
        }
    }

    /// The item's timeline as plain sentences: created, and once closed,
    /// when and how long it took.
    private func activity(_ item: ItemDetail) -> some View {
        VStack(alignment: .leading, spacing: 4) {
            Text("Created \(Self.stamp(item.createdAt))")
            if let closedAt = item.closedAt {
                Text("\(item.state == "done" ? "Completed" : "Cancelled") \(Self.stamp(closedAt))")
                if let took = Self.elapsed(from: item.createdAt, to: closedAt) {
                    Text("Took \(took)")
                }
            }
        }
        .font(.footnote)
        .foregroundStyle(.secondary)
    }

    // MARK: Formatting

    /// A planned date: "Saturday 12 Sep" (with the year once it is not
    /// this one), and "at 14:00" after it when timed.
    static func whenText(_ when: String) -> String {
        let day = dayText(String(when.prefix(10)))
        guard when.count >= 16 else { return day }
        return "\(day) at \(when.dropFirst(11).prefix(5))"
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
    static func lengthText(_ item: ItemDetail) -> String? {
        guard let minutes = item.duration, minutes > 0 else { return nil }
        if item.isTimed { return AgendaRowView.span(minutes) }
        let days = (minutes + 1439) / 1440
        return days >= 2 ? "\(days) days" : nil
    }

    /// An instant for the activity log: the time alone today, otherwise
    /// the date and time.
    static func stamp(_ millis: Int64, now: Date = Date(), calendar: Calendar = .current) -> String {
        let date = Date(timeIntervalSince1970: Double(millis) / 1000)
        if calendar.isDate(date, inSameDayAs: now) {
            return "today \(date.formatted(date: .omitted, time: .shortened))"
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

/// The title editor, in place of the item's page for as long as it is
/// open: the title as a text field with the keyboard up, and the notes
/// under it, display only for now. Nothing is written until the tick
/// (or Return: a title is one line, as on the web); the cross drops
/// what was typed. An empty title cannot be kept.
struct ItemTitleEditor: View {
    @EnvironmentObject private var model: AppModel
    let ref: ItemRef
    let item: ItemDetail
    let done: () -> Void
    @State private var text: String
    @FocusState private var focused: Bool

    init(ref: ItemRef, item: ItemDetail, done: @escaping () -> Void) {
        self.ref = ref
        self.item = item
        self.done = done
        _text = State(initialValue: item.text)
    }

    private var trimmed: String { text.trimmingCharacters(in: .whitespacesAndNewlines) }

    private func commit() {
        guard !trimmed.isEmpty else { return }
        if trimmed != item.text { model.setText(ref, text: trimmed) }
        done()
    }

    var body: some View {
        VStack(spacing: 0) {
            EditorHeader(title: "Title", canFinish: !trimmed.isEmpty, cancel: done, done: commit)
            ScrollView {
                VStack(alignment: .leading, spacing: 20) {
                    // Wraps like the title it replaces. A wrapping
                    // field takes Return as a line break, so Return is
                    // caught as it lands and commits instead.
                    TextField("Title", text: $text, axis: .vertical)
                        .font(.title2.weight(.semibold))
                        .focused($focused)
                        .submitLabel(.done)
                        .onChange(of: text) {
                            guard text.contains(where: \.isNewline) else { return }
                            text = text.split(whereSeparator: \.isNewline).joined(separator: " ")
                            commit()
                        }
                    Divider()
                    if item.notes.isEmpty {
                        Text("Notes").foregroundStyle(.tertiary)
                    } else {
                        Text(item.notes).textSelection(.enabled)
                    }
                }
                .frame(maxWidth: .infinity, alignment: .leading)
                .padding()
            }
            .scrollBounceBehavior(.basedOnSize)
            .scrollDismissesKeyboard(.interactively)
        }
        .onAppear { focused = true }
    }
}

/// The star at the top right of an item's page: gold and filled when
/// the item is starred, an outline when it is not; a tap flips it.
private struct StarButton: View {
    let starred: Bool
    let action: () -> Void

    static let gold = Color(red: 1.0, green: 0.76, blue: 0.03)

    var body: some View {
        Button(action: action) {
            Image(systemName: starred ? "star.fill" : "star")
                .font(.title3)
                .foregroundStyle(starred ? AnyShapeStyle(Self.gold) : AnyShapeStyle(.secondary))
                // A full 44pt target around the glyph, without pushing
                // the status row taller or the star in from the edge.
                .frame(width: 44, height: 44, alignment: .trailing)
                .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .padding(.vertical, -10)
        .sensoryFeedback(.selection, trigger: starred)
        .accessibilityLabel(starred ? "Unstar" : "Star")
    }
}

/// A list's icon for a menu row. Menus take images only, so an emoji is
/// drawn into one (kept in its own colours); a list without an emoji
/// gets the same built-in glyphs as `ListIcon`.
private enum ListMenuIcon {
    static func image(listId: String, emoji: String?) -> Image {
        guard let emoji, !emoji.isEmpty else {
            return Image(systemName: listId == "inbox" ? "tray" : "folder")
        }
        let font = UIFont.systemFont(ofSize: 17)
        let text = emoji as NSString
        let size = text.size(withAttributes: [.font: font])
        let drawn = UIGraphicsImageRenderer(size: size).image { _ in
            text.draw(at: .zero, withAttributes: [.font: font])
        }
        return Image(uiImage: drawn.withRenderingMode(.alwaysOriginal))
    }
}

/// A place with coordinates as a small map: a pin at the spot, a few
/// streets around it. The map is a picture, not a control (no pan or
/// zoom, so it never fights the sheet's drag); a tap anywhere on it
/// opens the spot in Apple Maps under the place's own label. Map tiles
/// come from Apple, so showing the card tells Apple roughly where the
/// place is (as the web's lookup tells Komoot what was typed).
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
        .frame(height: 150)
        .clipShape(RoundedRectangle(cornerRadius: 12, style: .continuous))
        .contentShape(Rectangle())
        .onTapGesture { if let url = Self.mapsURL(label: label, coordinate: coordinate) { openURL(url) } }
        .accessibilityElement()
        .accessibilityLabel("Map of \(label)")
        .accessibilityHint("Opens in Maps")
        .accessibilityAddTraits(.isButton)
    }

    /// Apple Maps at the coordinates, the pin named by our label
    /// (`ll` places it, `q` names it): our place, not a search for
    /// Apple's.
    static func mapsURL(label: String, coordinate: CLLocationCoordinate2D) -> URL? {
        var parts = URLComponents(string: "https://maps.apple.com/")
        parts?.queryItems = [
            URLQueryItem(name: "ll", value: "\(coordinate.latitude),\(coordinate.longitude)"),
            URLQueryItem(name: "q", value: label),
        ]
        return parts?.url
    }
}

/// One field of the page: its icon, its value, and a secondary line
/// under it when there is one. `empty` mutes a field that is unset.
struct FieldRow<Icon: View>: View {
    let text: String
    var note: String? = nil
    var empty = false
    @ViewBuilder let icon: Icon

    var body: some View {
        HStack(alignment: .firstTextBaseline, spacing: 12) {
            icon
                .foregroundStyle(empty ? .tertiary : .secondary)
                .frame(width: 24)
            VStack(alignment: .leading, spacing: 2) {
                Text(text).foregroundStyle(empty ? .tertiary : .primary)
                if let note {
                    Text(note).font(.footnote).foregroundStyle(.secondary)
                }
            }
        }
    }
}

extension FieldRow where Icon == Image {
    /// A field led by an SF Symbol.
    init(_ systemImage: String, text: String, note: String? = nil, empty: Bool = false) {
        self.init(text: text, note: note, empty: empty) { Image(systemName: systemImage) }
    }
}

/// How far an editable field's box reaches past the plain rows.
let itemFieldInset: CGFloat = 12

extension View {
    /// An editable field's box: full width, a faint fill, a full-height
    /// (44pt) tap target.
    func itemField() -> some View {
        padding(.horizontal, itemFieldInset)
            .frame(maxWidth: .infinity, minHeight: 44, alignment: .leading)
            .background(.quaternary.opacity(0.5), in: RoundedRectangle(cornerRadius: 10, style: .continuous))
            .contentShape(RoundedRectangle(cornerRadius: 10, style: .continuous))
    }

    /// Outset by the box's own padding, so a field's icon and text stay
    /// in line with the plain rows around it.
    func itemFieldOutset() -> some View {
        padding(.horizontal, -itemFieldInset)
    }
}
