// The item panel: the selected row's item, at the right of the window.
// The iOS item page (apple/MonoplanIOS/Monoplan/ItemView.swift) laid
// out the same way, top to bottom: status, title, list, planned date,
// deadline, place, subtasks (`SubtasksField.swift`),
// notes, then the activity log. Read from
// `Engine.item` in the window's snapshot, so it is reread on every doc
// change like the rest.
//
// Read only but for the two writes the window already had: the
// checkbox ticks and reopens the item, the star stars and unstars it.
// Both are toolbar items with the close button, over the panel in the
// window's bar and on the system's glass, as the iOS page's navigation
// bar (`MainSplit.swift`).
// The list row moves the item (`ListField.swift`); the planned date
// (`DateField.swift`), its times (`TimeField.swift`) and the place
// (`PlaceField.swift`) are edited in place. No other
// field editors, no state menu, no bin: those come with the views
// shared with iOS.

import MapKit
import MonoplanCore
import MonoplanMacKit
import SwiftUI

struct ItemPanel: View {
    /// Nil until the read for a new selection lands.
    let item: ItemInfo?
    /// What tells this page's views from another's (their state is
    /// kept while it holds): the item's id, or a capture's own, which
    /// the item it becomes keeps.
    let identity: String
    /// A capture (`ItemCapture`): the item is not made yet. Its title,
    /// list, date and place are what it will be made with; it has no
    /// subtasks, notes or history to show.
    var isNew = false
    /// The title takes the caret as the page appears: a new item
    /// begun from the toolbar, with nothing typed yet.
    var focusTitle = false
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
    /// The item was marked an event, or a task again.
    let setEvent: (ItemInfo, Bool) -> Void
    /// Who the item can be assigned to; nil on Home, which has no
    /// assignee row.
    let assignment: PanelAssignment?
    /// The item was assigned to a member (their account id), or
    /// unassigned (nil).
    let setAssignee: (ItemInfo, String?) -> Void
    /// The item's length was set, in minutes, or taken off (nil).
    let setDuration: (ItemInfo, Int?) -> Void
    /// The item's subtasks edited (spec/subtasks.md): one added, then
    /// by its id one retitled, ticked or unticked, moved to an index,
    /// removed.
    let addSubtask: (ItemInfo, String) -> Void
    let setSubtaskText: (ItemInfo, String, String) -> Void
    let setSubtaskDone: (ItemInfo, String, Bool) -> Void
    let moveSubtask: (ItemInfo, String, Int) -> Void
    let removeSubtask: (ItemInfo, String) -> Void

