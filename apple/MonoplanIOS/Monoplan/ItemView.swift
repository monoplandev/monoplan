// An item's page: the web client's task dialog (js/web/src/TaskDialog.tsx)
// laid out top to bottom in the same order: status, title, list, planned
// date, deadline, place, notes, then the activity log. Read from
// `item show --json` and refetched on every doc change while shown.
// It comes up as a sheet over whatever list the row was in (half height,
// draggable to full), as the dialog floats over the web's lists, rather
// than as a pushed page. The checkbox ticks and reopens the item;
// everything else is display only.

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

    func body(content: Content) -> some View {
        content
            .environment(\.openItem) { open = $0 }
            .sheet(item: $open) { ref in
                ItemView(ref: ref)
                    .environmentObject(model)
                    .presentationDetents([.medium, .large])
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

    private var item: ItemDetail? { model.itemDetails[ref.key] }

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 20) {
                status
                Text(item?.text ?? ref.title)
                    .font(.title2.weight(.semibold))
                    .strikethrough(item?.state == "done")
                    .foregroundStyle(item?.closedAt == nil ? .primary : .secondary)
                    .textSelection(.enabled)
                if let item {
                    fields(item)
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
        .onAppear { model.refreshItem(ref) }
        .onChange(of: model.changeCount) { model.refreshItem(ref) }
    }

    /// The checkbox with the workflow state beside it, as one status
    /// control; a binned item says so instead (the bin masks the state).
    private var status: some View {
        HStack(spacing: 10) {
            // Ticking a binned item makes no sense (the bin masks the
            // state; Restore is the way out), and neither does ticking
            // one that has not loaded.
            if let item, item.binnedAt == nil {
                ItemCheckButton(ref: ref, state: item.state)
            } else {
                ItemCheckbox(state: item?.state ?? "backlog")
            }
            if let item {
                Text(item.binnedAt != nil ? "Binned" : ItemRowView.stateLabel(item.state))
                    .font(.subheadline.weight(.medium))
                    .padding(.horizontal, 10)
                    .padding(.vertical, 4)
                    .background(.quaternary, in: Capsule())
            }
        }
    }

    /// List, planned date, deadline (only when set, as on the web) and
    /// place. The date and place rows are always there, muted when
    /// empty: they are where setting one will happen.
    private func fields(_ item: ItemDetail) -> some View {
        VStack(alignment: .leading, spacing: 14) {
            FieldRow(text: item.listLabel) { ListIcon(listId: item.listId, emoji: item.listIcon) }
            if let when = item.when {
                FieldRow("calendar", text: Self.whenText(when), note: Self.lengthText(item))
            } else {
                FieldRow("calendar", text: "No date", empty: true)
            }
            if let deadline = item.deadline {
                FieldRow("timer", text: "Due \(Self.dayText(deadline))")
            }
            if let place = item.place {
                FieldRow("mappin.and.ellipse", text: place.label, note: place.address)
            } else {
                FieldRow("mappin.and.ellipse", text: "No place", empty: true)
            }
        }
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

/// One field of the page: its icon, its value, and a secondary line
/// under it when there is one. `empty` mutes a field that is unset.
private struct FieldRow<Icon: View>: View {
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

private extension FieldRow where Icon == Image {
    /// A field led by an SF Symbol.
    init(_ systemImage: String, text: String, note: String? = nil, empty: Bool = false) {
        self.init(text: text, note: note, empty: empty) { Image(systemName: systemImage) }
    }
}
