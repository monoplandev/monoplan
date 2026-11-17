// The item panel's planned-date row (`when` and `duration`,
// spec/calendar-plan.md), and its editor: the place row's way of
// working (`PlaceField.swift`) with the web date input's contents
// (`js/web/src/WhenField.tsx`). At rest it is the day on one line, its
// length at the trailing edge, or a muted "Date" when there is none. A
// click turns the row into a text box and drops a box under it,
// floating over the panel's rows:
//
//   - the offers: what the typed text could mean, a few at most
//     (`DateOffers`, over `WhenPhrase`, as the iOS editor reads it).
//     Today and Tomorrow until something is typed on an item with no
//     date; "Remove date" on one with a date.
//   - a month calendar (`RangeCalendar`: AppKit's `NSDatePicker` in its
//     range mode). It shows the item's date as a run of days, or the
//     highlighted offer's day while there is one, so the offers are
//     seen on the month as they are stepped through. A click on a day
//     sets it; a drag over several sets the span.
//
// Typing highlights the first offer; Return takes the highlighted row,
// the arrow keys move the highlight; Escape, or the caret leaving the
// box, shuts it with the date as it was. Every pick is written at once
// and shuts the editor.

import AppKit
import MonoplanCore
import MonoplanMacKit
import SwiftUI

struct DateField: View {
    /// `YYYY-MM-DD` or `YYYY-MM-DDTHH:MM`, nil when unset.
    let when: String?
    /// Minutes, nil when unset.
    let duration: Int?
    /// Write the date, or clear it with nil, and what becomes of the
    /// duration in the same call (`Engine.setWhen`).
    let set: (String?, DurationChange) -> Void

    @State private var query = ""
    @State private var picking = false
    /// The text box has had the caret since it opened: only then is
    /// the caret leaving it a reason to shut.
    @State private var armed = false
    /// The row the arrow keys or the pointer chose; nil for the
    /// default (the first offer once something is typed).
    @State private var chosen: Int?
    /// The pointer is over the calendar: the caret leaving the box
    /// then is a click there, not a reason to shut.
    @State private var overCalendar = false
    @FocusState private var focused: Bool

    private var typed: String { query.trimmingCharacters(in: .whitespacesAndNewlines) }
    private var timed: Bool { (when?.count ?? 0) >= 16 }

    var body: some View {
        Group {
            if picking {
                // As the place field's list: an overlay hung from the
                // box's bottom edge, taking no room in the panel.
                textBox.overlay(alignment: .bottomLeading) {
                    Color.clear
                        .frame(height: 0)
                        .overlay(alignment: .topLeading) {
                            dropdown
                                .fixedSize(horizontal: false, vertical: true)
                                .padding(.top, 6)
                        }
                }
            } else {
                shown
            }
        }
        .onChange(of: query) { chosen = nil }
        .onChange(of: focused) {
            if focused {
                armed = true
            } else if armed, !overCalendar {
                finish()
            }
        }
    }

    // MARK: Rows

    /// The date at rest: its day, or a muted "Date".
    private var shown: some View {
        Button {
            picking = true
        } label: {
            HStack(alignment: .firstTextBaseline, spacing: 10) {
                PanelGlyph(image: NavArt.calendar)
                    .foregroundStyle(when == nil ? .tertiary : .secondary)
                    .frame(width: 20)
                Text(verbatim: when.map { ItemPanel.dayText(String($0.prefix(10))) } ?? String(localized: "Date"))
                    .lineLimit(1)
                    .foregroundStyle(when == nil ? .tertiary : .primary)
                if let note = restNote {
                    Spacer(minLength: 8)
                    Text(verbatim: note).font(.footnote).foregroundStyle(.secondary).lineLimit(1)
                }
            }
            .panelBox(filled: false)
        }
        .buttonStyle(.plain)
        .accessibilityHint(Text("Changes the date"))
    }

