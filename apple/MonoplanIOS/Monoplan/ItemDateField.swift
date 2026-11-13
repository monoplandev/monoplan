// The item sheet's planned date (`when` and `duration`,
// spec/calendar-plan.md). On the item it is one row (`ItemDateField`),
// or, while a date read out of the title waits on an answer, the
// question in its place (`suggestion`);
// its button hands the whole sheet over to the editor
// (`ItemDateEditor`): quick picks and a month calendar, and once a date
// is set a Time switch (with the time beside it), a Length menu and
// Clear. The tick gives the sheet back to the item; the cross first
// undoes what the editor changed. Every pick is written at once
// (`item set --when / --duration`); there is no Save. Dates are
// floating wall-clock strings, never instants.

import MonoplanMacKit
import SwiftUI

/// The date as a row on the item: its value, or "Date" when unset,
/// with a small round button at the trailing edge that opens the
/// editor (a plus to add a date, a pencil to change one). Only the
/// button is tappable; the row itself is display.
///
/// A pending date suggestion (`ItemDetail.pendingSuggestion`: an open
/// item with no date whose title named one) takes the row over, as the
/// web's `WhenSuggestion.tsx` takes the date input's place: the same
/// row in orange, "Suggested: Friday 20 Nov at 08:30", so it is never
/// taken for a set date, with two answers at the trailing edge. The
/// tick makes it the item's date; the cross drops it, and the plain
/// row with its plus comes back.
struct ItemDateField: View {
    let item: ItemDetail
    let edit: () -> Void
    /// The answer to the pending suggestion: its value, and yes or no.
    let answer: (String, Bool) -> Void

    var body: some View {
        HStack(spacing: 8) {
            if let suggested = item.pendingSuggestion {
                suggestion(suggested)
            } else {
                if let when = item.when {
                    FieldRow("calendar", text: ItemView.whenText(when), note: ItemView.lengthText(item))
                } else {
                    FieldRow("calendar", text: "Date", empty: true)
                }
                Spacer(minLength: 8)
                EditBubble(adding: item.when == nil, what: "date", action: edit)
                    .disabled(item.binnedAt != nil)
            }
        }
        .frame(maxWidth: .infinity, minHeight: 36, alignment: .leading)
    }

    @ViewBuilder
    private func suggestion(_ suggested: String) -> some View {
        let label = "Suggested: \(ItemView.whenText(suggested))"
        // `FieldRow`'s anatomy, in orange.
        HStack(alignment: .firstTextBaseline, spacing: 12) {
            Image(systemName: "calendar").frame(width: 24)
            Text(label)
        }
        .foregroundStyle(.orange)
        .accessibilityElement(children: .ignore)
        .accessibilityLabel(label)
        Spacer(minLength: 8)
        answerBubble("checkmark", "Use suggested date") { answer(suggested, true) }
        answerBubble("xmark", "Dismiss suggestion") { answer(suggested, false) }
    }

    /// One answer, drawn as the row's edit button is (`EditBubble`).
    private func answerBubble(_ systemImage: String, _ name: String, action: @escaping () -> Void) -> some View {
        Button(action: action) {
            Image(systemName: systemImage)
                .font(.footnote.weight(.semibold))
                .frame(width: 18, height: 18)
        }
        .buttonStyle(.bordered)
        .buttonBorderShape(.circle)
        .accessibilityLabel(name)
    }
}

/// The date editor, in place of the item's page for as long as it is
/// open: a header (cross, the item's name, tick), then the controls.
/// Picks are written as they are made, so the tick only closes; the
/// cross puts back the date and length the editor opened with, then
/// closes.
struct ItemDateEditor: View {
    @EnvironmentObject private var model: AppModel
    let ref: ItemRef
    let item: ItemDetail
    let done: () -> Void
    /// What the item had when the editor opened, for the cross.
    @State private var original: Original

    struct Original: Equatable {
        let when: String?
        let duration: Int?
    }

    init(ref: ItemRef, item: ItemDetail, done: @escaping () -> Void) {
        self.ref = ref
        self.item = item
        self.done = done
        _original = State(initialValue: Original(when: item.when, duration: item.duration))
    }

    private func cancel() {
        if Original(when: item.when, duration: item.duration) != original {
            model.setWhen(
                ref, when: original.when,
                duration: original.duration.map(DurationEdit.minutes) ?? .clear)
        }
        done()
    }

    var body: some View {
        ScrollView {
            editor.padding()
        }
        .scrollBounceBehavior(.basedOnSize)
        .editorHeader(title: "Date", itemTitle: item.text, cancel: cancel, done: done)
    }

