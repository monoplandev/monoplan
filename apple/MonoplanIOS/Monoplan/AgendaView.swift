// The Agenda page, pushed from Home (which owns the navigation stack
// and the item sheet): the engine's agenda projection (`Engine.agenda`,
// spec/cli.md, with a horizon far enough out to be the whole future,
// `AppModel.agendaDays`) as sections: Overdue while something is
// (overdue deadlines and tasks whose planned day has passed), Today,
// then each non-empty day. A row opens the item's sheet
// (`ItemView`) and its checkbox ticks it, as in the other lenses.
//
// The days gone by (`AppModel.agendaPastDays`: the events that
// happened on them) are sections above
// Overdue and Today. The page opens with Today's header at the top
// (Overdue's, when it is there), so they are reached by scrolling up,
// and the "Today" button comes back to the same place; Today's section
// is always there to anchor on, with a line in it when it has no rows. The list is a UIKit
// collection view (`AgendaCollection.swift`), which can be put on
// Today exactly; a SwiftUI `List` cannot.

import MonoplanCore
import MonoplanMacKit
import SwiftUI

struct AgendaView: View {
    @EnvironmentObject private var model: AppModel
    @Environment(\.openItem) private var openItem
    /// Counts the taps on "Today": each one scrolls the list back to
    /// Today's header, or Overdue's above it.
    @State private var todayJumps = 0

    var body: some View {
        Group {
            if model.agenda.allSatisfy({ $0.rows.isEmpty }) {
                emptyState
            } else {
                AgendaCollection(groups: model.agenda, model: model, todayJumps: todayJumps, openItem: openItem) {
                    model.refreshAgenda()
                }
                // Under the bars, as a list is: the collection view
                // insets its own content for them.
                .ignoresSafeArea()
                .toolbar {
                    ToolbarItem(placement: .topBarTrailing) {
                        Button("Today") { todayJumps += 1 }
                    }
                }
            }
        }
        .navigationTitle("Calendar")
        .refreshable { model.refreshAgenda() }
        .onAppear { model.refreshAgenda() }
    }

    private var emptyState: some View {
        ScrollView {
            if let error = model.agendaError {
                ContentUnavailableView(
                    "Calendar unavailable",
                    systemImage: "exclamationmark.triangle",
                    description: Text(error)
                )
            } else if model.sync == nil, case .connecting(let why) = model.engineState {
                ContentUnavailableView("Calendar", systemImage: "calendar", description: Text(why))
            } else {
                ContentUnavailableView(
                    "Nothing scheduled",
                    systemImage: "calendar",
                    description: Text("Items with a date or deadline will show here.")
                )
            }
        }
    }

    /// "Today 13 Nov" (the date in the weekday's place) or the day
    /// spelled out ("Friday 7 Nov").
    static func title(for day: AgendaGroup) -> String {
        if day.today {
            let date = Agenda.start(of: day.day) ?? Date()
            return String(localized: "Today \(date.formatted(.dateTime.day().month(.abbreviated)))")
        }
        guard let date = Agenda.start(of: day.day) else { return day.day }
        return date.formatted(.dateTime.weekday(.wide).day().month(.abbreviated))
    }
}

/// One agenda row, laid out as the other lenses' rows are: the
/// checkbox, the text (its star right after it when starred) with
/// under it the list it is in (icon and name; none for an item on the
/// calendar alone), its place when it has one and then
/// the workspace when the row is not Personal's, and on the right the time when timed with a note under
/// it for the tone (overdue in red, due today in orange) or the
/// duration. The checkbox ticks; the rest of the row opens the sheet.
/// An event has the event pill for a checkbox while it is open or its
/// day has gone by (`AgendaEntry.showsCheck`). A task done today is
/// muted at the end of Today, with no time or length.
struct AgendaRowView: View {
    let row: AgendaEntry

    var body: some View {
        ItemLink(ItemRef(docId: row.docId, itemId: row.id, title: row.text)) {
            CheckboxRow(state: row.state, trailingStar: false, moves: row.quickMoves, check: row.showsCheck) {
                HStack(alignment: .firstTextBaseline, spacing: 8) {
                    VStack(alignment: .leading, spacing: 2) {
                        // The star sits by the text, not past the
                        // time on the right.
                        HStack(alignment: .firstTextBaseline, spacing: 6) {
                            Text(row.text)
                                .itemTextSlot()
                                .foregroundStyle(row.isDone ? .secondary : .primary)
                            RowStar(state: row.state)
                        }
                        if let list = row.listName {
                            // The list's icon, then its name.
                            HStack(alignment: .firstTextBaseline, spacing: 4) {
                                ListIcon(listId: row.listId, emoji: row.listIcon)
                                    .imageScale(.small)
                                Text(verbatim: list).lineLimit(1)
                            }
                            .font(.caption)
                            .foregroundStyle(.secondary)
                            .accessibilityElement(children: .ignore)
                            .accessibilityLabel(Text(verbatim: list))
                        }
                        if let place = row.place {
                            // The item page's place glyph, then the label.
                            HStack(alignment: .firstTextBaseline, spacing: 4) {
                                PinIcon(size: 11)
                                Text(place.label).lineLimit(1)
                            }
                            .font(.caption)
                            .foregroundStyle(.secondary)
                            .accessibilityElement(children: .ignore)
                            .accessibilityLabel("At \(place.label)")
                        }
                        if let name = row.docName {
                            // The shared glyph, then the workspace's
                            // name, as `SharedWorkspaceBadge` has them.
                            HStack(spacing: 4) {
                                SharedIcon()
                                Text(name).lineLimit(1)
                            }
                            .font(.caption)
                            .foregroundStyle(.tertiary)
                            .accessibilityElement(children: .ignore)
                            .accessibilityLabel("Shared workspace \(name)")
                        }
                    }
                    Spacer(minLength: 8)
                    VStack(alignment: .trailing, spacing: 2) {
                        if row.isTimed, let when = row.when {
                            Text(String(when.dropFirst(11).prefix(5)))
                                .font(.subheadline.monospacedDigit())
                                .foregroundStyle(.secondary)
                        }
                        if let note {
                            Text(note.text)
                                .font(.footnote)
                                .foregroundStyle(note.color)
                        }
                    }
                }
            }
        }
    }

    private var note: (text: String, color: Color)? {
        switch row.tone {
        case .overdue: return (String(localized: "Overdue"), .red)
        case .warning: return (String(localized: "Due today"), .orange)
        case .neutral: break
        }
        // A deadline's own row says what it is; the length belongs to
        // the `when`'s row.
        if row.placedBy == .deadline {
            return row.deadline.map { (String(localized: "due \($0)"), .secondary) }
        }
        if !row.isCompletedToday, let minutes = row.duration, minutes > 0 {
            return (Self.span(Int(minutes)), .secondary)
        }
        return nil
    }

    /// "45m", "1h 30m", "3d".
    static func span(_ minutes: Int) -> String {
        let days = minutes / 1440
        let hours = (minutes % 1440) / 60
        let mins = minutes % 60
        let d = String(localized: "\(days)d")
        let h = String(localized: "\(hours)h")
        let m = String(localized: "\(mins)m")
        if days > 0 { return hours > 0 ? "\(d) \(h)" : d }
        if hours > 0 { return mins > 0 ? "\(h) \(m)" : h }
        return m
    }
}