    /// Beside an all-day date that spans several days, how many. A
    /// timed date's times are on the panel's row under this one.
    private var restNote: String? {
        guard when != nil, !timed else { return nil }
        let days = DateOffers.spanDays(when: when, duration: duration)
        return days >= 2 ? String(localized: "\(days) days") : nil
    }

    private var textBox: some View {
        HStack(alignment: .firstTextBaseline, spacing: 10) {
            PanelGlyph(image: NavArt.calendar)
                .foregroundStyle(.secondary)
                .frame(width: 20)
            TextField("Type a date: fri 3pm", text: $query)
                .textFieldStyle(.plain)
                .autocorrectionDisabled()
                .focused($focused)
                .onSubmit(submit)
                .onExitCommand(perform: finish)
                .onKeyPress(.downArrow) { step(1) }
                .onKeyPress(.upArrow) { step(-1) }
        }
        .panelBox()
        // A turn after the box appears, as the place field's.
        .onAppear { DispatchQueue.main.async { focused = true } }
    }

    /// The offers, then the month, in a box under the text box.
    private var dropdown: some View {
        let rows = rows
        let highlight = highlight(in: rows)
        return VStack(alignment: .leading, spacing: 0) {
            ForEach(Array(rows.enumerated()), id: \.offset) { index, row in
                DateRow(row: row, highlighted: highlight == index) { pick(row) }
                    .onHover { if $0 { chosen = index } }
            }
            if rows.isEmpty, !typed.isEmpty {
                Text("No date in that yet")
                    .foregroundStyle(.secondary)
                    .padding(.horizontal, 8)
                    .padding(.vertical, 4)
            }
            if !rows.isEmpty || !typed.isEmpty {
                Divider().padding(.vertical, 4)
            }
            RangeCalendar(day: calendarSpan(rows, highlight).day, days: calendarSpan(rows, highlight).days) {
                pickSpan(day: $0, days: $1)
            }
            .fixedSize()
            .frame(maxWidth: .infinity)
            .onHover { overCalendar = $0 }
            .accessibilityLabel(Text("Calendar"))
        }
        .padding(4)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(Color(nsColor: .controlBackgroundColor), in: RoundedRectangle(cornerRadius: 8, style: .continuous))
        .overlay(RoundedRectangle(cornerRadius: 8, style: .continuous).strokeBorder(.separator))
        .shadow(color: .black.opacity(0.18), radius: 10, y: 4)
    }

    // MARK: Offers

    private var rows: [DateRow.Row] {
        var rows: [DateRow.Row] = []
        // Over a set date the list opens on the way to take it off.
        if when != nil, typed.isEmpty { rows.append(.remove) }
        rows += DateOffers.offers(
            typed: typed, current: when, today: DateOffers.today(), tomorrow: DateOffers.today(plus: 1)
        ).map(DateRow.Row.offer)
        return rows
    }

    /// The row Return takes: the chosen one, else the first once
    /// something is typed.
    private func highlight(in rows: [DateRow.Row]) -> Int? {
        if let chosen, rows.indices.contains(chosen) { return chosen }
        return typed.isEmpty || rows.isEmpty ? nil : 0
    }

    /// What the month shows: the highlighted offer's day, else the
    /// item's own span, else today.
    private func calendarSpan(_ rows: [DateRow.Row], _ highlight: Int?) -> (day: String, days: Int) {
        if let highlight, case .offer(let offer) = rows[highlight] {
            return (String(offer.when.prefix(10)), 1)
        }
        if let when {
            return (String(when.prefix(10)), DateOffers.spanDays(when: when, duration: duration))
        }
        return (DateOffers.today(), 1)
    }

    private func step(_ by: Int) -> KeyPress.Result {
        let rows = rows
        guard !rows.isEmpty else { return .ignored }
        let from = highlight(in: rows)
        chosen = max(0, min(rows.count - 1, from.map { $0 + by } ?? (by > 0 ? 0 : rows.count - 1)))
        return .handled
    }

