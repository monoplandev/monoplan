// An item's page: the web client's task dialog (js/web/src/TaskDialog.tsx)
// laid out top to bottom in the same order: status, title, list, planned
// date, deadline, place, notes, then the activity log. Read from
// `Engine.item` and reread on every doc change while shown.
// It comes up as a page sheet over whatever list the row was in (full
// height only), as the dialog floats over the web's lists, rather
// than as a pushed page. The sheet has a navigation bar, its buttons the
// system's Liquid Glass: at the left the status control (the checkbox
// ticks and reopens the item, the state label beside it opens a menu of
// the workflow states) after the "x", which closes the sheet (as a drag
// down does); at the right the star, which stars and unstars it, and
// the more menu. A tap
// on the title hands the sheet to the title editor (`ItemTitleEditor`),
// the list and assignee fields and the date and place rows hand the
// sheet to their editors (`ItemListEditor`, `ItemAssigneeEditor`,
// `ItemDateEditor`, `ItemPlaceEditor`);
// everything else is display only.
// The same sheet is where a new item is captured (`ItemCaptureView`,
// opened by the bottom bar's add button): a title field, and this page
// from the moment the title makes it a real item.

import MapKit
import MonoplanCore
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

    func body(content: Content) -> some View {
        content
            .environment(\.openItem) { open = .item($0) }
            // Nothing to capture into before the session has loaded.
            .environment(\.captureItem) { list in
                guard let list = list ?? model.lists(inDoc: nil).first else { return }
                open = .capture(CaptureTarget(list: list))
            }
            // One navigation stack for whatever has the sheet (the
            // page, a capture, a field editor): each puts its own
            // buttons in the bar. Nothing is pushed on it. A page
            // sheet: full height only, no half detent.
            .sheet(item: $open) { target in
                NavigationStack {
                    switch target {
                    case .item(let ref): ItemView(ref: ref)
                    case .capture(let capture): ItemCaptureView(target: capture)
                    }
                }
                .environmentObject(model)
            }
            // A tapped reminder opens its item, over whatever is up.
            .onChange(of: model.tappedReminder, initial: true) {
                guard let ref = model.tappedReminder else { return }
                open = .item(ref)
                model.showedReminder()
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
    /// The item as the sheet was opened on it.
    private let opened: ItemRef
    /// Where it lives now, once it has been moved to another
    /// workspace from this page: the page follows it, the sheet stays.
    @State private var moved: ItemRef?
    private var ref: ItemRef { moved ?? opened }
    /// An editor to open straight away: a capture that became this
    /// item on the way to its date or place.
    private let startEditing: Editor?
    /// A list of another workspace was picked and the move changes who
    /// sees the item: waiting on its confirmation.
    @State private var crossing: Crossing?

    private struct Crossing {
        let list: ListRef
        let workspace: String
        let note: String
    }

    init(ref: ItemRef, startEditing: Editor? = nil) {
        opened = ref
        self.startEditing = startEditing
    }
    @Environment(\.dismiss) private var dismiss
    /// The field editor that has the sheet, if one does.
    @State private var editing: Editor?
    /// The "Delete" of the more menu is waiting on its confirmation.
    @State private var confirmingDelete = false
    /// As is its "Move to Bin".
    @State private var confirmingBin = false

    enum Editor {
        case title
        case list
        case date
        case place
        case assignee
    }

    /// An editor takes the sheet over; closing it gives the sheet back.
    private func edit(_ editor: Editor) {
        withAnimation(.easeOut(duration: 0.15)) { editing = editor }
    }

    private func endEditing() {
        withAnimation(.easeOut(duration: 0.15)) { editing = nil }
    }

    private var item: ItemInfo? { model.itemDetails[ref.key] }

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
                            others: otherListOptions(item),
                            pick: { if $0.listId != item.listId { model.moveItem(ref, toList: $0.listId) } },
                            pickOther: moveAcross,
                            done: endEditing)
                    case .date: ItemDateEditor(ref: ref, item: item, done: endEditing)
                    case .place: ItemPlaceEditor(ref: ref, item: item, done: endEditing)
                    case .assignee:
                        ItemAssigneeEditor(
                            itemTitle: item.text, members: ref.docId.flatMap { model.members[$0] },
                            current: item.assignee,
                            pick: { model.setAssignee(ref, assignee: $0) },
                            done: endEditing)
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
        .alert(
            "Move to \(crossing?.workspace ?? "")?",
            isPresented: Binding(get: { crossing != nil }, set: { if !$0 { crossing = nil } }),
            presenting: crossing
        ) { crossing in
            Button("Move") { model.moveItem(ref, across: crossing.list) { moved = $0 } }
            Button("Cancel", role: .cancel) {}
        } message: { crossing in
            Text(crossing.note)
        }
    }

    /// A list of another workspace was picked: the item moves there
    /// and the page follows it. Asked first only when the move changes
    /// who sees the item (spec/sharing-plan.md "Cross-doc move").
    private func moveAcross(_ list: ListRef) {
        if let note = model.moveChangesAudience(from: ref.docId, to: list.docId) {
            crossing = Crossing(list: list, workspace: model.workspaceLabel(list.docId), note: note)
        } else {
            model.moveItem(ref, across: list) { moved = $0 }
        }
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
                    // Between the place and the notes, as on the web
                    // and the Mac. A binned item's are shown, not
                    // edited, and it has no row to add one.
                    if item.binnedAt == nil || !item.subtasks.isEmpty {
                        ItemSubtasksField(
                            subtasks: item.subtasks, locked: item.binnedAt != nil,
                            add: { model.addSubtask(ref, text: $0) },
                            setText: { model.setSubtaskText(ref, subtask: $0, text: $1) },
                            setDone: { model.setSubtaskDone(ref, subtask: $0, done: $1) },
                            move: { model.moveSubtask(ref, subtask: $0, index: $1) },
                            remove: { model.removeSubtask(ref, subtask: $0) }
                        )
                        .id(ref.key)
                        // A row is a 44 pt target around a line of
                        // text: the slack over the first and under the
                        // last is given back, so they sit as far from
                        // the dividers as the date and place rows do.
                        .padding(.vertical, -10)
                        Divider()
                    }
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
        // The subtasks are typed in on the page itself.
        .scrollDismissesKeyboard(.interactively)
        .navigationBarTitleDisplayMode(.inline)
        .toolbar { bar }
        .confirmationDialog("Move this item to the Bin?", isPresented: $confirmingBin, titleVisibility: .visible) {
            Button("Move to Bin", role: .destructive) {
                model.setBinned(ref, binned: true)
                dismiss()
            }
        } message: {
            Text("It can be restored from the Bin.")
        }
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
    /// then the status control (`SheetStatusControl`): the checkbox
    /// ticks and reopens, the label opens a menu of the states. A
    /// binned item just says so (the bin masks the state; Restore is
    /// the way out), and one that has not loaded shows an inert box.
    /// At the right, the star and the more menu.
    @ToolbarContentBuilder
    private var bar: some ToolbarContent {
        SheetCloseItem { dismiss() }
        ToolbarSpacer(.fixed, placement: .topBarLeading)
        if let item, item.binnedAt == nil {
            SheetStatusControl {
                ItemCheckButton(ref: ref, state: item.state)
            } menu: {
                stateMenu(item)
            }
        } else {
            SheetStatusLabel(state: item?.state ?? .backlog, text: item == nil ? "" : String(localized: "Binned"))
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
    /// good after a confirmation. "Show in List" closes the sheet on
    /// the item's list page, scrolled to its row (`parentList`).
    /// `width` is set when it shares the glass with the star: the
    /// label is then that wide, all of it the tap target.
    private func moreMenu(_ item: ItemInfo, width: CGFloat? = nil) -> some View {
        Menu {
            if let list = parentList(item) {
                Button("Show in List", systemImage: "list.bullet") {
                    model.reveal(ref.itemId, in: list)
                    dismiss()
                }
            }
            if item.binnedAt == nil {
                Button("Move to Bin", systemImage: "trash", role: .destructive) {
                    confirmingBin = true
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

    /// The list page the item is a row of, for "Show in List": its own
    /// list, or the Inbox when its list row is gone (the catch-all,
    /// spec/inbox-plan.md). None for what no list page shows: a closed
    /// or binned item, or a dated one with no list (the Calendar's).
    private func parentList(_ item: ItemInfo) -> ListRef? {
        guard item.binnedAt == nil, item.closedAt == nil else { return nil }
        let lists = model.lists(inDoc: ref.docId)
        guard let list = lists.first(where: { $0.listId == item.listId }) ?? lists.first else { return nil }
        if list.listId == "inbox" && item.when != nil { return nil }
        return list
    }

    /// List, planned date, deadline (only when set, as on the web) and
    /// place. The date and place rows are always there, muted when
    /// empty: they are where setting one will happen. A pending date
    /// suggestion stands in the date row's place until it is answered.
    private func fields(_ item: ItemInfo) -> some View {
        VStack(alignment: .leading, spacing: 14) {
            listField(item)
            assigneeField(item)
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
                FieldRow("timer", text: String(localized: "Due \(Self.dayText(deadline))"))
            }
            Divider()
            // As the date: a row with the round button that opens the
            // editor (the whole row does while there is no place); the
            // map card stays under it.
            HStack(spacing: 8) {
                if let place = item.place {
                    FieldRow(text: place.label, note: place.address) { PinIcon() }
                } else {
                    FieldRow(text: String(localized: "Place"), empty: true) { PinIcon() }
                }
                if item.place != nil {
                    Spacer(minLength: 8)
                    EditBubble(what: String(localized: "place")) { edit(.place) }
                        .disabled(item.binnedAt != nil)
                }
            }
            .frame(maxWidth: .infinity, minHeight: 36, alignment: .leading)
            .addsOnTap(item.place == nil && item.binnedAt == nil, what: String(localized: "place")) { edit(.place) }
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
    private static let taskStates: [ItemState] = [.backlog, .todo, .inProgress, .review, .done, .cancelled]
    private static let eventStates: [ItemState] = [.backlog, .done, .cancelled]

    private static func statusLabel(_ state: ItemState, event: Bool) -> String {
        event && !state.isClosed ? String(localized: "Event") : state.label
    }

    /// The state label as a pull-down menu: the current state ticked, a
    /// pick moves the item there in one write. On an Open event the
    /// single open choice is ticked whatever open state the item holds
    /// underneath, and picking it changes nothing, so a state the item
    /// brought in from a list is kept.
    private func stateMenu(_ item: ItemInfo) -> some View {
        let event = item.event
        let picked: ItemState = event && !item.state.isClosed ? .backlog : item.state
        return ItemStateMenu(
            states: event ? Self.eventStates : Self.taskStates,
            picked: picked,
            label: Self.statusLabel(item.state, event: event),
            name: { Self.statusLabel($0, event: event) },
            pick: { model.setState(ref, state: $0) })
    }

    /// The list editor's rows: the workspace's lists, plus the item's
    /// own list when it is not among them (archived, or the calendar's),
    /// so there is always a row to tick.
    private func listOptions(_ item: ItemInfo) -> [ListRef] {
        // For an item with a date, "no list" is the Calendar, not Inbox.
        let lists = model.lists(inDoc: ref.docId).map { Self.noList($0, dated: item.when != nil) }
        if lists.contains(where: { $0.listId == item.listId }) { return lists }
        let own = ListRef(docId: ref.docId ?? "", listId: item.listId, name: item.listLabel, icon: item.listIcon)
        return lists + [own]
    }

    /// The other workspaces' rows, "no list" named the same way.
    private func otherListOptions(_ item: ItemInfo) -> [ListGroup] {
        model.otherLists(thanDoc: ref.docId).map { group in
            ListGroup(
                docId: group.docId, name: group.name, shared: group.shared,
                lists: group.lists.map { Self.noList($0, dated: item.when != nil) })
        }
    }

    private static func noList(_ list: ListRef, dated: Bool) -> ListRef {
        guard dated, list.listId == "inbox" else { return list }
        return ListRef(docId: list.docId, listId: list.listId, name: NoList.label(dated: true), calendar: true)
    }

    /// The item's list, drawn as a field: full width, a faint fill, a
    /// full-height (44pt) tap target, outset by its own padding so its
    /// icon and text stay in line with the plain rows under it. A tap
    /// hands the sheet to the list editor (`ItemListEditor`), as the
    /// date and place rows do with theirs; a system menu here covered
    /// the item it was moving. The editor offers every other
    /// workspace's lists under the item's own. A binned item stays
    /// put.
    private func listField(_ item: ItemInfo) -> some View {
        Button {
            edit(.list)
        } label: {
            HStack(spacing: 6) {
                FieldRow(text: item.listLabel) {
                    ListIcon(listId: item.listId, emoji: item.listIcon, calendar: item.when != nil)
                }
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

    /// Who the item is assigned to, on a shared workspace
    /// (spec/sharing-plan.md "Assignment"), with a divider under it. A
    /// plain row, no box until it is pressed; a tap hands the sheet to
    /// the assignee editor (`ItemAssigneeEditor`), as the list field
    /// does with its own. Not shown on the Personal workspace, nor on a
    /// workspace of one unless the item is already assigned (so that
    /// can be cleared). A binned item stays as it is.
    @ViewBuilder
    private func assigneeField(_ item: ItemInfo) -> some View {
        if let docId = ref.docId, workspace != nil, model.hasOtherMembers(docId) || item.assignee != nil {
            let name = item.assignee.map { ItemAssigneeEditor.name(of: $0, in: model.members[docId]) }
            Button {
                edit(.assignee)
            } label: {
                // The assignee's avatar in the icon's place once there
                // is one (a former member's is the dashed "?").
                FieldRow(text: name ?? String(localized: "Assign"), empty: name == nil) {
                    if let id = item.assignee {
                        MemberAvatar(
                            accountId: id,
                            email: model.members[docId]?.first(where: { $0.accountId == id })?.email,
                            side: 22)
                    } else {
                        Image(systemName: "person")
                    }
                }
            }
            .buttonStyle(BareFieldStyle())
            .disabled(item.binnedAt != nil)
            .itemFieldOutset()
            .accessibilityHint("Assigns the item to a member")
            Divider()
        }
    }

    @ViewBuilder
    private func notes(_ item: ItemInfo) -> some View {
        if item.notes.isEmpty {
            Text("Notes").foregroundStyle(.tertiary)
        } else {
            Text(item.notes).textSelection(.enabled)
        }
    }

    /// The item's timeline as label and value rows, the label at the
    /// leading edge and the value at the trailing: who created it (on
    /// a shared workspace), when, and once closed, when and how long
    /// it took.
    private func activity(_ item: ItemInfo) -> some View {
        VStack(alignment: .leading, spacing: 4) {
            // Who added it, on a shared workspace: their email beside
            // their avatar, the avatar alone while the members are not
            // known.
            if let docId = ref.docId, workspace != nil, let creator = item.item.createdBy {
                let members = model.members[docId]
                let member = members?.first(where: { $0.accountId == creator })
                activityRow(String(localized: "Created by")) {
                    if members != nil {
                        Text(verbatim: member?.email ?? String(localized: "A former member"))
                            .lineLimit(1)
                            .truncationMode(.middle)
                    }
                    MemberAvatar(accountId: creator, email: member?.email, side: 18)
                }
            }
            activityRow(String(localized: "Created"), Self.stamp(item.createdAt))
            if let closedAt = item.closedAt {
                activityRow(
                    item.state == .done ? String(localized: "Completed") : String(localized: "Cancelled"),
                    Self.stamp(closedAt))
                if let took = Self.elapsed(from: item.createdAt, to: closedAt) {
                    activityRow(String(localized: "Elapsed"), took)
                }
            }
        }
        .foregroundStyle(.secondary)
    }

    private func activityRow(_ label: String, _ value: String) -> some View {
        activityRow(label) {
            Text(verbatim: value)
                .multilineTextAlignment(.trailing)
                .monospacedDigit()
        }
    }

    private func activityRow(_ label: String, @ViewBuilder value: () -> some View) -> some View {
        HStack(alignment: .center, spacing: 8) {
            Text(verbatim: label)
            Spacer(minLength: 0)
            value()
        }
        .accessibilityElement(children: .combine)
    }

    // MARK: Formatting

    /// A planned date: "Saturday 12 Sep" (with the year once it is not
    /// this one), and "at 14:00" after it when timed.
    static func whenText(_ when: String) -> String {
        let day = dayText(String(when.prefix(10)))
        guard when.count >= 16 else { return day }
        return String(localized: "\(day) at \(String(when.dropFirst(11).prefix(5)))")
    }

    /// A planned date for the item's row: today and tomorrow by name
    /// in the date's place ("Today 14:00"), any other day spelled out
    /// ("Saturday 12 Sep 14:00"). No "at" before the time.
    static func rowWhenText(_ when: String, now: Date = Date(), calendar: Calendar = .current) -> String {
        let day = String(when.prefix(10))
        var name = dayText(day, now: now, calendar: calendar)
        if let date = Agenda.start(of: day, calendar: calendar) {
            if calendar.isDate(date, inSameDayAs: now) {
                name = String(localized: "Today")
            } else if let tomorrow = calendar.date(byAdding: .day, value: 1, to: now),
                calendar.isDate(date, inSameDayAs: tomorrow)
            {
                name = String(localized: "Tomorrow")
            }
        }
        return when.count >= 16 ? "\(name) \(when.dropFirst(11).prefix(5))" : name
    }

    /// A planned date said short, for a row's note: "Today 14:00",
    /// "Tomorrow", the weekday alone within the week ahead ("Friday
    /// 14:00"), otherwise the day and month ("13 Feb"), with the year
    /// once it is not this one.
    static func shortWhenText(_ when: String, now: Date = Date(), calendar: Calendar = .current) -> String {
        let day = String(when.prefix(10))
        var name = day
        if let date = Agenda.start(of: day, calendar: calendar) {
            let today = calendar.startOfDay(for: now)
            let ahead = calendar.dateComponents([.day], from: today, to: calendar.startOfDay(for: date)).day ?? 0
            switch ahead {
            case 0: name = String(localized: "Today")
            case 1: name = String(localized: "Tomorrow")
            case 2...6: name = date.formatted(.dateTime.weekday(.wide))
            default:
                name = calendar.isDate(date, equalTo: now, toGranularity: .year)
                    ? date.formatted(.dateTime.day().month(.abbreviated))
                    : date.formatted(.dateTime.day().month(.abbreviated).year())
            }
        }
        return when.count >= 16 ? "\(name) \(when.dropFirst(11).prefix(5))" : name
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
        guard let minutes = item.duration, minutes > 0 else { return nil }
        if item.isTimed { return AgendaRowView.span(minutes) }
        let days = (minutes + 1439) / 1440
        return days >= 2 ? String(localized: "\(days) days") : nil
    }

    /// An instant for the activity log: "Today 2:05 PM" today, otherwise
    /// the locale's day and abbreviated month, "9 Nov 2:05 PM", the year
    /// only when it is not this one. The rows show it in monospaced
    /// digits, end-aligned, so they compare at a glance one under the
    /// other.
    static func stamp(_ millis: Int64, now: Date = Date(), calendar: Calendar = .current) -> String {
        let date = Date(timeIntervalSince1970: Double(millis) / 1000)
        let time = date.formatted(date: .omitted, time: .shortened)
        if calendar.isDate(date, inSameDayAs: now) {
            return String(localized: "Today \(time)")
        }
        let day = calendar.isDate(date, equalTo: now, toGranularity: .year)
            ? date.formatted(.dateTime.day().month(.abbreviated))
            : date.formatted(.dateTime.day().month(.abbreviated).year())
        return day + " " + time
    }

    /// "1d 3h": two units at most, the larger first, the smaller only
    /// when it is not zero, each a single letter (y, d, h, m, s; no
    /// weeks or months, whose letters would be taken for something
    /// else). The same in every language.
    static func elapsed(from start: Int64, to end: Int64) -> String? {
        let seconds = max(0, end - start) / 1000
        let units: [(Int64, String)] = [(365 * 86400, "y"), (86400, "d"), (3600, "h"), (60, "m"), (1, "s")]
        guard let first = units.firstIndex(where: { seconds >= $0.0 }) else { return "0s" }
        let whole = seconds / units[first].0
        var text = "\(whole)\(units[first].1)"
        if first + 1 < units.count {
            let rest = (seconds - whole * units[first].0) / units[first + 1].0
            if rest > 0 { text += " \(rest)\(units[first + 1].1)" }
        }
        return text
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
    let item: ItemInfo
    let done: () -> Void
    @State private var text: String
    @FocusState private var focused: Bool

    init(ref: ItemRef, item: ItemInfo, done: @escaping () -> Void) {
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
            model.setText(ref, text: trimmed, suggestion: SuggestionChange(suggestion))
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
        .editorHeader(title: String(localized: "Title"), canFinish: !trimmed.isEmpty, cancel: done, done: commit)
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

    var body: some View {
        Button(action: action) {
            ZStack {
                Image("StarFill")
                    .resizable()
                    .foregroundStyle(starred ? StarMark.fill : .clear)
                Image("StarStroke")
                    .resizable()
                    .foregroundStyle(Color.primary.opacity(0.7))
            }
            .frame(width: 22, height: 22)
            .frame(width: width, height: 36)
            .contentShape(Rectangle())
        }
        .sensoryFeedback(.selection, trigger: starred)
        .accessibilityLabel(starred ? String(localized: "Unstar") : String(localized: "Star"))
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

/// A field with no box of its own: the same full-height target as
/// `itemField()`, its faint fill showing only while it is pressed.
struct BareFieldStyle: ButtonStyle {
    func makeBody(configuration: Configuration) -> some View {
        let box = RoundedRectangle(cornerRadius: 10, style: .continuous)
        configuration.label
            .padding(.horizontal, itemFieldInset)
            .frame(maxWidth: .infinity, minHeight: 44, alignment: .leading)
            .background(.quaternary.opacity(configuration.isPressed ? 0.5 : 0), in: box)
            .contentShape(box)
    }
}
