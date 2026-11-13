// An item's page: the web client's task dialog (js/web/src/TaskDialog.tsx)
// laid out top to bottom in the same order: status, title, list, planned
// date, deadline, place, notes, then the activity log. Read from
// `item show --json` and refetched on every doc change while shown.
// It comes up as a sheet over whatever list the row was in (half height,
// draggable to full), as the dialog floats over the web's lists, rather
// than as a pushed page. The sheet has a navigation bar, its buttons the
// system's Liquid Glass: at the left the status control (the checkbox
// ticks and reopens the item, the state label beside it opens a menu of
// the workflow states) after the "x", which closes the sheet (as a drag
// down does); at the right the star, which stars and unstars it, and
// the more menu. A tap
// on the title hands the sheet to the title editor (`ItemTitleEditor`),
// the list field and the date and place rows hand the sheet to their
// editors (`ItemListEditor`, `ItemDateEditor`, `ItemPlaceEditor`);
// everything else is display only.
// The same sheet is where a new item is captured (`ItemCaptureView`,
// opened by the bottom bar's add button): a title field, and this page
// from the moment the title makes it a real item.

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

private struct CaptureItemKey: EnvironmentKey {
    static let defaultValue: (ListRef?) -> Void = { _ in }
}

extension EnvironmentValues {
    /// Open the sheet on a new item for this list (nil: the Personal
    /// Inbox); set by `opensItems()`.
    var captureItem: (ListRef?) -> Void {
        get { self[CaptureItemKey.self] }
        set { self[CaptureItemKey.self] = newValue }
    }
}

extension View {
    /// Item rows anywhere under this view (Home's whole navigation
    /// stack, pushed pages included) open the item's sheet over it,
    /// and `captureItem` opens the same sheet on a new one.
    func opensItems() -> some View { modifier(ItemSheet()) }
}

/// One capture: the list it starts aimed at. Each has its own identity,
/// so the sheet stays up while the capture inside it becomes an item.
struct CaptureTarget: Identifiable {
    let id = UUID()
    let list: ListRef
}

/// What the sheet is up for: an item, or a capture that becomes one.
private enum SheetTarget: Identifiable {
    case item(ItemRef)
    case capture(CaptureTarget)

    var id: String {
        switch self {
        case .item(let ref): return ref.key
        case .capture(let target): return target.id.uuidString
        }
    }
}

private struct ItemSheet: ViewModifier {
    @EnvironmentObject private var model: AppModel
    @State private var open: SheetTarget?
    /// The sheet's height: half to start, full once a field expands
    /// (`ItemView` raises it) or the user drags it up.
    @State private var detent: PresentationDetent = .medium

