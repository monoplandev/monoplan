// The Agenda page, pushed from Home (which owns the navigation stack
// and the item sheet): the daemon's agenda projection (`agenda --json`,
// spec/cli.md, with a horizon far enough out to be the whole future,
// `AppModel.agendaDays`) as sections: Today first (overdue deadlines
// and tasks whose planned day has passed folded in), Unscheduled
// events, then each non-empty day. A row opens the item's sheet
// (`ItemView`) and its checkbox ticks it, as in the other lenses.

import MonoplanMacKit
import SwiftUI

struct AgendaView: View {
    @EnvironmentObject private var model: AppModel

    var body: some View {
        Group {
            if model.agenda.allSatisfy({ $0.rows.isEmpty }) {
                emptyState
            } else {
                List {
                    ForEach(model.agenda, id: \.day) { day in
                        if !day.rows.isEmpty {
                            Section(Self.title(for: day)) {
                                ForEach(day.rows, id: \.id) { row in
                                    AgendaRowView(row: row)
                                }
                            }
                        }
                    }
                }
                .listStyle(.insetGrouped)
                .animation(.default, value: model.agenda)
            }
        }
        .navigationTitle("Agenda")
        .refreshable { model.refreshAgenda() }
        .onAppear { model.refreshAgenda() }
    }

    private var emptyState: some View {
        ScrollView {
            if let error = model.agendaError {
                ContentUnavailableView(
                    "Agenda unavailable",
                    systemImage: "exclamationmark.triangle",
                    description: Text(error)
                )
            } else if model.sync == nil, case .connecting(let why) = model.clientState {
                ContentUnavailableView("Agenda", systemImage: "calendar", description: Text(why))
            } else {
                ContentUnavailableView(
                    "Nothing scheduled",
                    systemImage: "calendar",
                    description: Text("Items with a date or deadline will show here.")
                )
            }
        }
    }

    /// "Today", "Unscheduled", or the day spelled out ("Friday 7 Nov").
    static func title(for day: AgendaDay) -> String {
        if day.today { return "Today" }
        guard let text = day.day else { return "Unscheduled" }
        guard let date = Agenda.start(of: text) else { return text }
        return date.formatted(.dateTime.weekday(.wide).day().month(.abbreviated))
    }
}

/// One agenda row, laid out as the other lenses' rows are: the
/// checkbox, the text with the workspace under it when the row is not
/// Personal's, and on the right the time when timed with a note under
/// it for the tone (overdue in red, due today in orange) or the
/// duration. The checkbox ticks; the rest of the row opens the sheet.
struct AgendaRowView: View {
    let row: AgendaRow

    var body: some View {
        ItemLink(ItemRef(docId: row.docId, itemId: row.id, title: row.text)) {
            CheckboxRow(state: row.state) {
                HStack(alignment: .firstTextBaseline, spacing: 8) {
                    VStack(alignment: .leading, spacing: 2) {
                        Text(row.text)
                            .strikethrough(row.isDone)
                            .foregroundStyle(row.isDone ? .secondary : .primary)
                        if let name = row.docName {
                            Text(name)
                                .font(.caption)
                                .foregroundStyle(.tertiary)
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
        case "overdue": return ("overdue", .red)
        case "warning": return ("due today", .orange)
        default: break
        }
        if let minutes = row.duration, minutes > 0 {
            return (Self.span(minutes), .secondary)
        }
        if row.placedBy == "deadline", let deadline = row.deadline, !row.isTimed {
            return ("due \(deadline)", .secondary)
        }
        return nil
    }

    /// "45m", "1h 30m", "3d".
    static func span(_ minutes: Int) -> String {
        let days = minutes / 1440
        let hours = (minutes % 1440) / 60
        let mins = minutes % 60
        if days > 0 { return hours > 0 ? "\(days)d \(hours)h" : "\(days)d" }
        if hours > 0 { return mins > 0 ? "\(hours)h \(mins)m" : "\(hours)h" }
        return "\(mins)m"
    }
}