    var body: some View {
        Group {
            if let item {
                ScrollView {
                    VStack(alignment: .leading, spacing: 16) {
                        // The bin masks an item: nothing of it is
                        // edited until it is restored.
                        if item.item.binnedAt == nil {
                            PanelTitle(text: item.item.text, closed: item.closedAt != nil, autofocus: focusTitle) {
                                retitle(item, $0)
                            }
                                .id(identity)
                        } else {
                            Text(verbatim: item.item.text)
                                .font(.title2.weight(.medium))
                                .foregroundStyle(.secondary)
                                .textSelection(.enabled)
                        }
                        fields(item)
                            // The list, date and time lists float over the rows below.
                            .zIndex(1)
                        if !isNew { PanelRule() }
                        // Between the place and the notes, as on the
                        // web. A binned item's are shown, not edited,
                        // and it has no row to add one.
                        if !isNew, item.item.binnedAt == nil || !item.subtasks.isEmpty {
                            SubtasksField(
                                subtasks: item.subtasks, locked: item.item.binnedAt != nil,
                                add: { addSubtask(item, $0) },
                                setText: { setSubtaskText(item, $0, $1) },
                                setDone: { setSubtaskDone(item, $0, $1) },
                                move: { moveSubtask(item, $0, $1) },
                                remove: { removeSubtask(item, $0) }
                            )
                            .id(identity)
                            // A row is taller than its text by 6 pt
                            // each side: given back, so the first and
                            // last sit 16 pt from the rules, as the
                            // date and place rows do from theirs.
                            .padding(.vertical, -(SubtasksField.rowHeight - 16) / 2)
                            PanelRule()
                        }
                        if !isNew {
                            notes(item)
                            PanelRule()
                            activity(item)
                        }
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
            .id(identity)
            // Its list floats over every row under it, the date and
            // place fields' included.
            .zIndex(3)
            if let assignment, assignment.others || item.item.assignee != nil {
                AssigneeField(
                    assignee: item.item.assignee, assignment: assignment,
                    locked: item.item.binnedAt != nil
                ) { setAssignee(item, $0) }
            }
            // The list, the date rows and the place are set apart: a
            // rule between each, with 16 pt between it and a row's
            // text (the stack's 4, 4 more, and the 8 inside a box).
            PanelRule().padding(.vertical, 4)
            if item.item.binnedAt == nil {
                DateField(when: item.item.when, duration: item.item.duration.map(Int.init)) {
                    setWhen(item, $0, $1)
                }
                .id(identity)
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
            // The times, on a row of their own under the day, in two
            // columns: when it starts ("All day" while it has no
            // time), then, once it has a start, when it ends (a muted
            // "End" while it has no length).
            if let when = item.item.when, item.item.binnedAt == nil {
                let start = TimeOffers.minutes(of: when)
                // Each box is outset by its inset, so 4 pt shows
                // between the two.
                HStack(spacing: PanelBox.inset * 2 + 4) {
                    TimeField(
                        value: start, placeholder: String(localized: "All day"),
                        label: String(localized: "Start time"), systemImage: "clock",
                        clearLabel: String(localized: "Clear time")
                    ) {
                        setTime(item, $0)
                    }
                    if let start {
                        let length = max(0, item.item.duration.map(Int.init) ?? 0)
                        // A retyped end stays on the day it lands on.
                        let offset = (start + length) / TimeOffers.day
                        TimeField(
                            value: length > 0 ? (start + length) % TimeOffers.day : nil,
                            placeholder: String(localized: "End"),
                            label: String(localized: "End time"), systemImage: "arrow.right",
                            after: start,
                            hint: {
                                TodayMenu.span(
                                    TimeInterval(TimeOffers.duration(from: start, to: $0, dayOffset: offset) * 60))
                            }
                        ) {
                            guard let end = $0 else { return }
                            setDuration(item, TimeOffers.duration(from: start, to: end, dayOffset: offset))
                        }
                    }
                }
                .id(identity)
                // Their lists float over the rows under them.
                .zIndex(1.5)
            } else if let time = item.item.when.flatMap(Self.timeText) {
                HStack(spacing: PanelBox.inset * 2 + 4) {
                    PanelField(text: time) { Image(systemName: "clock") }
                    let end = Self.endTimeText(item)
                    PanelField(text: end ?? String(localized: "End"), empty: end == nil) {
                        Image(systemName: "arrow.right")
                    }
                }
            }
            // What the date means: something to do by then, or
            // something that happens then (spec/events-plan.md).
            if item.item.when != nil, item.item.binnedAt == nil {
                EventToggle(event: item.item.event) { setEvent(item, $0) }
            }
            if let deadline = item.item.deadline {
                PanelField(text: String(localized: "Due \(Self.dayText(deadline))")) { Image(systemName: "timer") }
            }
            PanelRule().padding(.vertical, 4)
            // The place row with no map or address under it ends the
            // block. Its box has 8 pt of nothing under its text, which
            // would put the text 24 pt over the rule after the block
            // against the 16 under the rule before it: the row gives
            // those 8 back.
            let bare = item.item.place?.lat == nil && (item.item.place?.address ?? "").isEmpty
            let trim: CGFloat = bare ? -PanelBox.inset : 0
            if item.item.binnedAt == nil {
                PlaceField(place: item.item.place, knownPlaces: knownPlaces, trim: trim) {
                    setPlace(item, $0, $1)
                }
                .id(identity)
            } else if let place = item.item.place {
                PanelField(text: place.label) { PanelGlyph(image: NavArt.pin) }
                    .padding(.bottom, trim)
            } else {
                PanelField(text: String(localized: "Place"), empty: true) { PanelGlyph(image: NavArt.pin) }
                    .padding(.bottom, trim)
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

    /// A start time picked for the item's day, or taken off (nil: the
    /// date is all-day again; the length is kept, so a time put back
    /// brings the end back with it). A time put on an all-day span of
    /// whole days would end a day late (Tue to Thu as three days is 72
    /// hours, so from Tue 09:00 it would end Fri 09:00): that length is
    /// reshaped to end on the same last day, an hour after the start.
    private func setTime(_ item: ItemInfo, _ minutes: Int?) {
        guard let when = item.item.when else { return }
        let day = String(when.prefix(10))
        guard let minutes else { return setWhen(item, day, .keep) }
        var change = DurationChange.keep
        if when.count < 16, let length = item.item.duration.map(Int.init),
           length >= 2 * TimeOffers.day, length % TimeOffers.day == 0 {
            change = .minutes(minutes: UInt32(length - TimeOffers.day + 60))
        }
        setWhen(item, "\(day)T\(TimeOffers.text(minutes))", change)
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

    /// The item's timeline as label and value rows, the label at the
    /// leading edge and the value at the trailing: who created it (on
    /// a shared workspace), when, and once closed, when and how long
    /// it took.
    private func activity(_ item: ItemInfo) -> some View {
        VStack(alignment: .leading, spacing: 4) {
            // Who added it, on a shared workspace (`assignment` is nil
            // on Home): their email beside their avatar, the avatar
            // alone while the members are not known.
            if let assignment, let creator = item.item.createdBy {
                let member = assignment.members?.first(where: { $0.accountId == creator })
                activityRow(String(localized: "Created by")) {
                    if assignment.members != nil {
                        Text(verbatim: member?.email ?? String(localized: "A former member"))
                            .lineLimit(1)
                            .truncationMode(.middle)
                    }
                    MemberAvatar(accountId: creator, email: member?.email, side: 16)
                }
            }
            activityRow(String(localized: "Created"), Self.stamp(item.item.createdAt))
            if let closedAt = item.closedAt {
                activityRow(
                    item.item.state == .done ? String(localized: "Completed") : String(localized: "Cancelled"),
                    Self.stamp(closedAt))
                if let took = Self.elapsed(from: item.item.createdAt, to: closedAt) {
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
                .textSelection(.enabled)
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

/// What the assignee row needs of the item's workspace: whether anyone
/// else is in it, and its members when the list is known (nil while it
/// is loading or the server cannot be reached).
struct PanelAssignment {
    let others: Bool
    let members: [WorkspaceMember]?
}

/// Who the item is assigned to, on a shared workspace
/// (spec/sharing-plan.md "Assignment"): the assignee's email, or a muted
/// "Assign", and a click offers the workspace's members and
/// "Unassigned". One assignee, so a pick replaces the last. An assignee
/// the member list does not hold is a former member; while the list is
/// not known the row says only that there is one. Display first, as the
/// other rows: the box fills under the pointer only.
private struct AssigneeField: View {
    let assignee: String?
    let assignment: PanelAssignment
    /// A binned item: shown, not offered.
    let locked: Bool
    let set: (String?) -> Void

    private var name: String? {
        guard let assignee else { return nil }
        if let member = assignment.members?.first(where: { $0.accountId == assignee }) { return member.email }
        return assignment.members == nil ? String(localized: "Assigned") : String(localized: "A former member")
    }

    var body: some View {
        let row = PanelField(text: name ?? String(localized: "Assign"), empty: assignee == nil) {
            // The assignee's avatar in the icon's place once there is
            // one (a former member's is the dashed "?"), as on iOS.
            if let assignee {
                MemberAvatar(
                    accountId: assignee,
                    email: assignment.members?.first(where: { $0.accountId == assignee })?.email,
                    side: 16)
            } else {
                Image(systemName: "person")
            }
        }
        if locked {
            row
        } else {
            Menu {
                Picker("Assignee", selection: Binding(get: { assignee ?? "" }, set: { set($0.isEmpty ? nil : $0) })) {
                    Text("Unassigned").tag("")
                    ForEach(assignment.members ?? [], id: \.accountId) { member in
                        Text(verbatim: member.email).tag(member.accountId)
                    }
                    if let assignee, let name, assignment.members?.contains(where: { $0.accountId == assignee }) != true {
                        Text(verbatim: name).tag(assignee)
                    }
                }
                .pickerStyle(.inline)
                .labelsHidden()
            } label: {
                row
            }
            .menuStyle(.button)
            .buttonStyle(.plain)
            .menuIndicator(.hidden)
        }
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
        HStack(alignment: .firstTextBaseline, spacing: 5) {
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

/// The event flag of a dated item, as a small switch at the row's
/// leading edge with its name after it, as on the web. The whole row
/// takes the click.
private struct EventToggle: View {
    let event: Bool
    let set: (Bool) -> Void

    var body: some View {
        HStack(alignment: .center, spacing: 10) {
            Toggle("Event", isOn: Binding(get: { event }, set: set))
                .labelsHidden()
                .toggleStyle(.switch)
                .controlSize(.mini)
            Text("Event")
                .lineLimit(1)
                .foregroundStyle(event ? .primary : .secondary)
        }
        .panelBox(filled: false)
        .onTapGesture { set(!event) }
        .help("Goes by when its day passes, instead of going overdue.")
        .accessibilityElement(children: .combine)
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
    /// Takes the caret as it appears (a new item's, from the toolbar).
    let autofocus: Bool
    let commit: (String) -> Void
    @State private var draft: String
    @FocusState private var focused: Bool

    init(text: String, closed: Bool, autofocus: Bool = false, commit: @escaping (String) -> Void) {
        self.text = text
        self.closed = closed
        self.autofocus = autofocus
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
        // Focused, the list field's fill (`panelBox`) behind it, drawn
        // outset so the title does not move as the caret comes and goes.
        .background {
            RoundedRectangle(cornerRadius: PanelBox.radius, style: .continuous)
                .fill(.quaternary.opacity(focused ? 0.5 : 0))
                .padding(.horizontal, -PanelBox.inset)
                .padding(.vertical, -4)
        }
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
            // After the appearing pass: the focus set inside it is lost.
            .onAppear { if autofocus { DispatchQueue.main.async { focused = true } } }
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