    private var editor: some View {
        VStack(alignment: .leading, spacing: 12) {
            HStack(spacing: 8) {
                quickPick("Today", WhenText.today())
                quickPick("Tomorrow", WhenText.today(plus: 1))
            }
            DatePicker("Date", selection: day, displayedComponents: .date)
                .datePickerStyle(.graphical)
            if let when = item.when {
                Divider()
                HStack {
                    Text("Time")
                    Spacer()
                    if item.isTimed {
                        DatePicker("Time", selection: time(on: when), displayedComponents: .hourAndMinute)
                            .labelsHidden()
                    }
                    Toggle("Time", isOn: timed(when)).labelsHidden()
                }
                .frame(minHeight: 36)
                HStack {
                    Text("Length")
                    Spacer()
                    lengthMenu
                }
                .frame(minHeight: 36)
                Divider()
                Button("Clear date", role: .destructive) {
                    // The engine clears the duration with it.
                    model.setWhen(ref, when: nil)
                }
                .frame(minHeight: 36)
            }
        }
    }

    private func quickPick(_ title: String, _ dayText: String) -> some View {
        Button(title) { setDay(dayText) }
            .buttonStyle(.bordered)
            .buttonBorderShape(.capsule)
            .controlSize(.regular)
    }

    /// A new day keeps the time of day when there is one.
    private func setDay(_ dayText: String) {
        if let when = item.when, item.isTimed {
            model.setWhen(ref, when: dayText + when.dropFirst(10).prefix(6))
        } else {
            model.setWhen(ref, when: dayText)
        }
    }

    /// The calendar's selection. With no date yet it rests on today,
    /// which is why Today is also a quick pick: tapping the day the
    /// picker already shows selected is not a change it reports.
    private var day: Binding<Date> {
        Binding(
            get: { item.when.flatMap(WhenText.date) ?? Date() },
            set: { setDay(WhenText.day($0)) }
        )
    }

    private func time(on when: String) -> Binding<Date> {
        Binding(
            get: { WhenText.date(when) ?? Date() },
            set: { model.setWhen(ref, when: "\(when.prefix(10))T\(WhenText.time($0))") }
        )
    }

    /// Time on: 09:00 for an hour (any all-day span no longer applies).
    /// Time off: the bare day, a single one.
    private func timed(_ when: String) -> Binding<Bool> {
        Binding(
            get: { item.isTimed },
            set: { on in
                if on {
                    model.setWhen(ref, when: "\(when.prefix(10))T09:00", duration: .minutes(60))
                } else {
                    model.setWhen(ref, when: String(when.prefix(10)), duration: .clear)
                }
            }
        )
    }

    // MARK: Length

    /// Minutes beside a time; whole days beside an all-day date, where
    /// one day is "no duration" (spec/calendar-plan.md).
    private var lengthMenu: some View {
        let current = Self.currentLength(item)
        var options = item.isTimed ? [0, 15, 30, 45, 60, 90, 120, 180, 240] : (1...7).map { $0 * 1440 }
        if !options.contains(current) { options = (options + [current]).sorted() }
        return Menu {
            Picker("Length", selection: Binding(
                get: { current },
                set: { model.setDuration(ref, minutes: Self.stored($0, timed: item.isTimed)) }
            )) {
                ForEach(options, id: \.self) { Text(Self.lengthLabel($0, timed: item.isTimed)).tag($0) }
            }
        } label: {
            HStack(spacing: 4) {
                Text(Self.lengthLabel(current, timed: item.isTimed))
                Image(systemName: "chevron.up.chevron.down").font(.footnote)
            }
        }
    }

    /// The menu's value: minutes when timed (0 is none), else the span
    /// rounded up to whole days in minutes (at least one day).
    static func currentLength(_ item: ItemDetail) -> Int {
        let minutes = max(0, item.duration ?? 0)
        if item.isTimed { return minutes }
        return max(1, (minutes + 1439) / 1440) * 1440
    }

    /// What a menu value is stored as: nil for "none" (no minutes, or a
    /// single day).
    static func stored(_ value: Int, timed: Bool) -> Int? {
        if timed { return value > 0 ? value : nil }
        return value > 1440 ? value : nil
    }

    static func lengthLabel(_ value: Int, timed: Bool) -> String {
        if timed { return value > 0 ? AgendaRowView.span(value) : "None" }
        let days = value / 1440
        return days == 1 ? "1 day" : "\(days) days"
    }
}

/// Floating `when` text to and from `Date`s, always in the Gregorian
/// calendar the format is written in, at the device's time zone.
enum WhenText {
    static var calendar: Calendar {
        var calendar = Calendar(identifier: .gregorian)
        calendar.locale = .current
        calendar.timeZone = .current
        return calendar
    }

    static func date(_ when: String) -> Date? { Agenda.start(of: when, calendar: calendar) }

    /// `YYYY-MM-DD`.
    static func day(_ date: Date) -> String {
        let c = calendar.dateComponents([.year, .month, .day], from: date)
        return String(format: "%04d-%02d-%02d", c.year ?? 0, c.month ?? 0, c.day ?? 0)
    }

    /// `HH:MM`.
    static func time(_ date: Date) -> String {
        let c = calendar.dateComponents([.hour, .minute], from: date)
        return String(format: "%02d:%02d", c.hour ?? 0, c.minute ?? 0)
    }

    static func today(plus days: Int = 0, now: Date = Date()) -> String {
        day(calendar.date(byAdding: .day, value: days, to: now) ?? now)
    }
}