    private func submit() {
        let rows = rows
        if let highlight = highlight(in: rows) {
            pick(rows[highlight])
        } else if typed.isEmpty {
            finish()
        }
        // Text that names no date stays up to be fixed.
    }

    private func pick(_ row: DateRow.Row) {
        switch row {
        case .remove:
            // The engine clears the duration with it.
            set(nil, .keep)
        case .offer(let offer):
            // Going between all-day and timed the old length no longer
            // applies: an hour beside a time, a single day without one.
            let nowTimed = offer.when.count >= 16
            var change = DurationChange.keep
            if when != nil, nowTimed != timed { change = nowTimed ? .minutes(minutes: 60) : .clear }
            set(offer.when, change)
        }
        finish()
    }

    /// A day clicked on the month (`days` 1) or a run of them dragged
    /// over. The time of day stays; a run is the span's whole days,
    /// with a timed date's end kept on the clock on the last one.
    private func pickSpan(day: String, days: Int) {
        guard let when, timed else {
            let change: DurationChange =
                days > 1 ? .minutes(minutes: UInt32(days * 1440)) : (when == nil ? .keep : .clear)
            set(day, change)
            return finish()
        }
        let start = (Int(when.dropFirst(11).prefix(2)) ?? 0) * 60 + (Int(when.dropFirst(14).prefix(2)) ?? 0)
        let length = max(0, duration ?? 0)
        let change: DurationChange
        if days > 1 {
            let end = (start + length) % 1440
            change = .minutes(minutes: UInt32((days - 1) * 1440 + end - start))
        } else if length >= 1440 {
            // Whole days come off the length; the end's time stays.
            change = .minutes(minutes: UInt32(length % 1440 == 0 ? 60 : length % 1440))
        } else {
            change = .keep
        }
        set(day + when.dropFirst(10).prefix(6), change)
        finish()
    }

    /// Back to the row as it was: no text, no list.
    private func finish() {
        query = ""
        chosen = nil
        picking = false
        armed = false
        overCalendar = false
        focused = false
    }
}

/// What typed text could mean as a date, likeliest first: the iOS
/// editor's rule (`ItemDateEditor.offers`) plus the web's repeats
/// (`dateSuggest.ts`).
enum DateOffers {
    struct Offer: Equatable {
        /// `YYYY-MM-DD` or `YYYY-MM-DDTHH:MM`.
        let when: String
        /// The standing ones' name ("Today").
        var name: String?
    }

    /// Rows shown at most.
    static let limit = 5
    /// The longest span the month writes, in days.
    static let maxDays = 366

    /// The standing two (Today, Tomorrow: both with nothing typed on an
    /// item with no date, then only those the text is the start of),
    /// each keeping the item's time of day; then what the text reads
    /// as. A weekday that is the only reading comes with the two after
    /// it ("friday": the next three Fridays).
    static func offers(typed: String, current: String?, today: String, tomorrow: String) -> [Offer] {
        let time = current.map { $0.count >= 16 ? String($0.dropFirst(10).prefix(6)) : "" } ?? ""
        let word = typed.lowercased()
        var offers = [(String(localized: "Today"), today), (String(localized: "Tomorrow"), tomorrow)]
            .filter { word.isEmpty ? current == nil : $0.0.lowercased().hasPrefix(word) }
            .map { Offer(when: $0.1 + time, name: $0.0) }
        guard let phrase = WhenPhrase.find(in: typed) else { return offers }
        var readings = phrase.readings(over: current, today: today)
        if readings.count == 1, namesWeekday(phrase.text) {
            readings += [7, 14].compactMap { shift(readings[0], days: $0) }
        }
        for when in readings where !offers.contains(where: { $0.when == when }) {
            offers.append(Offer(when: when))
        }
        return Array(offers.prefix(limit))
    }

