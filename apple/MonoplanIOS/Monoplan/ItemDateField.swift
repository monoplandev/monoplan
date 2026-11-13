// The item sheet's planned date (`when` and `duration`,
// spec/calendar-plan.md). On the item it is one row (`ItemDateField`),
// or, while a date read out of the title waits on an answer, the
// question in its place (`suggestion`);
// its button hands the whole sheet over to the editor
// (`ItemDateEditor`): a text box that reads a typed date ("fri 3pm")
// and offers what it means (Today and Tomorrow until something is
// typed), then a month calendar that shows a multi-day date as a run
// of days and picks one in two taps, and once a date is set its start and
// end time side by side with Clear time under them (an all-day date
// has Add time and a Length in days instead), and Clear date. The tick
// gives the sheet back to the item; the cross first undoes what the
// editor changed. Every pick is written at once
// (`item set --when / --duration`); there is no Save. Dates are
// floating wall-clock strings, never instants.

import MonoplanMacKit
import SwiftUI

/// The date as a row on the item: its value, or "Date" when unset,
/// with a small round button at the trailing edge that opens the
/// editor (a plus to add a date, a pencil to change one). With no date
/// the whole row opens it; once one is set only the pencil does, the
/// row itself display, so changing a date takes a deliberate tap.
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
                    let parts = ItemView.whenParts(when)
                    FieldRow("calendar", text: parts.text, muted: parts.muted, note: ItemView.lengthText(item))
                } else {
                    FieldRow("calendar", text: "Date", empty: true)
                }
                Spacer(minLength: 8)
                EditBubble(adding: item.when == nil, what: "date", action: edit)
                    .disabled(item.binnedAt != nil)
            }
        }
        .frame(maxWidth: .infinity, minHeight: 36, alignment: .leading)
        .addsOnTap(
            item.when == nil && item.pendingSuggestion == nil && item.binnedAt == nil, action: edit)
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
///
/// The text box at the top is where the editor starts, keyboard up: a
/// date typed as words ("fri 3pm", "tomorrow", "14 nov") is read by
/// `WhenPhrase` as it is typed and offered underneath as one or two
/// readings (`WhenPhrase.readings`: the second is there when the phrase
/// said only a day or only a time and the item's date has the other
/// half). Before anything is typed the offers are Today and Tomorrow
/// (none once the item has a date);
/// typing replaces them, except one the text is the start of ("tom"
/// keeps Tomorrow). A tap on an offer, or Return for the first once
/// something is typed, makes it the date and leaves the editor up
/// with the keyboard, so a day can be followed by a time
/// ("fri", then "3pm": a time alone goes on the item's day). Only the
/// tick (which first takes what is typed) and the cross close it.
///
/// While a date is set and nothing is being typed the box says the
/// date ("Friday 20 Nov at 15:00"), and the editor opens with the
/// keyboard down. Focusing the box selects that text, so typing
/// replaces it; the offers read only what was typed, never the date
/// shown. The controls below are
/// as they were, a scroll or a tap away.
struct ItemDateEditor: View {
    @EnvironmentObject private var model: AppModel
    let ref: ItemRef
    let item: ItemDetail
    let done: () -> Void
    /// What the item had when the editor opened, for the cross.
    @State private var original: Original
    @State private var query = ""
    @State private var selection: TextSelection?
    /// The last tap on the calendar set a start day: the next one, on
    /// a later day, ends the span there.
    @State private var awaitingEnd = false
    @FocusState private var typing: Bool

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
        .scrollDismissesKeyboard(.interactively)
        .editorHeader(title: "Date", itemTitle: item.text, cancel: cancel, done: finish)
        .onAppear {
            // No date: start typing. A date: show it, keyboard down.
            query = shown
            typing = item.when == nil
        }
        .onChange(of: item.when) {
            // Whatever set it (an offer, the calendar, a time, Clear),
            // the box says the date again, ready to be typed over.
            query = shown
            if typing { selectAll() }
        }
        .onChange(of: typing) {
            if typing {
                selectAll()
            } else if query.isEmpty {
                query = shown
            }
        }
    }

    // MARK: Typed date

    /// What the box says while nothing is being typed: the item's
    /// date, or nothing.
    private var shown: String { item.when.map(ItemView.whenText) ?? "" }

    /// What has been typed: the box's text, unless that is still the
    /// date it shows.
    private var typed: String {
        query == shown ? "" : query.trimmingCharacters(in: .whitespacesAndNewlines)
    }

    /// Select the box's text, so the next key replaces it. A turn
    /// later: the selection set while focus is still arriving is
    /// dropped for the caret the tap places.
    private func selectAll() {
        DispatchQueue.main.async {
            selection = query.isEmpty ? nil : TextSelection(range: query.startIndex..<query.endIndex)
        }
    }


    /// One offer under the text box: a date, and for the standing ones
    /// their name ("Today").
    struct Offer: Equatable {
        let when: String
        var name: String?
    }

    /// The offers, likeliest first. The standing two (Today, Tomorrow:
    /// both with nothing typed on an item with no date, none on one
    /// with a date, then only those the text is the start of), each keeping the item's time of day as a calendar
    /// pick does; then what the text reads as, less any date already
    /// offered.
    private var offers: [Offer] {
        Self.offers(typed: typed, current: item.when, today: WhenText.today(), tomorrow: WhenText.today(plus: 1))
    }

    static func offers(typed: String, current: String?, today: String, tomorrow: String) -> [Offer] {
        let time = current.map { $0.count >= 16 ? String($0.dropFirst(10).prefix(6)) : "" } ?? ""
        let word = typed.lowercased()
        var offers = [("Today", today), ("Tomorrow", tomorrow)]
            .filter { word.isEmpty ? current == nil : $0.0.lowercased().hasPrefix(word) }
            .map { Offer(when: $0.1 + time, name: $0.0) }
        if let phrase = WhenPhrase.find(in: typed) {
            for when in phrase.readings(over: current, today: today)
            where !offers.contains(where: { $0.when == when }) {
                offers.append(Offer(when: when))
            }
        }
        return offers
    }

    private var typedBox: some View {
        HStack(spacing: 8) {
            Image(systemName: "calendar").foregroundStyle(.secondary)
            TextField("Type a date: fri 3pm", text: $query, selection: $selection)
                .focused($typing)
                .submitLabel(.done)
                .autocorrectionDisabled()
                .textInputAutocapitalization(.never)
                .onSubmit(submit)
            if !query.isEmpty {
                Button(action: clearDate) {
                    Image(systemName: "xmark.circle.fill").foregroundStyle(.tertiary)
                }
                .buttonStyle(.plain)
                .accessibilityLabel("Clear date")
            }
        }
        .padding(.horizontal, 12)
        .frame(minHeight: 40)
        .background(.quaternary.opacity(0.5), in: RoundedRectangle(cornerRadius: 10, style: .continuous))
    }

    /// The offers, each a row to tap; a muted line when the text names
    /// no date.
    @ViewBuilder
    private var offerRows: some View {
        let offers = offers
        if offers.isEmpty {
            if !typed.isEmpty {
                Text("No date in that yet")
                    .foregroundStyle(.secondary)
                    .frame(minHeight: 36)
            }
        } else {
            ForEach(offers, id: \.when) { offer in
                Button {
                    use(offer.when)
                } label: {
                    HStack(alignment: .firstTextBaseline, spacing: 12) {
                        Image(systemName: "arrow.turn.down.right")
                            .foregroundStyle(.secondary)
                            .frame(width: 24)
                        if let name = offer.name {
                            Text(name)
                            Spacer(minLength: 8)
                            Text(ItemView.whenText(offer.when)).foregroundStyle(.secondary)
                        } else {
                            Text(ItemView.whenText(offer.when))
                        }
                    }
                    .frame(maxWidth: .infinity, minHeight: 36, alignment: .leading)
                    .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
                Divider()
            }
        }
    }

    /// Return: the first offer for the typed text, and the editor
    /// stays for more. Text that names no date stays up to be fixed;
    /// with nothing typed Return closes, as the tick does.
    private func submit() {
        if typed.isEmpty {
            done()
        } else {
            if let first = offers.first { use(first.when) }
            typing = true
        }
    }

    /// The tick: what is typed, if it names a date, then back to the
    /// item. Text that names none stays up to be fixed.
    private func finish() {
        if typed.isEmpty {
            done()
        } else if let first = offers.first {
            use(first.when)
            done()
        } else {
            typing = true
        }
    }

    /// An offer as the item's date; the box then says it, selected,
    /// for the next thing to type. Going between all-day and timed the old length
    /// no longer applies: an hour beside a time, a single day without
    /// one.
    private func use(_ when: String) {
        let timed = when.count >= 16
        var duration = DurationEdit.keep
        if item.when != nil, timed != item.isTimed { duration = timed ? .minutes(60) : .clear }
        model.setWhen(ref, when: when, duration: duration)
        // Now, not only when the write comes back: the same date
        // picked again changes nothing to hear of.
        query = ItemView.whenText(when)
        if typing { selectAll() }
    }

    private func clearDate() {
        query = ""
        selection = nil
        awaitingEnd = false
        // The engine clears the duration with it.
        model.setWhen(ref, when: nil)
    }

    private var editor: some View {
        VStack(alignment: .leading, spacing: 12) {
            typedBox
            offerRows
            MultiDatePicker("Date", selection: days)
                .environment(\.calendar, WhenText.calendar)
            if let when = item.when {
                Divider()
                if item.isTimed {
                    // Start and end, as the web's time row: the end is
                    // start + the stored length.
                    HStack(spacing: 12) {
                        timeBox("clock", "Start", time(on: when))
                        timeBox("arrow.right", "End", end(of: when), note: Self.endDayNote(when, item.duration))
                    }
                    .frame(minHeight: 36)
                    Button("Clear time") {
                        // Back to all-day, a single one.
                        model.setWhen(ref, when: String(when.prefix(10)), duration: .clear)
                    }
                    .frame(minHeight: 36)
                } else {
                    HStack {
                        Button("Add time", systemImage: "clock") {
                            // 09:00 for an hour (any all-day span no
                            // longer applies).
                            model.setWhen(ref, when: "\(when.prefix(10))T09:00", duration: .minutes(60))
                        }
                        .buttonStyle(.bordered)
                        .buttonBorderShape(.capsule)
                        Spacer()
                        lengthMenu
                    }
                    .frame(minHeight: 36)
                }
                Divider()
                Button("Clear date", role: .destructive, action: clearDate)
                .frame(minHeight: 36)
            }
        }
    }

    // MARK: Calendar

    /// The calendar's selection: every day the item's span covers, so
    /// a multi-day date shows as a run of days. The picker
    /// (`MultiDatePicker`) only toggles single days, so each change it
    /// reports is read as a tap on the one day that differs (`tap`)
    /// and the selection it proposed is never kept: what shows is
    /// always the span as stored.
    private var days: Binding<Set<DateComponents>> {
        Binding(
            get: { Set(Self.spanDays(item).map(Self.components)) },
            set: { proposed in
                let now = Set(Self.spanDays(item))
                let changed = Set(proposed.compactMap(Self.dayText)).symmetricDifference(now)
                if changed.count == 1, let day = changed.first { tap(day) }
            }
        )
    }

    /// A tap on a day of the calendar, as a range picker reads it: the
    /// first sets the day (the span back to a single one, the time of
    /// day kept), the next on a later day ends the span there, the one
    /// after starts over. A span is at most `maxDays`, as the Length
    /// menu's; a tap past that is a new start. A tap on the only day
    /// selected changes nothing: Clear date is what removes a date.
    private func tap(_ day: String) {
        guard let when = item.when else {
            model.setWhen(ref, when: day)
            awaitingEnd = true
            return
        }
        let start = String(when.prefix(10))
        let single = Self.spanDays(item).count == 1
        let ahead = Self.daysBetween(start, day)
        if awaitingEnd, single, ahead > 0, ahead < Self.maxDays {
            if item.isTimed {
                // The end keeps its time of day, on the new last day.
                let startClock = Self.startClock(when)
                let endClock = (startClock + max(0, item.duration ?? 0)) % 1440
                model.setDuration(ref, minutes: ahead * 1440 + endClock - startClock)
            } else {
                model.setDuration(ref, minutes: (ahead + 1) * 1440)
            }
            awaitingEnd = false
        } else if day == start, single {
            awaitingEnd = true
        } else {
            if item.isTimed {
                // Whole days come off the length; the end's time stays.
                let length = max(0, item.duration ?? 0)
                model.setWhen(
                    ref, when: day + when.dropFirst(10).prefix(6),
                    duration: length >= 1440 ? .minutes(length % 1440 == 0 ? 60 : length % 1440) : .keep)
            } else {
                model.setWhen(ref, when: day, duration: .clear)
            }
            awaitingEnd = true
        }
    }

    static let maxDays = 7

    /// The days (`YYYY-MM-DD`) the item's date covers, in order: whole
    /// days beside an all-day date, and beside a timed one every day
    /// up to the one its end falls on (an end at midnight belongs to
    /// the day before).
    static func spanDays(_ item: ItemDetail) -> [String] {
        guard let when = item.when, let first = WhenText.date(String(when.prefix(10))) else { return [] }
        let count: Int
        if item.isTimed {
            let length = max(0, item.duration ?? 0)
            count = length > 0 ? (startClock(when) + length - 1) / 1440 + 1 : 1
        } else {
            count = currentLength(item) / 1440
        }
        let calendar = WhenText.calendar
        return (0..<count).compactMap {
            calendar.date(byAdding: .day, value: $0, to: first).map(WhenText.day)
        }
    }

    /// Whole days from one `YYYY-MM-DD` to another; 0 if either is not
    /// a day.
    static func daysBetween(_ from: String, _ to: String) -> Int {
        guard let a = WhenText.date(from), let b = WhenText.date(to) else { return 0 }
        return WhenText.calendar.dateComponents([.day], from: a, to: b).day ?? 0
    }

    /// A day as the picker holds one.
    static func components(_ day: String) -> DateComponents {
        guard let date = WhenText.date(day) else { return DateComponents() }
        return WhenText.calendar.dateComponents([.calendar, .era, .year, .month, .day], from: date)
    }

    static func dayText(_ day: DateComponents) -> String? {
        guard let year = day.year, let month = day.month, let date = day.day else { return nil }
        return String(format: "%04d-%02d-%02d", year, month, date)
    }

    private func time(on when: String) -> Binding<Date> {
        Binding(
            get: { WhenText.date(when) ?? Date() },
            set: { model.setWhen(ref, when: "\(when.prefix(10))T\(WhenText.time($0))") }
        )
    }

    /// One of the two times: its glyph, then the system's time control.
    /// Each takes half the row.
    private func timeBox(_ systemImage: String, _ name: String, _ selection: Binding<Date>, note: String? = nil)
        -> some View
    {
        HStack(spacing: 8) {
            Image(systemName: systemImage).foregroundStyle(.secondary)
            DatePicker(name, selection: selection, displayedComponents: .hourAndMinute)
                .labelsHidden()
            if let note {
                Text(note).font(.footnote).foregroundStyle(.secondary)
            }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
    }

    /// The end: start + the stored length (the start itself when there
    /// is none). Picking one writes the length up to it.
    private func end(of when: String) -> Binding<Date> {
        Binding(
            get: {
                let start = WhenText.date(when) ?? Date()
                return WhenText.calendar.date(byAdding: .minute, value: max(0, item.duration ?? 0), to: start) ?? start
            },
            set: {
                let c = WhenText.calendar.dateComponents([.hour, .minute], from: $0)
                let end = (c.hour ?? 0) * 60 + (c.minute ?? 0)
                model.setDuration(ref, minutes: Self.length(from: when, toClock: end, was: item.duration))
            }
        )
    }

    /// Minutes of the day a timed `when` starts at.
    static func startClock(_ when: String) -> Int {
        let parts = when.dropFirst(11).prefix(5).split(separator: ":").compactMap { Int($0) }
        return parts.count == 2 ? parts[0] * 60 + parts[1] : 0
    }

    /// Days past the start day the end lands on.
    static func endDayOffset(_ when: String, _ duration: Int?) -> Int {
        (startClock(when) + max(0, duration ?? 0)) / 1440
    }

    /// "+1d" beside an end that is not on the start day.
    static func endDayNote(_ when: String, _ duration: Int?) -> String? {
        let days = endDayOffset(when, duration)
        return days > 0 ? "+\(days)d" : nil
    }

    /// The length an end time picked on the clock gives, the web's
    /// rule (spec/calendar-plan.md "UI shows an end time, stores a
    /// length"): the end stays on the day it was already on; an end at
    /// or before the start means the next day.
    static func length(from when: String, toClock end: Int, was duration: Int?) -> Int {
        let length = endDayOffset(when, duration) * 1440 + end - startClock(when)
        return length > 0 ? length : length + 1440
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
