// The Agenda widget: today's open rows, then what is next, on the home
// screen. It runs no engine: it draws the snapshot the app leaves in
// the app group's container (`AgendaSnapshot` in MonoplanMacKit, which
// says what that costs), so it is as fresh as the app's last read of
// the agenda. The app reloads it whenever that read changes; by itself
// it only redraws at midnight, when "today" moves. Display only: a tap
// anywhere opens the app.

import MonoplanMacKit
import SwiftUI
import WidgetKit

@main
struct MonoplanWidgets: WidgetBundle {
    var body: some Widget {
        AgendaWidget()
    }
}

struct AgendaWidget: Widget {
    var body: some WidgetConfiguration {
        StaticConfiguration(kind: AgendaSnapshot.widgetKind, provider: AgendaProvider()) { entry in
            AgendaWidgetView(entry: entry)
                .containerBackground(.background, for: .widget)
        }
        .configurationDisplayName("Agenda")
        .description("Today, and what is next.")
        .supportedFamilies([.systemSmall, .systemMedium, .systemLarge])
    }
}

struct AgendaEntry: TimelineEntry {
    let date: Date
    /// Nil when there is no snapshot: signed out, or the app has not
    /// read the agenda yet.
    let days: [AgendaDay]?
}

struct AgendaProvider: TimelineProvider {
    private func days() -> [AgendaDay]? {
        AgendaSnapshot.url().flatMap(AgendaSnapshot.read(from:))
    }

    func placeholder(in context: Context) -> AgendaEntry {
        AgendaEntry(date: Date(), days: [])
    }

    func getSnapshot(in context: Context, completion: @escaping (AgendaEntry) -> Void) {
        completion(AgendaEntry(date: Date(), days: days()))
    }

    /// Now, and again at the next midnight, when the same snapshot
    /// reads differently (`WidgetAgenda.lines` takes the date).
    func getTimeline(in context: Context, completion: @escaping (Timeline<AgendaEntry>) -> Void) {
        let now = Date()
        let days = days()
        var entries = [AgendaEntry(date: now, days: days)]
        let calendar = Calendar.current
        if let midnight = calendar.date(byAdding: .day, value: 1, to: calendar.startOfDay(for: now)) {
            entries.append(AgendaEntry(date: midnight, days: days))
        }
        completion(Timeline(entries: entries, policy: .atEnd))
    }
}

struct AgendaWidgetView: View {
    @Environment(\.widgetFamily) private var family
    let entry: AgendaEntry

    /// How many lines fit under the title.
    private var limit: Int {
        switch family {
        case .systemLarge: return 11
        case .systemMedium: return 4
        default: return 4
        }
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            HStack(alignment: .firstTextBaseline) {
                Text("Today").font(.headline)
                Spacer(minLength: 4)
                Text(entry.date, format: .dateTime.weekday(.abbreviated).day())
                    .font(.subheadline)
                    .foregroundStyle(.secondary)
            }
            if let days = entry.days {
                content(WidgetAgenda.lines(from: days, on: entry.date, limit: limit))
            } else {
                note("Open Monoplan to load your agenda.")
            }
            Spacer(minLength: 0)
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
    }

    @ViewBuilder
    private func content(_ agenda: (lines: [WidgetLine], todayCount: Int)) -> some View {
        if agenda.todayCount == 0 {
            note(agenda.lines.isEmpty ? "Nothing scheduled." : "Nothing today.")
        }
        ForEach(agenda.lines) { line in
            switch line {
            case .heading(let day):
                Text(Self.heading(day))
                    .font(.caption.weight(.semibold))
                    .foregroundStyle(.secondary)
                    .padding(.top, 2)
            case .row(let row):
                AgendaWidgetRow(row: row, compact: family == .systemSmall)
            }
        }
        // Today's rows the widget had no room for.
        let shown = agenda.lines.prefix { if case .row = $0 { return true } else { return false } }.count
        if agenda.todayCount > shown {
            Text("+\(agenda.todayCount - shown) more")
                .font(.caption)
                .foregroundStyle(.secondary)
        }
    }

    private func note(_ text: String) -> some View {
        Text(text)
            .font(.subheadline)
            .foregroundStyle(.secondary)
    }

    /// "Tomorrow", or the day spelled out ("Fri 7 Nov").
    static func heading(_ day: String, now: Date = Date(), calendar: Calendar = .current) -> String {
        guard let date = Agenda.start(of: day, calendar: calendar) else { return day }
        if calendar.isDateInTomorrow(date) { return "Tomorrow" }
        return date.formatted(.dateTime.weekday(.abbreviated).day().month(.abbreviated))
    }
}

/// One row: a dot in the tone's colour (red overdue, orange due today),
/// the text, and the time when timed (under the text where the widget
/// is narrow, beside it otherwise).
struct AgendaWidgetRow: View {
    let row: WidgetRow
    let compact: Bool

    private var tone: Color {
        switch row.tone {
        case "overdue": return .red
        case "warning": return .orange
        default: return .accentColor
        }
    }

    var body: some View {
        HStack(alignment: .firstTextBaseline, spacing: 6) {
            Circle()
                .fill(tone)
                .frame(width: 6, height: 6)
                .alignmentGuide(.firstTextBaseline) { $0[.bottom] + 3 }
            if compact, let time = row.time {
                Text("\(Text(time).foregroundStyle(.secondary).monospacedDigit()) \(row.text)")
                    .font(.subheadline)
                    .lineLimit(1)
            } else {
                Text(row.text)
                    .font(.subheadline)
                    .lineLimit(1)
                Spacer(minLength: 4)
                if let time = row.time {
                    Text(time)
                        .font(.caption.monospacedDigit())
                        .foregroundStyle(.secondary)
                }
            }
        }
        // Item text is the user's: redacted where the system hides
        // sensitive widget content (a locked phone).
        .privacySensitive()
    }
}