    func body(content: Content) -> some View {
        content
            .environment(\.openItem) {
                detent = .medium
                open = .item($0)
            }
            // A capture opens at full height: the keyboard comes up
            // with it and would cover a half sheet. Nothing to capture
            // into before the session has loaded.
            .environment(\.captureItem) { list in
                guard let list = list ?? model.lists(inDoc: nil).first else { return }
                detent = .large
                open = .capture(CaptureTarget(list: list))
            }
            // One navigation stack for whatever has the sheet (the
            // page, a capture, a field editor): each puts its own
            // buttons in the bar. Nothing is pushed on it.
            .sheet(item: $open) { target in
                NavigationStack {
                    switch target {
                    case .item(let ref): ItemView(ref: ref, detent: $detent)
                    case .capture(let capture): ItemCaptureView(target: capture, detent: $detent)
                    }
                }
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
    /// An editor to open straight away: a capture that became this
    /// item on the way to its date or place.
    var startEditing: Editor? = nil
    @Environment(\.dismiss) private var dismiss
    /// The field editor that has the sheet, if one does.
    @State private var editing: Editor?
    @State private var detentBeforeEditing: PresentationDetent = .medium
    /// The "Delete" of the more menu is waiting on its confirmation.
    @State private var confirmingDelete = false

    enum Editor {
        case title
        case list
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

    /// The item's workspace when it is a shared one, for the list field
    /// and the list editor's rows.
    private var workspace: String? { ref.docId.flatMap(model.sharedWorkspaceName) }

    var body: some View {
        ZStack {
            if let editing, let item {
                Group {
                    switch editing {
                    case .title: ItemTitleEditor(ref: ref, item: item, done: endEditing)
                    case .list:
                        ItemListEditor(
                            itemTitle: item.text, lists: listOptions(item), current: item.listId,
                            workspace: workspace,
                            pick: { if $0.listId != item.listId { model.moveItem(ref, toList: $0.listId) } },
                            done: endEditing)
                    case .date: ItemDateEditor(ref: ref, item: item, done: endEditing)
                    case .place: ItemPlaceEditor(ref: ref, item: item, done: endEditing)
                    }
                }
                .transition(.opacity)
            } else {
                page.transition(.opacity)
            }
        }
        .onAppear {
            model.refreshItem(ref)
            if let startEditing, editing == nil { edit(startEditing) }
        }
        .onChange(of: model.changeCount) { model.refreshItem(ref) }
    }

    private var page: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 20) {
                Text(item?.text ?? ref.title)
                    .font(.title2.weight(.semibold))
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
        }
        // No pull-to-refresh: the page refetches on every doc change by
        // itself, and in a sheet a pull at the top has to mean "drag the
        // sheet down". Content that fits does not scroll at all, so the
        // same drag works from anywhere on it.
        .scrollBounceBehavior(.basedOnSize)
        .navigationBarTitleDisplayMode(.inline)
        .toolbar { bar }
        // A hard delete cannot be taken back (no undo here, and the
        // bin was the second chance), so it asks first.
        .confirmationDialog("Delete this item for good?", isPresented: $confirmingDelete, titleVisibility: .visible) {
            Button("Delete", role: .destructive) {
                model.deleteBinned(ref)
                dismiss()
            }
        } message: {
            Text("It is removed from every device and cannot be restored.")
        }
    }

    /// The page's navigation bar. At the left, the close button in the
    /// corner on its own glass (where the editors have their cross),
    /// then the checkbox and the workflow state beside it as one glass
    /// control with two halves:
    /// the checkbox ticks and reopens, the label opens a menu of the
    /// states (the web dialog's status badge). A binned item just says
    /// so (the bin masks the state; Restore is the way out), and one
    /// that has not loaded shows an inert box. At the right, the star
    /// and the more menu.
    /// The gaps inside the status control, set to look the same on a
    /// device: checkbox to label, and label to the menu's chevrons. The
    /// first is the smaller number because the bar puts a few points
    /// of its own around the menu.
    private static let checkGap: CGFloat = 3
    private static let chevronGap: CGFloat = 7

    @ToolbarContentBuilder
    private var bar: some ToolbarContent {
        SheetCloseItem { dismiss() }
        ToolbarSpacer(.fixed, placement: .topBarLeading)
        if let item, item.binnedAt == nil {
            ToolbarItem(placement: .topBarLeading) {
                HStack(spacing: Self.checkGap) {
                    ItemCheckButton(ref: ref, state: item.state)
                    stateMenu(item)
                }
                .padding(.horizontal, 6)
                .fixedSize()
            }
        } else {
            SheetStatusLabel(state: item?.state ?? "backlog", text: item == nil ? "" : "Binned")
        }
        // Only an Open item can be starred (spec/focus.md): a closed
        // or binned one has no star to show or set.
        // With a star, the two are one item laid out here: as two
        // items the bar spaces them itself, further apart than wanted.
        if let item, item.binnedAt == nil, item.closedAt == nil {
            ToolbarItem(placement: .topBarTrailing) {
                HStack(spacing: 0) {
                    StarButton(starred: item.starred, width: Self.pairedButtonWidth) {
                        model.setStarred(ref, starred: !item.starred)
                    }
                    moreMenu(item, width: Self.pairedButtonWidth)
                }
                .fixedSize()
            }
        } else if let item {
            ToolbarItem(placement: .topBarTrailing) { moreMenu(item) }
        }
    }

    /// The width each of the star and the more menu takes when they
    /// share the glass: the distance between their centres, and the
    /// width of each one's tap target.
    private static let pairedButtonWidth: CGFloat = 34

    /// The "…" menu: what is done to the item as a whole. An item goes
    /// to the bin and the sheet closes on it, as it leaves the list
    /// underneath. A binned one (reached from search) can be restored,
    /// which keeps the sheet up on the item as it was, or deleted for
    /// good after a confirmation.
    /// `width` is set when it shares the glass with the star: the
    /// label is then that wide, all of it the tap target.
    private func moreMenu(_ item: ItemDetail, width: CGFloat? = nil) -> some View {
        Menu {
            if item.binnedAt == nil {
                Button("Move to Bin", systemImage: "trash", role: .destructive) {
                    model.setBinned(ref, binned: true)
                    dismiss()
                }
            } else {
                Button("Restore", systemImage: "arrow.uturn.backward") {
                    model.setBinned(ref, binned: false)
                }
                Button("Delete", systemImage: "trash", role: .destructive) {
                    confirmingDelete = true
                }
            }
        } label: {
            if let width {
                Image(systemName: "ellipsis")
                    .frame(width: width, height: 36)
                    .contentShape(Rectangle())
                    .accessibilityLabel("More")
            } else {
                Label("More", systemImage: "ellipsis")
            }
        }
    }

    /// List, planned date, deadline (only when set, as on the web) and
    /// place. The date and place rows are always there, muted when
    /// empty: they are where setting one will happen. A pending date
    /// suggestion stands in the date row's place until it is answered.
    private func fields(_ item: ItemDetail) -> some View {
        VStack(alignment: .leading, spacing: 14) {
            listField(item)
            ItemDateField(
                item: item,
                edit: { edit(.date) },
                answer: { suggested, yes in
                    if yes {
                        model.setWhen(ref, when: suggested)
                    } else {
                        model.setWhenSuggested(ref, when: nil)
                    }
                })
            if let deadline = item.deadline {
                FieldRow("timer", text: "Due \(Self.dayText(deadline))")
            }
            Divider()
            // As the date: a row with the round button that opens the
            // editor (the whole row does while there is no place); the
            // map card stays under it.
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
            .addsOnTap(item.place == nil && item.binnedAt == nil) { edit(.place) }
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
            HStack(spacing: Self.chevronGap) {
                Text(Self.statusLabel(item.state, event: event))
                    .font(.subheadline.weight(.medium))
                Image(systemName: "chevron.up.chevron.down")
                    .font(.caption2)
                    .foregroundStyle(.secondary)
            }
            // The bar's full height, for the tap.
            .frame(minHeight: 36)
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .sensoryFeedback(.selection, trigger: item.state)
        .accessibilityLabel("Status")
        .accessibilityValue(Self.statusLabel(item.state, event: event))
    }

    /// The list editor's rows: the workspace's lists, plus the item's
    /// own list when it is not among them (archived, or the calendar's),
    /// so there is always a row to tick.
    private func listOptions(_ item: ItemDetail) -> [ListRef] {
        let lists = model.lists(inDoc: ref.docId)
        if lists.contains(where: { $0.listId == item.listId }) { return lists }
        let own = ListRef(docId: ref.docId ?? "", listId: item.listId, name: item.listLabel, icon: item.listIcon)
        return lists + [own]
    }

    /// The item's list, drawn as a field: full width, a faint fill, a
    /// full-height (44pt) tap target, outset by its own padding so its
    /// icon and text stay in line with the plain rows under it. A tap
    /// hands the sheet to the list editor (`ItemListEditor`), as the
    /// date and place rows do with theirs; a system menu here covered
    /// the item it was moving. Lists of other workspaces are not
    /// offered: the engine has no cross-doc move yet. A binned item
    /// stays put.
    private func listField(_ item: ItemDetail) -> some View {
        Button {
            edit(.list)
        } label: {
            HStack(spacing: 6) {
                FieldRow(text: item.listLabel) { ListIcon(listId: item.listId, emoji: item.listIcon) }
                Spacer(minLength: 8)
                SharedWorkspaceBadge(name: workspace)
                Image(systemName: "chevron.up.chevron.down")
                    .font(.footnote)
                    .foregroundStyle(.secondary)
            }
            .itemField()
        }
        .buttonStyle(.plain)
        .disabled(item.binnedAt != nil)
        .itemFieldOutset()
        .accessibilityHint("Moves the item to another list")
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

    /// A planned date for the item's row: today and tomorrow by name
    /// ("Today at 14:00") with the date itself to follow, muted
    /// ("Sat 14 Nov"); any other day as `whenText` has it, alone.
    static func whenParts(_ when: String, now: Date = Date(), calendar: Calendar = .current) -> (text: String, muted: String?) {
        guard let date = Agenda.start(of: String(when.prefix(10)), calendar: calendar),
            let tomorrow = calendar.date(byAdding: .day, value: 1, to: now)
        else { return (whenText(when), nil) }
        let name: String
        if calendar.isDate(date, inSameDayAs: now) {
            name = "Today"
        } else if calendar.isDate(date, inSameDayAs: tomorrow) {
            name = "Tomorrow"
        } else {
            return (whenText(when), nil)
        }
        let text = when.count >= 16 ? "\(name) at \(when.dropFirst(11).prefix(5))" : name
        return (text, date.formatted(.dateTime.weekday(.abbreviated).day().month(.abbreviated)))
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
/// what was typed. An empty title cannot be kept. A date phrase the new
/// title brings is written with it as the item's date suggestion, and
/// one it loses takes its suggestion along (`WhenSuggest.edit`).
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
        if trimmed != item.text {
            let suggestion = WhenSuggest.edit(
                title: trimmed, was: item.text, when: item.when, pending: item.whenSuggested,
                closed: item.closedAt != nil)
            model.setText(ref, text: trimmed, suggestion: suggestion)
        }
        done()
    }

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 20) {
                // Wraps like the title it replaces. A wrapping field
                // takes Return as a line break, so Return is caught as
                // it lands and commits instead.
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
        .editorHeader(title: "Title", canFinish: !trimmed.isEmpty, cancel: done, done: commit)
        .onAppear { focused = true }
    }
}

/// The star at the top right of an item's page, beside the more menu:
/// the web dialog's glyph (`.task-dialog-pin-toggle`,
/// `js/web/src/icons/star-filled.svg`, in the asset catalog as
/// `StarFill` and, stroked, `StarStroke`) on the bar's glass, a thin
/// outline that fills yellow when the item is starred; a tap flips it.
private struct StarButton: View {
    let starred: Bool
    /// The width of the button, all of it the tap target.
    let width: CGFloat
    let action: () -> Void

    /// The web's `#ffe785`.
    static let fill = Color(red: 1.0, green: 0.906, blue: 0.522)

    var body: some View {
        Button(action: action) {
            ZStack {
                Image("StarFill")
                    .resizable()
                    .foregroundStyle(starred ? Self.fill : .clear)
                Image("StarStroke")
                    .resizable()
                    .foregroundStyle(Color.primary.opacity(0.7))
            }
            .frame(width: 22, height: 22)
            .frame(width: width, height: 36)
            .contentShape(Rectangle())
        }
        .sensoryFeedback(.selection, trigger: starred)
        .accessibilityLabel(starred ? "Unstar" : "Star")
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
    /// More of the same line, after the text and in its font, muted.
    var muted: String? = nil
    var note: String? = nil
    var empty = false
    @ViewBuilder let icon: Icon

    var body: some View {
        HStack(alignment: .firstTextBaseline, spacing: 12) {
            icon
                .foregroundStyle(empty ? .tertiary : .secondary)
                .frame(width: 24)
            VStack(alignment: .leading, spacing: 2) {
                if let muted {
                    Text("\(text) \(Text(muted).foregroundStyle(.secondary))")
                } else {
                    Text(text).foregroundStyle(empty ? .tertiary : .primary)
                }
                if let note {
                    Text(note).font(.footnote).foregroundStyle(.secondary)
                }
            }
        }
    }
}

extension FieldRow where Icon == Image {
    /// A field led by an SF Symbol.
    init(_ systemImage: String, text: String, muted: String? = nil, note: String? = nil, empty: Bool = false) {
        self.init(text: text, muted: muted, note: note, empty: empty) { Image(systemName: systemImage) }
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