    /// The days an item's date covers: whole days beside an all-day
    /// date, and beside a timed one every day up to the one its end
    /// falls on (an end at midnight belongs to the day before).
    static func spanDays(when: String?, duration: Int?) -> Int {
        guard let when else { return 1 }
        let minutes = max(0, duration ?? 0)
        guard when.count >= 16 else { return min(maxDays, max(1, (minutes + 1439) / 1440)) }
        let start = (Int(when.dropFirst(11).prefix(2)) ?? 0) * 60 + (Int(when.dropFirst(14).prefix(2)) ?? 0)
        return min(maxDays, max(1, (start + minutes + 1439) / 1440))
    }

    /// The Gregorian calendar the register is written in, at the
    /// device's time zone and with its week (first day, names).
    static var calendar: Calendar {
        var calendar = Calendar(identifier: .gregorian)
        calendar.locale = .current
        calendar.timeZone = .current
        return calendar
    }

    /// `YYYY-MM-DD`.
    static func day(_ date: Date) -> String {
        let c = calendar.dateComponents([.year, .month, .day], from: date)
        return String(format: "%04d-%02d-%02d", c.year ?? 0, c.month ?? 0, c.day ?? 0)
    }

    static func today(plus days: Int = 0, now: Date = Date()) -> String {
        day(calendar.date(byAdding: .day, value: days, to: now) ?? now)
    }

    /// `when` moved on by whole days, its time of day kept.
    static func shift(_ when: String, days: Int) -> String? {
        let calendar = calendar
        guard let date = Agenda.start(of: String(when.prefix(10)), calendar: calendar),
              let moved = calendar.date(byAdding: .day, value: days, to: date)
        else { return nil }
        return day(moved) + when.dropFirst(10)
    }

    /// The phrase has a weekday's name in it, in either grammar.
    static func namesWeekday(_ phrase: String) -> Bool {
        fold(phrase).split(whereSeparator: { !$0.isLetter }).contains { weekdays.contains(String($0)) }
    }

    /// Case and accents folded, so "miercoles" finds "miércoles".
    private static func fold(_ text: String) -> String {
        text.folding(options: [.diacriticInsensitive, .caseInsensitive], locale: nil).lowercased()
    }

    /// Weekday names, English and Spanish, and English's three-letter
    /// ones. Spanish's short ones are left out: "mar" is also March.
    private static let weekdays: Set<String> = {
        var words = Set(["mon", "tue", "tues", "wed", "thu", "thur", "thurs", "fri", "sat", "sun"])
        for identifier in ["en", "es"] {
            var calendar = Calendar(identifier: .gregorian)
            calendar.locale = Locale(identifier: identifier)
            words.formUnion(calendar.weekdaySymbols.map(fold))
        }
        return words
    }()
}

/// One row of the list: "Remove date", or an offer (its name when it
/// has one, the date after it).
private struct DateRow: View {
    enum Row {
        /// Take the set date off the item.
        case remove
        case offer(DateOffers.Offer)
    }

    let row: Row
    let highlighted: Bool
    let action: () -> Void

    var body: some View {
        Button(action: action) {
            HStack(alignment: .firstTextBaseline, spacing: 8) {
                switch row {
                case .remove:
                    Image(systemName: "xmark")
                        .foregroundStyle(highlighted ? .white : .secondary)
                        .frame(width: 16)
                    Text("Remove date").lineLimit(1)
                case .offer(let offer):
                    if let name = offer.name {
                        Text(verbatim: name).lineLimit(1)
                        Spacer(minLength: 8)
                        Text(verbatim: Self.text(offer.when))
                            .foregroundStyle(highlighted ? .white.opacity(0.8) : .secondary)
                            .lineLimit(1)
                    } else {
                        Text(verbatim: Self.text(offer.when)).lineLimit(1)
                    }
                }
                Spacer(minLength: 0)
            }
            .foregroundStyle(highlighted ? .white : .primary)
            .padding(.horizontal, 8)
            .padding(.vertical, 4)
            .frame(maxWidth: .infinity, alignment: .leading)
            .background(
                highlighted ? Color.accentColor : .clear,
                in: RoundedRectangle(cornerRadius: 5, style: .continuous))
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
    }

    /// "Friday 20 Nov", with its time after it when it has one.
    static func text(_ when: String) -> String {
        let day = ItemPanel.dayText(String(when.prefix(10)))
        return ItemPanel.timeText(when).map { "\(day) \($0)" } ?? day
    }
}

/// A month of days with a run of them selected: AppKit's graphical
/// date picker in its range mode (`NSDatePicker.Mode.range`, which only
/// that style has). `day` and `days` are what it shows; a click on a
/// day, or a drag over several, is reported once the mouse is up.
private struct RangeCalendar: NSViewRepresentable {
    /// The first day selected, `YYYY-MM-DD`.
    let day: String
    /// How many days the selection runs, from 1.
    let days: Int
    let pick: (_ day: String, _ days: Int) -> Void

