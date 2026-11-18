// The item panel's planned-date row (`when` and `duration`,
// spec/calendar-plan.md), and its editor: the place row's way of
// working (`PlaceField.swift`) with the web date input's contents and
// layout (`js/web/src/WhenField.tsx`). With no date it is one box, a
// muted "Date". Once a date is set the row splits into the time row's
// two columns: the start day, then, after an arrow, the day the span
// ends on, over the end time. While that is the start day itself (no
// span) the end reads dim: it is implied, not stored. Under the pointer
// a box shows an "x": the start's takes the date off, the end's brings
// the span back onto the start day.
//
// A click on either box turns it into a text box and drops a box under
// the row, floating over the panel's rows:
//
//   - the offers: what the typed text could mean, a few at most
//     (`DateOffers`, over `WhenPhrase`, as the iOS editor reads it).
//     For the start, Today and Tomorrow until something is typed on an
//     item with no date. The end's are days only, read from the start
//     day once that is ahead of today ("friday" is the Friday the span
//     could end on), with the days it cannot reach left out.
//   - a month calendar (`MonthCalendar`: AppKit's `NSDatePicker`). It
//     shows the box's day, or the highlighted offer's while there is
//     one, so the offers are seen on the month as they are stepped
//     through. A click on a day sets it. A start day keeps the time
//     and the length, so the span moves whole; an end day is the
//     second way to write the stored `duration` (the end time under it
//     is the first): beside a timed start it keeps the end's clock
//     time, beside an all-day one the length counts whole days.
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

    private enum Side { case start, end }

    @State private var query = ""
    /// The box being edited, nil at rest.
    @State private var picking: Side?
    /// The text box has had the caret since it opened: only then is
    /// the caret leaving it a reason to shut.
    @State private var armed = false
    /// The row the arrow keys or the pointer chose; nil for the
    /// default (the first offer once something is typed).
    @State private var chosen: Int?
    /// The pointer is over the calendar: the caret leaving the box
    /// then is a click there, not a reason to shut.
    @State private var overCalendar = false
    /// The pointer is over a box at rest: its "x" shows.
    @State private var hovered: Side?
    @FocusState private var focused: Bool

    private var typed: String { query.trimmingCharacters(in: .whitespacesAndNewlines) }
    private var timed: Bool { (when?.count ?? 0) >= 16 }
    private var startDay: String? { when.map { String($0.prefix(10)) } }
    /// Days past the start day the span ends on; 0 is the start day.
    private var endOffset: Int { when.map { DateOffers.endOffset(when: $0, duration: duration) } ?? 0 }
    private var endDay: String? {
        startDay.flatMap { DateOffers.shift($0, days: endOffset) }
    }

    var body: some View {
        // Each box is outset by its inset, so 4 pt shows between the
        // two, as on the time row under this one.
        HStack(spacing: PanelBox.inset * 2 + 4) {
            if picking == .start {
                textBox(.start)
            } else {
                shownStart
            }
            if when != nil {
                if picking == .end {
                    textBox(.end)
                } else {
                    shownEnd
                }
            }
        }
        // As the place field's list: an overlay hung from the row's
        // bottom edge, taking no room in the panel, as wide as the
        // boxes (which are drawn outset).
        .overlay(alignment: .bottomLeading) {
            if let side = picking {
                Color.clear
                    .frame(height: 0)
                    .overlay(alignment: .topLeading) {
                        dropdown(side)
                            .fixedSize(horizontal: false, vertical: true)
                            .padding(.top, 6)
                    }
                    .padding(.horizontal, -PanelBox.inset)
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

    /// The start at rest: its day, or a muted "Date".
    private var shownStart: some View {
        Button {
            open(.start)
        } label: {
            HStack(alignment: .firstTextBaseline, spacing: 10) {
                PanelGlyph(image: NavArt.calendar)
                    .foregroundStyle(when == nil ? .tertiary : .secondary)
                    .frame(width: 20)
                Text(verbatim: startDay.map(DateOffers.dayLabel) ?? String(localized: "Date"))
                    .lineLimit(1)
                    .foregroundStyle(when == nil ? .tertiary : .primary)
            }
            .panelBox(filled: false)
        }
        .buttonStyle(.plain)
        .accessibilityHint(Text("Changes the date"))
        .overlay(alignment: .trailing) {
            if when != nil, hovered == .start {
                clearButton(String(localized: "Remove date")) {
                    // The engine clears the duration with it.
                    set(nil, .keep)
                }
            }
        }
        .onHover { hover(.start, $0) }
    }

    /// The end at rest: the day the span ends on, dim while that is
    /// only the start day again.
    private var shownEnd: some View {
        let label = endDay.map(DateOffers.dayLabel) ?? ""
        return Button {
            open(.end)
        } label: {
            HStack(alignment: .firstTextBaseline, spacing: 10) {
                Image(systemName: "arrow.right")
                    .foregroundStyle(endOffset == 0 ? .tertiary : .secondary)
                    .frame(width: 20)
                Text(verbatim: label)
                    .lineLimit(1)
                    .foregroundStyle(endOffset == 0 ? .tertiary : .primary)
            }
            .panelBox(filled: false)
        }
        .buttonStyle(.plain)
        .accessibilityLabel(Text("End date"))
        .accessibilityValue(Text(verbatim: label))
        .overlay(alignment: .trailing) {
            if endOffset > 0, hovered == .end {
                clearButton(String(localized: "Remove end date"), action: removeEnd)
            }
        }
        .onHover { hover(.end, $0) }
    }

    /// The "x" at a box's trailing edge.
    private func clearButton(_ label: String, action: @escaping () -> Void) -> some View {
        Button(action: action) {
            Image(systemName: "xmark")
                .font(.caption.weight(.semibold))
                .foregroundStyle(.secondary)
                .frame(width: 20, height: 20)
                .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .hoverCircle()
        .help(label)
        .accessibilityLabel(Text(verbatim: label))
    }

    private func hover(_ side: Side, _ inside: Bool) {
        if inside {
            hovered = side
        } else if hovered == side {
            hovered = nil
        }
    }

    @ViewBuilder
    private func textBox(_ side: Side) -> some View {
        HStack(alignment: .firstTextBaseline, spacing: 10) {
            Group {
                switch side {
                case .start: PanelGlyph(image: NavArt.calendar)
                case .end: Image(systemName: "arrow.right")
                }
            }
            .foregroundStyle(.secondary)
            .frame(width: 20)
            Group {
                switch side {
                case .start:
                    TextField("Type a date: fri 3pm", text: $query)
                case .end:
                    // The stored day stays in sight, dim, until typed
                    // over, as the time fields' does.
                    TextField(text: $query, prompt: Text(verbatim: endDay.map(DateOffers.dayLabel) ?? "")) {
                        Text("End date")
                    }
                }
            }
            .textFieldStyle(.plain)
            .autocorrectionDisabled()
            .focused($focused)
            .onSubmit(submit)
            .onExitCommand(perform: finish)
            .onKeyPress(.downArrow) { step(1) }
            .onKeyPress(.upArrow) { step(-1) }
        }
        .panelBox()
        // A turn after the box appears, as the place field's. Armed
        // here too: going straight from one box to the other the caret
        // may never be seen to leave.
        .onAppear {
            DispatchQueue.main.async {
                focused = true
                armed = true
            }
        }
    }

    /// The offers, then the month, in a box under the row.
    private func dropdown(_ side: Side) -> some View {
        let rows = rows
        let highlight = highlight(in: rows)
        return VStack(alignment: .leading, spacing: 0) {
            ForEach(Array(rows.enumerated()), id: \.offset) { index, row in
                DateRow(offer: row, highlighted: highlight == index) { pick(row) }
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
            MonthCalendar(day: calendarDay(rows, highlight)) { pickDay($0) }
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

    private var rows: [DateOffers.Offer] {
        guard picking == .end, let when else {
            return DateOffers.offers(
                typed: typed, current: when, today: DateOffers.today(), tomorrow: DateOffers.today(plus: 1))
        }
        return DateOffers.endOffers(typed: typed, when: when, duration: duration)
    }

    /// The row Return takes: the chosen one, else the first once
    /// something is typed.
    private func highlight(in rows: [DateOffers.Offer]) -> Int? {
        if let chosen, rows.indices.contains(chosen) { return chosen }
        return typed.isEmpty || rows.isEmpty ? nil : 0
    }

    /// What the month shows: the highlighted offer's day, else the
    /// day of the box being edited, else today.
    private func calendarDay(_ rows: [DateOffers.Offer], _ highlight: Int?) -> String {
        if let highlight { return String(rows[highlight].when.prefix(10)) }
        return (picking == .end ? endDay : startDay) ?? DateOffers.today()
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

    private func pick(_ offer: DateOffers.Offer) {
        if picking == .end {
            _ = pickEnd(String(offer.when.prefix(10)))
            return
        }
        // Going between all-day and timed the old length no longer
        // applies: an hour beside a time, a single day without one.
        let nowTimed = offer.when.count >= 16
        var change = DurationChange.keep
        if when != nil, nowTimed != timed { change = nowTimed ? .minutes(minutes: 60) : .clear }
        set(offer.when, change)
        finish()
    }

    /// A day clicked on the month. False when the box cannot take it:
    /// the month then goes back to the day it showed.
    private func pickDay(_ day: String) -> Bool {
        if picking == .end { return pickEnd(day) }
        // The time of day and the length stay: the span moves whole.
        set(day + (when.map { String($0.dropFirst(10).prefix(6)) } ?? ""), .keep)
        finish()
        return true
    }

    /// The span ends on `day`, if it can.
    private func pickEnd(_ day: String) -> Bool {
        guard let when, let startDay, let offset = DateOffers.days(from: startDay, to: day),
              let length = DateOffers.endLength(when: when, duration: duration, offset: offset)
        else { return false }
        if offset != endOffset {
            set(when, length > 0 ? .minutes(minutes: UInt32(length)) : .clear)
        }
        finish()
        return true
    }

    /// The end's "x": the span back onto the start day.
    private func removeEnd() {
        guard let when else { return }
        let length = DateOffers.sameDayLength(when: when, duration: duration)
        set(when, length > 0 ? .minutes(minutes: UInt32(length)) : .clear)
    }

    /// A box takes the caret: a fresh editor, whichever was open.
    private func open(_ side: Side) {
        query = ""
        chosen = nil
        armed = false
        overCalendar = false
        picking = side
    }

    /// Back to the row as it was: no text, no list.
    private func finish() {
        query = ""
        chosen = nil
        picking = nil
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
    // MARK: The end day (`WhenSpan`, shared with the iOS editor)

    static func dayLabel(_ day: String) -> String { WhenSpan.dayLabel(day, calendar: calendar) }

    static func days(from: String, to: String) -> Int? {
        WhenSpan.days(from: from, to: to, calendar: calendar)
    }

    static func endOffset(when: String, duration: Int?) -> Int {
        WhenSpan.endOffset(when: when, duration: duration)
    }

    static func endLength(when: String, duration: Int?, offset: Int) -> Int? {
        WhenSpan.endLength(when: when, duration: duration, offset: offset)
    }

    static func sameDayLength(when: String, duration: Int?) -> Int {
        WhenSpan.sameDayLength(when: when, duration: duration)
    }

    /// What typed text could mean as the day a span ends on: days
    /// only, read from the start day once that is ahead of today, the
    /// days the register cannot reach left out. A weekday comes with
    /// the two after it, as the start's.
    static func endOffers(typed: String, when: String, duration: Int?, now: Date = Date()) -> [Offer] {
        let startDay = String(when.prefix(10))
        var reference = now
        if let start = Agenda.start(of: startDay, calendar: calendar), start > now { reference = start }
        guard let phrase = WhenPhrase.find(in: typed, now: reference) else { return [] }
        var days = [String(phrase.when.prefix(10))]
        if namesWeekday(phrase.text) {
            days += [7, 14].compactMap { shift(days[0], days: $0) }
        }
        var offers: [Offer] = []
        for day in days where !offers.contains(where: { $0.when == day }) {
            guard let offset = self.days(from: startDay, to: day),
                  endLength(when: when, duration: duration, offset: offset) != nil
            else { continue }
            offers.append(Offer(when: day))
        }
        return Array(offers.prefix(limit))
    }
}

/// One row of the list: an offer (its name when it has one, the date
/// after it).
private struct DateRow: View {
    let offer: DateOffers.Offer
    let highlighted: Bool
    let action: () -> Void

    var body: some View {
        Button(action: action) {
            HStack(alignment: .firstTextBaseline, spacing: 8) {
                if let name = offer.name {
                    Text(verbatim: name).lineLimit(1)
                    Spacer(minLength: 8)
                    Text(verbatim: Self.text(offer.when))
                        .foregroundStyle(highlighted ? .white.opacity(0.8) : .secondary)
                        .lineLimit(1)
                } else {
                    Text(verbatim: Self.text(offer.when)).lineLimit(1)
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

/// A month of days with one selected: AppKit's graphical date picker.
/// `day` is what it shows; a click on a day is reported once the mouse
/// is up. A day the caller turns down is not kept: the month goes back
/// to `day`.
private struct MonthCalendar: NSViewRepresentable {
    /// The day selected, `YYYY-MM-DD`.
    let day: String
    /// Take the day clicked; false when it cannot be taken.
    let pick: (_ day: String) -> Bool

    func makeCoordinator() -> Coordinator { Coordinator(day: day, pick: pick) }

    func makeNSView(context: Context) -> NSDatePicker {
        let picker = Picker()
        picker.datePickerStyle = .clockAndCalendar
        picker.datePickerElements = [.yearMonthDay]
        picker.datePickerMode = .single
        picker.calendar = DateOffers.calendar
        picker.timeZone = .current
        picker.locale = .current
        picker.isBezeled = false
        picker.isBordered = false
        picker.drawsBackground = false
        picker.target = context.coordinator
        picker.action = #selector(Coordinator.changed(_:))
        Coordinator.show(day, on: picker)
        return picker
    }

    func updateNSView(_ picker: NSDatePicker, context: Context) {
        context.coordinator.pick = pick
        context.coordinator.day = day
        // Not under a click: what it shows then is the user's.
        guard context.coordinator.pending == nil else { return }
        Coordinator.show(day, on: picker)
    }

    /// The picker, never the key view: a click on it leaves the caret
    /// in the text box over it, whose losing the caret shuts the
    /// editor.
    private final class Picker: NSDatePicker {
        override var acceptsFirstResponder: Bool { false }
    }

    final class Coordinator: NSObject {
        var pick: (String) -> Bool
        /// The day the caller last asked for.
        var day: String
        /// The day clicked, not reported yet, and the picker it is on.
        private(set) var pending: (day: String, picker: NSDatePicker)?

        init(day: String, pick: @escaping (String) -> Bool) {
            self.day = day
            self.pick = pick
        }

        /// Noon of the day: an hour either way at a clock change stays
        /// inside the day meant. Left alone when it already shows the
        /// day, so the month the user has turned to is not turned back.
        static func show(_ day: String, on picker: NSDatePicker) {
            let calendar = DateOffers.calendar
            guard DateOffers.day(picker.dateValue) != day,
                  let start = Agenda.start(of: day, calendar: calendar),
                  let noon = calendar.date(bySettingHour: 12, minute: 0, second: 0, of: start)
            else { return }
            picker.dateValue = noon
        }

        /// The picker's action. Held until the mouse is up and the run
        /// loop is back out of the click, so a pick is one write.
        @objc func changed(_ sender: NSDatePicker) {
            pending = (DateOffers.day(sender.dateValue), sender)
            settle(after: 0)
        }

        private func settle(after delay: TimeInterval) {
            NSObject.cancelPreviousPerformRequests(withTarget: self, selector: #selector(settled), object: nil)
            perform(#selector(settled), with: nil, afterDelay: delay, inModes: [.default])
        }

        @objc private func settled() {
            guard NSEvent.pressedMouseButtons & 1 == 0 else { return settle(after: 0.05) }
            guard let picked = pending else { return }
            pending = nil
            if !pick(picked.day) {
                NSSound.beep()
                Self.show(day, on: picked.picker)
            }
        }
    }
}
