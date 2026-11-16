// The Done page, pushed from Home (which owns the navigation stack):
// what got finished in every open workspace, newest first, a section
// per day ticked (`Engine.doneEverywhere`), each row naming its list
// and, when not Personal's, its workspace. The bar's filter button
// narrows it to chosen workspaces and lists (`DoneFilterView.swift`),
// and shows how many checks are in effect.
// A window that grows on scroll rather than the whole history: the
// trailing spinner row asks for more as it comes into sight. A row's
// checkbox reopens the item; the row opens its sheet.

import MonoplanCore
import SwiftUI

struct DoneView: View {
    @EnvironmentObject private var model: AppModel
    @State private var filtering = false

    var body: some View {
        let filterCount = model.doneFilter.count
        Group {
            if model.doneDays.isEmpty {
                emptyState
            } else {
                List {
                    ForEach(model.doneDays) { day in
                        Section(Self.title(for: day.day)) {
                            ForEach(day.rows, id: \.key) { row in
                                ItemLink(row.ref) {
                                    DoneRowView(
                                        item: row.item,
                                        listName: model.listName(
                                            row.item.listId, inDoc: row.docId, dated: row.item.when != nil),
                                        workspace: model.sharedWorkspaceName(row.docId))
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
                            .listRowSeparator(.hidden)
                            .id(model.doneLoadedLimit)
                            .onAppear { model.loadMoreDone() }
                    }
                }
                // Plain: no grouped surface, the day's header is
                // enough. A plain list also pins the header of the
                // section being read under the bar, so the day stays
                // in sight until the next one pushes it out.
                .listStyle(.plain)
                .animation(.default, value: model.doneDays)
            }
        }
        .navigationTitle("Done")
        // Done loads a window at a time, so the count is asked for on
        // its own: every Done item the filter admits, loaded or not.
        .countedTitle(String(localized: "Done"), count: model.doneError == nil ? model.doneTotal : nil) {
            CatalogIcon(name: "Done")
        }
        .toolbar {
            ToolbarItem(placement: .topBarTrailing) {
                Button { filtering = true } label: {
                    // The count rides beside the glyph while any check
                    // is in effect, as the web's trigger has it.
                    HStack(spacing: 4) {
                        Image(systemName: "line.3.horizontal.decrease")
                        if filterCount > 0 {
                            Text(verbatim: "\(filterCount)").monospacedDigit()
                        }
                    }
                    .fontWeight(filterCount > 0 ? .semibold : .regular)
                    .foregroundStyle(filterCount > 0 ? AnyShapeStyle(.tint) : AnyShapeStyle(.primary))
                }
                .accessibilityLabel("Filter")
                .accessibilityValue(filterCount > 0 ? String(localized: "\(filterCount) active") : "")
            }
        }
        .sheet(isPresented: $filtering) { DoneFilterView() }
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
            } else if model.sync == nil, case .connecting(let why) = model.engineState {
                ContentUnavailableView("Done", systemImage: "checkmark.circle", description: Text(why))
            } else if !model.doneFilter.isEmpty {
                ContentUnavailableView {
                    Label("Nothing done here", systemImage: "line.3.horizontal.decrease")
                } description: {
                    Text("No finished items match the filter.")
                } actions: {
                    Button("Clear Filters") { model.clearDoneFilter() }
                }
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
        if calendar.isDate(day, inSameDayAs: now) { return String(localized: "Today") }
        if let yesterday = calendar.date(byAdding: .day, value: -1, to: now),
           calendar.isDate(day, inSameDayAs: yesterday) {
            return String(localized: "Yesterday")
        }
        if calendar.isDate(day, equalTo: now, toGranularity: .year) {
            return day.formatted(.dateTime.weekday(.wide).day().month(.abbreviated))
        }
        return day.formatted(.dateTime.weekday(.wide).day().month(.abbreviated).year())
    }
}

/// One finished item: the time it was ticked (the section header
/// already says the day) leading its text on one line, with the list it
/// lives in underneath and, when that is not Personal's, its workspace.
struct DoneRowView: View {
    let item: Item
    let listName: String?
    var workspace: String? = nil

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
                if let note {
                    Text(note)
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

    private var note: String? {
        let parts = [listName, workspace].compactMap { $0 }
        return parts.isEmpty ? nil : parts.joined(separator: " · ")
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