    func makeCoordinator() -> Coordinator { Coordinator(pick: pick) }

    func makeNSView(context: Context) -> NSDatePicker {
        let picker = Picker()
        picker.datePickerStyle = .clockAndCalendar
        picker.datePickerElements = [.yearMonthDay]
        picker.datePickerMode = .range
        picker.calendar = DateOffers.calendar
        picker.timeZone = .current
        picker.locale = .current
        picker.isBezeled = false
        picker.isBordered = false
        picker.drawsBackground = false
        picker.target = context.coordinator
        picker.action = #selector(Coordinator.changed(_:))
        show(on: picker)
        return picker
    }

    func updateNSView(_ picker: NSDatePicker, context: Context) {
        context.coordinator.pick = pick
        // Not under a drag: what it shows then is the user's.
        guard context.coordinator.pending == nil else { return }
        show(on: picker)
    }

    /// Noon of the first day, and whole days after it: an hour either
    /// way at a clock change stays inside the day meant. Left alone
    /// when it already shows this, so the month the user has turned to
    /// is not turned back.
    private func show(on picker: NSDatePicker) {
        let calendar = DateOffers.calendar
        guard let start = Agenda.start(of: day, calendar: calendar),
              let noon = calendar.date(bySettingHour: 12, minute: 0, second: 0, of: start)
        else { return }
        let interval = TimeInterval((max(1, days) - 1) * 86400)
        if Coordinator.span(of: picker) != (day, max(1, days)) {
            picker.dateValue = noon
            picker.timeInterval = interval
        }
    }

    /// The picker, never the key view: a click on it leaves the caret
    /// in the text box over it, whose losing the caret shuts the
    /// editor.
    private final class Picker: NSDatePicker {
        override var acceptsFirstResponder: Bool { false }
    }

    final class Coordinator: NSObject {
        var pick: (String, Int) -> Void
        /// The selection changed and has not been reported yet.
        private(set) var pending: (day: String, days: Int)?

        init(pick: @escaping (String, Int) -> Void) {
            self.pick = pick
        }

        /// What the picker has selected: its first day and how many.
        static func span(of picker: NSDatePicker) -> (String, Int) {
            let days = Int((picker.timeInterval / 86400).rounded()) + 1
            return (DateOffers.day(picker.dateValue), min(DateOffers.maxDays, max(1, days)))
        }

        /// The picker's action, sent for every change, a drag's each
        /// step included. Held until the mouse is up and the run loop
        /// is back out of the drag, so a span is one write.
        @objc func changed(_ sender: NSDatePicker) {
            pending = Self.span(of: sender)
            settle(after: 0)
        }

        private func settle(after delay: TimeInterval) {
            NSObject.cancelPreviousPerformRequests(withTarget: self, selector: #selector(settled), object: nil)
            perform(#selector(settled), with: nil, afterDelay: delay, inModes: [.default])
        }

        @objc private func settled() {
            guard NSEvent.pressedMouseButtons & 1 == 0 else { return settle(after: 0.05) }
            guard let span = pending else { return }
            pending = nil
            pick(span.day, span.days)
        }
    }
}
