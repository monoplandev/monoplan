// The Done page, pushed from Home (which owns the navigation stack):
// what got finished in the Personal workspace, newest first, a section
// per day ticked (`done --json`, spec/cli.md §Items).
// A window that grows on scroll rather than the whole history: the
// trailing spinner row asks for more as it comes into sight. A row's
// checkbox reopens the item; the row opens its sheet.

import MonoplanMacKit
import SwiftUI

struct DoneView: View {
    @EnvironmentObject private var model: AppModel

    var body: some View {
        Group {
            if model.doneDays.isEmpty {
                emptyState
            } else {
                List {
                    ForEach(model.doneDays) { day in
                        Section(Self.title(for: day.day)) {
                            ForEach(day.rows) { item in
                                ItemLink(ItemRef(docId: nil, itemId: item.id, title: item.text)) {
                                    DoneRowView(item: item, listName: model.listNames[item.listId])
                                }
                            }
                        }
                    }
                    if model.doneHasMore {
                        // A fresh identity per loaded window, so the
                        // row asks again if it is still in sight
                        // after a load.
                        ProgressView()
                            .frame(maxWidth: .infinity)
                            .listRowBackground(Color.clear)
                            .id(model.doneLoadedLimit)
                            .onAppear { model.loadMoreDone() }
                    }
                }
                .listStyle(.insetGrouped)
                .animation(.default, value: model.doneDays)
            }
        }
        .navigationTitle("Done")
        // Done loads a window at a time: the count is of what has
        // loaded, with a "+" while there is more.
        .countedTitle(
            "Done",
            count: model.doneError == nil ? model.doneDays.reduce(0) { $0 + $1.rows.count } : nil,
            more: model.doneHasMore
        ) {
            Image(systemName: "checkmark.circle")
        }
        .refreshable { model.refreshDone() }
        .onAppear { model.refreshDone() }
    }

    private var emptyState: some View {
        ScrollView {
            if let error = model.doneError {
                ContentUnavailableView(
                    "Done unavailable",
                    systemImage: "exclamationmark.triangle",
                    description: Text(error)
                )
            } else if model.sync == nil, case .connecting(let why) = model.clientState {
                ContentUnavailableView("Done", systemImage: "checkmark.circle", description: Text(why))
            } else {
                ContentUnavailableView(
                    "Nothing done yet",
                    systemImage: "checkmark.circle",
                    description: Text("Items you finish show here, newest first.")
                )
            }
        }
    }

    /// "Today", "Yesterday", or the day spelled out ("Friday 7 Nov"),
    /// with the year once it is not this one.
    static func title(for day: Date, calendar: Calendar = .current, now: Date = Date()) -> String {
        if calendar.isDate(day, inSameDayAs: now) { return "Today" }
        if let yesterday = calendar.date(byAdding: .day, value: -1, to: now),
           calendar.isDate(day, inSameDayAs: yesterday) {
            return "Yesterday"
        }
        if calendar.isDate(day, equalTo: now, toGranularity: .year) {
            return day.formatted(.dateTime.weekday(.wide).day().month(.abbreviated))
        }
        return day.formatted(.dateTime.weekday(.wide).day().month(.abbreviated).year())
    }
}

/// One finished item: the time it was ticked (the section header
/// already says the day) leading its text on one line, with the list it
/// lives in underneath.
struct DoneRowView: View {
    let item: ItemRow
    let listName: String?

    var body: some View {
        CheckboxRow(state: item.state) {
            VStack(alignment: .leading, spacing: 2) {
                HStack(alignment: .firstTextBaseline, spacing: 8) {
                    // Sized to the widest time of day, so the text
                    // starts at the same place on every row.
                    Text(Self.widestTime)
                        .hidden()
                        .overlay(alignment: .trailing) { Text(Self.time(doneAt)) }
                        .foregroundStyle(.secondary)
                        .fixedSize()
                        .accessibilityElement()
                        .accessibilityLabel(Self.time(doneAt))
                    Text(item.text)
                        .lineLimit(1)
                        .truncationMode(.tail)
                }
                if let listName {
                    Text(listName)
                        .foregroundStyle(.secondary)
                        .lineLimit(1)
                        .truncationMode(.tail)
                }
            }
            // Half the row's usual gap after the checkbox: the stamp
            // reads as part of the tick.
            .padding(.leading, -6)
        }
    }

    private var doneAt: Date {
        Date(timeIntervalSince1970: TimeInterval(item.lifecycleAt) / 1000)
    }

    static func time(_ date: Date) -> String {
        date.formatted(date: .omitted, time: .shortened)
    }

    /// A time with two-digit hours in either clock ("10:00 PM", "22:00"):
    /// about as wide as a stamp gets.
    private static let widestTime: String = {
        let late = Calendar.current.date(bySettingHour: 22, minute: 0, second: 0, of: Date()) ?? Date()
        return time(late)
    }()
}
