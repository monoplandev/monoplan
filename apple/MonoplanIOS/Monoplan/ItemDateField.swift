// The item sheet's planned date (`when` and `duration`,
// spec/calendar-plan.md). On the item it is one row (`ItemDateField`),
// or, while a date read out of the title waits on an answer, the
// question in its place (`suggestion`);
// its button hands the whole sheet over to the editor
// (`ItemDateEditor`): the date's boxes (start day, end day, start time,
// end time, as the Mac panel's rows). A tap on a box brings up its
// picker as a half sheet, as a keyboard comes up: a month calendar
// with a text box that reads a typed date ("fri 3pm") and offers what
// it means, or a time wheel; the sheet's tick writes what was picked
// (`item set --when / --duration`). The editor's own tick gives the
// sheet back to the item; its cross first undoes what the editor
// changed. Dates are floating wall-clock strings, never instants.

import MonoplanCore
import MonoplanMacKit
import SwiftUI

/// The date as a row on the item: its value, or "Date" when unset,
/// muted. With no date the whole row opens the editor and there is no
/// button; once one is set a small round pencil at the trailing edge
/// is the only way in, the row itself display, so changing a date
/// takes a deliberate tap.
///
/// A pending date suggestion (`ItemInfo.pendingSuggestion`: an open
/// item with no date whose title named one) takes the row over, as the
/// web's `WhenSuggestion.tsx` takes the date input's place: the same
/// row in orange, "Suggested: Friday 20 Nov at 08:30", so it is never
/// taken for a set date, with two answers at the trailing edge. The
/// tick makes it the item's date; the cross drops it, and the plain
/// empty row comes back.
struct ItemDateField: View {
    let item: ItemInfo
    let edit: () -> Void
    /// The answer to the pending suggestion: its value, and yes or no.
    let answer: (String, Bool) -> Void

    var body: some View {
        HStack(spacing: 8) {
            if let suggested = item.pendingSuggestion {
                suggestion(suggested)
            } else {
                if let when = item.when {
                    FieldRow("calendar", text: ItemView.rowWhenText(when), note: ItemView.lengthText(item))
                } else {
                    FieldRow("calendar", text: String(localized: "Date"), empty: true)
                }
                if item.when != nil {
                    Spacer(minLength: 8)
                    EditBubble(what: String(localized: "date"), action: edit)
                        .disabled(item.binnedAt != nil)
                }
            }
        }
        .frame(maxWidth: .infinity, minHeight: 36, alignment: .leading)
        .addsOnTap(
            item.when == nil && item.pendingSuggestion == nil && item.binnedAt == nil, what: String(localized: "date"), action: edit)
    }

    @ViewBuilder
    private func suggestion(_ suggested: String) -> some View {
        let label = String(localized: "Suggested: \(ItemView.whenText(suggested))")
        // `FieldRow`'s anatomy, in orange.
        HStack(alignment: .firstTextBaseline, spacing: 12) {
            Image(systemName: "calendar").frame(width: 24)
            Text(label)
        }
        .foregroundStyle(.orange)
        .accessibilityElement(children: .ignore)
        .accessibilityLabel(label)
        Spacer(minLength: 8)
        answerBubble("checkmark", String(localized: "Use suggested date")) { answer(suggested, true) }
        answerBubble("xmark", String(localized: "Dismiss suggestion")) { answer(suggested, false) }
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
/// open: a header (cross, the item's name, tick), then the date's
/// boxes. What is written is written as it is confirmed, so the tick
/// only closes; the cross puts back the date and length the editor
/// opened with, then closes.
///
/// The boxes are laid out as the Mac panel's date and time rows
/// (`DateField.swift`) and the web's (`WhenField.tsx`): with no date,
/// one box, "Date"; once one is set, the start day and, after an arrow,
/// the day the span ends on (dim while that is only the start day
/// again: implied, not stored), and under them the start time ("All
/// day" while it has none) and, beside a time, the end time.
///
/// A tap on a box makes it the selected one, outlined, and brings up
/// its picker as a half sheet from the bottom, the way a keyboard
/// comes up for a text field: a month calendar for a day (`DaySheet`),
/// a time wheel for a time (`TimeSheet`). What is picked there is a
/// draft until the sheet's tick, which writes it and puts the sheet
/// away; its cross, or a drag down, drops it. The boxes stay in reach
/// over the sheet, so a tap on another one moves the picker to it (the
/// draft dropped). The selected box shows an "x" when it has something
/// to take off, written at once: the start day's clears the date, the
/// end day's brings the span back onto the start day, the start
/// time's makes the date all-day again. With no date the editor opens
/// on the start day's picker.
struct ItemDateEditor: View {
    @EnvironmentObject private var model: AppModel
    let ref: ItemRef
    let item: ItemInfo
    let done: () -> Void
    /// What the item had when the editor opened, for the cross.
    @State private var original: Original
    /// The selected box, its picker up; nil with the sheet away.
    @State private var active: Box?
    /// The box the sheet draws: the selected one, and still the last
    /// one while the sheet slides away.
    @State private var sheetBox = Box.start

    enum Box { case start, end, startTime, endTime }

    struct Original: Equatable {
        let when: String?
        let duration: Int?
        let event: Bool
    }

    private var current: Original {
        Original(when: item.when, duration: item.duration, event: item.event)
    }

    init(ref: ItemRef, item: ItemInfo, done: @escaping () -> Void) {
        self.ref = ref
        self.item = item
        self.done = done
        _original = State(
            initialValue: Original(when: item.when, duration: item.duration, event: item.event))
    }

    private func cancel() {
        active = nil
        if current != original {
            model.setWhen(
                ref, when: original.when,
                duration: original.duration.map { .minutes(minutes: UInt32(clamping: $0)) } ?? .clear)
            // After the date: an event needs one, and clearing the
            // date took the flag with it.
            if original.when != nil { model.setEvent(ref, event: original.event) }
        }
        done()
    }

    private func finish() {
        active = nil
        done()
    }

    var body: some View {
        ScrollView {
            editor.padding()
        }
        .scrollBounceBehavior(.basedOnSize)
        .editorHeader(title: String(localized: "Date"), itemTitle: item.text, cancel: cancel, done: finish)
        .onAppear {
            // No date: straight to picking one.
            if item.when == nil { select(.start) }
        }
        .onChange(of: item.when) {
            // Whatever changed it, here or on another device: a box
            // that is no longer there cannot be the selected one.
            guard let active, active != .start else { return }
            if item.when == nil || (active == .endTime && !item.isTimed) { self.active = nil }
        }
        .sheet(isPresented: Binding(get: { active != nil }, set: { if !$0 { active = nil } })) {
            NavigationStack {
                picker(sheetBox)
            }
            // A box's picker starts from that box's value.
            .id(sheetBox)
            .presentationDetents(Self.isDay(sheetBox) ? [.height(540), .large] : [.height(320)])
            // The boxes over the sheet still take taps.
            .presentationBackgroundInteraction(.enabled)
        }
    }

    private static func isDay(_ box: Box) -> Bool { box == .start || box == .end }

    private var startDay: String? { item.when.map { String($0.prefix(10)) } }
    /// Days past the start day the span ends on; 0 is the start day.
    private var endOffset: Int { item.when.map { WhenSpan.endOffset(when: $0, duration: item.duration) } ?? 0 }
    private var endDay: String? {
        guard let startDay, let first = WhenText.date(startDay),
              let last = WhenText.calendar.date(byAdding: .day, value: endOffset, to: first)
        else { return nil }
        return WhenText.day(last)
    }

    // MARK: Boxes

    private var editor: some View {
        VStack(alignment: .leading, spacing: 12) {
            VStack(spacing: 8) {
                HStack(spacing: 8) {
                    dateBox(.start)
                    if item.when != nil { dateBox(.end) }
                }
                if let when = item.when {
                    HStack(spacing: 8) {
                        startTimeBox(when)
                        if item.isTimed {
                            endTimeBox(when)
                        } else {
                            // The start keeps its half of the row.
                            Color.clear.frame(maxWidth: .infinity, maxHeight: 1)
                        }
                    }
                }
            }
            if item.when != nil {
                Divider()
                // What the date means: something to do by then, or
                // something that happens then (spec/events-plan.md).
                Toggle(isOn: Binding(
                    get: { item.event },
                    set: { model.setEvent(ref, event: $0) }
                )) {
                    VStack(alignment: .leading, spacing: 2) {
                        Text("Event")
                        Text("Goes by when its day passes, instead of going overdue.")
                            .font(.footnote)
                            .foregroundStyle(.secondary)
                    }
                }
                .frame(minHeight: 36)
            }
        }
    }

    /// One box: its glyph, what it says, and, while it is the selected
    /// one, an "x" when there is something to take off. A tap anywhere
    /// on it selects it.
    private func fieldBox<Content: View>(
        _ box: Box, _ systemImage: String, clear: (label: String, action: () -> Void)? = nil,
        @ViewBuilder content: () -> Content
    ) -> some View {
        let shape = RoundedRectangle(cornerRadius: 10, style: .continuous)
        return HStack(spacing: 8) {
            Image(systemName: systemImage)
                .foregroundStyle(.secondary)
                .frame(width: 20)
            content().lineLimit(1).frame(maxWidth: .infinity, alignment: .leading)
            if active == box, let clear {
                Button(action: clear.action) {
                    Image(systemName: "xmark.circle.fill").foregroundStyle(.tertiary)
                }
                .buttonStyle(.plain)
                .accessibilityLabel(clear.label)
            }
        }
        .padding(.horizontal, 12)
        .frame(maxWidth: .infinity, minHeight: 44)
        .background(.quaternary.opacity(0.5), in: shape)
        .overlay {
            if active == box { shape.strokeBorder(Color.accentColor, lineWidth: 1.5) }
        }
        .contentShape(shape)
        .onTapGesture { select(box) }
        .accessibilityAddTraits(active == box ? [.isButton, .isSelected] : .isButton)
    }

    /// A day's box, the start's or the end's.
    private func dateBox(_ box: Box) -> some View {
        let isEnd = box == .end
        let day = isEnd ? endDay : startDay
        let dim = day == nil || (isEnd && endOffset == 0)
        var clear: (label: String, action: () -> Void)?
        if isEnd {
            if endOffset > 0 { clear = (String(localized: "Remove end date"), removeEnd) }
        } else if item.when != nil {
            clear = (String(localized: "Clear date"), clearDate)
        }
        return fieldBox(box, isEnd ? "arrow.right" : "calendar", clear: clear) {
            Text(verbatim: day.map(WhenText.dayLabel) ?? String(localized: "Date"))
                .foregroundStyle(dim ? .tertiary : .primary)
        }
    }

    private func startTimeBox(_ when: String) -> some View {
        fieldBox(
            .startTime, "clock",
            clear: item.isTimed ? (String(localized: "Clear time"), clearTime) : nil
        ) {
            if item.isTimed {
                Text(verbatim: String(when.dropFirst(11).prefix(5))).monospacedDigit()
            } else {
                Text("All day").foregroundStyle(.tertiary)
            }
        }
    }

    /// The end time: start + the stored length, a muted "End" while
    /// there is none.
    private func endTimeBox(_ when: String) -> some View {
        fieldBox(.endTime, "arrow.right") {
            if let start = WhenSpan.clock(of: when), let length = item.duration, length > 0 {
                let end = (start + length) % WhenSpan.day
                Text(verbatim: String(format: "%02d:%02d", end / 60, end % 60)).monospacedDigit()
            } else {
                Text("End").foregroundStyle(.tertiary)
            }
        }
    }

    /// A tap on a box: it is the selected one and the sheet is its
    /// picker.
    private func select(_ box: Box) {
        sheetBox = box
        active = box
    }

    private func clearDate() {
        active = nil
        // The engine clears the duration with it.
        model.setWhen(ref, when: nil)
    }

    /// All-day again. The length is kept, so a time put back brings
    /// the end back with it.
    private func clearTime() {
        guard let startDay else { return }
        active = nil
        model.setWhen(ref, when: startDay)
    }

    /// The end day's "x": the span back onto the start day.
    private func removeEnd() {
        guard let when = item.when else { return }
        active = nil
        let length = WhenSpan.sameDayLength(when: when, duration: item.duration)
        model.setDuration(ref, minutes: length > 0 ? length : nil)
    }

    // MARK: Pickers

    /// The sheet for a box: its picker, started from the box's value,
    /// and what its tick writes.
    @ViewBuilder
    private func picker(_ box: Box) -> some View {
        let close = { active = nil }
        switch box {
        case .start:
            DaySheet(
                title: String(localized: "Date"), initial: startDay ?? WhenText.today(), range: nil,
                offers: {
                    Self.offers(
                        typed: $0, current: item.when, today: WhenText.today(), tomorrow: WhenText.today(plus: 1))
                },
                cancel: close
            ) { when, offered in
                setStart(when, offered: offered)
                close()
            }
        case .end:
            DaySheet(
                title: String(localized: "End date"), initial: endDay ?? WhenText.today(), range: endRange,
                offers: { typed in
                    item.when.map { Self.endOffers(typed: typed, when: $0, duration: item.duration) } ?? []
                },
                cancel: close
            ) { when, _ in
                setEnd(String(when.prefix(10)))
                close()
            }
        case .startTime:
            TimeSheet(title: String(localized: "Start"), initial: startTime, cancel: close) {
                setStartTime($0)
                close()
            }
        case .endTime:
            TimeSheet(title: String(localized: "End"), initial: endTime, cancel: close) {
                setEndTime($0)
                close()
            }
        }
    }

    /// One offer in a day's sheet: a date, and for the standing ones
    /// their name ("Today").
    struct Offer: Equatable {
        let when: String
        var name: String?
    }

    /// The start's offers. The standing two (Today, Tomorrow: both
    /// with nothing typed on an item with no date, none on one with a
    /// date, then only those the text is the start of), each keeping
    /// the item's time of day as a calendar pick does; then what the
    /// text reads as, less any date already offered.
    static func offers(typed: String, current: String?, today: String, tomorrow: String) -> [Offer] {
        let time = current.map { $0.count >= 16 ? String($0.dropFirst(10).prefix(6)) : "" } ?? ""
        let word = typed.lowercased()
        var offers = [(String(localized: "Today"), today), (String(localized: "Tomorrow"), tomorrow)]
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

    /// The end's offer: the day the text names, read from the start
    /// day once that is ahead of today ("friday" is the Friday the
    /// span could end on), and only if the span can end there.
    static func endOffers(typed: String, when: String, duration: Int?, now: Date = Date()) -> [Offer] {
        let startDay = String(when.prefix(10))
        var reference = now
        if let start = WhenText.date(startDay), start > now { reference = start }
        guard let phrase = WhenPhrase.find(in: typed, now: reference) else { return [] }
        let day = String(phrase.when.prefix(10))
        guard let offset = WhenSpan.days(from: startDay, to: day, calendar: WhenText.calendar),
              WhenSpan.endLength(when: when, duration: duration, offset: offset) != nil
        else { return [] }
        return [Offer(when: day)]
    }

    /// The start day's sheet confirmed. An offer is a whole date:
    /// going between all-day and timed the old length no longer
    /// applies (an hour beside a time, a single day without one). A
    /// day off the calendar keeps the time of day and the length, so
    /// the span moves whole.
    private func setStart(_ picked: String, offered: Bool) {
        guard let when = item.when else { return model.setWhen(ref, when: picked) }
        if offered {
            let timed = picked.count >= 16
            model.setWhen(
                ref, when: picked,
                duration: timed == item.isTimed ? .keep : (timed ? .minutes(minutes: 60) : .clear))
        } else if picked != startDay {
            model.setWhen(ref, when: picked + when.dropFirst(10).prefix(6))
        }
    }

    /// The span ends on `day`, if it can; a day it cannot reach
    /// changes nothing.
    private func setEnd(_ day: String) {
        guard let when = item.when, let startDay,
              let offset = WhenSpan.days(from: startDay, to: day, calendar: WhenText.calendar),
              offset != endOffset,
              let length = WhenSpan.endLength(when: when, duration: item.duration, offset: offset)
        else { return }
        model.setDuration(ref, minutes: length > 0 ? length : nil)
    }

    /// The days the end's calendar offers: from the start day (the
    /// day after, while the end's clock time is not past the start's)
    /// to the one-year ceiling.
    private var endRange: ClosedRange<Date>? {
        guard let when = item.when, let startDay, let first = WhenText.date(startDay) else { return nil }
        let calendar = WhenText.calendar
        let from = WhenSpan.endLength(when: when, duration: item.duration, offset: 0) == nil ? 1 : 0
        guard let low = calendar.date(byAdding: .day, value: from, to: first),
              let high = calendar.date(byAdding: .day, value: WhenSpan.maxDuration / WhenSpan.day - 1, to: first)
        else { return nil }
        return low...high
    }

    // MARK: Times

    /// Where the start's wheel opens: the start time, 09:00 on an
    /// all-day date.
    private var startTime: Date {
        guard let when = item.when else { return Date() }
        return WhenText.date(item.isTimed ? when : "\(when.prefix(10))T09:00") ?? Date()
    }

    /// Where the end's wheel opens: start + the stored length, an hour
    /// after the start while there is none.
    private var endTime: Date {
        let start = item.when.flatMap(WhenText.date) ?? Date()
        let length = item.duration.flatMap { $0 > 0 ? $0 : nil } ?? WhenSpan.defaultLength
        return WhenText.calendar.date(byAdding: .minute, value: length, to: start) ?? start
    }

    /// A start time confirmed. On an all-day date a span of whole days
    /// keeps its last day (`WhenSpan.lengthForTimedStart`).
    private func setStartTime(_ time: Date) {
        guard let startDay else { return }
        var duration = DurationChange.keep
        if !item.isTimed {
            let length = WhenSpan.lengthForTimedStart(item.duration)
            if length != item.duration {
                duration = length.map { .minutes(minutes: UInt32(clamping: $0)) } ?? .clear
            }
        }
        model.setWhen(ref, when: "\(startDay)T\(WhenText.time(time))", duration: duration)
    }

    /// An end time confirmed: the length up to it.
    private func setEndTime(_ time: Date) {
        guard let when = item.when, item.isTimed else { return }
        let c = WhenText.calendar.dateComponents([.hour, .minute], from: time)
        let end = (c.hour ?? 0) * 60 + (c.minute ?? 0)
        model.setDuration(ref, minutes: Self.length(from: when, toClock: end, was: item.duration))
    }

    /// Minutes of the day a timed `when` starts at.
    static func startClock(_ when: String) -> Int { WhenSpan.clock(of: when) ?? 0 }

    /// The length an end time picked on the clock gives, the web's
    /// rule (spec/calendar-plan.md "UI shows an end time, stores a
    /// length"): the end stays on the day it was already on; an end at
    /// or before the start means the next day.
    static func length(from when: String, toClock end: Int, was duration: Int?) -> Int {
        let length = WhenSpan.endOffset(when: when, duration: duration) * 1440 + end - startClock(when)
        return length > 0 ? length : length + 1440
    }
}

/// A day's picker, the half sheet under the date editor's boxes: a
/// text box, the offers for what is typed in it, and a month calendar.
/// The day picked is a draft, shown on the calendar and named under
/// the title, until the tick.
///
/// A date typed as words ("fri 3pm", "tomorrow", "14 nov") is read as
/// it is typed and its first reading becomes the draft; the other
/// readings are rows to tap. Before anything is typed the rows are
/// whatever the caller offers for nothing (Today and Tomorrow on an
/// item with no date). A tap on the calendar is a draft of that day
/// alone. The tick is off while the text names no date.
private struct DaySheet: View {
    let title: String
    /// The days the calendar offers; nil for any.
    let range: ClosedRange<Date>?
    /// What typed text could mean, likeliest first.
    let offers: (String) -> [ItemDateEditor.Offer]
    let cancel: () -> Void
    /// The tick: the draft, and whether it is an offer (a whole date,
    /// maybe with a time) or a day off the calendar (`YYYY-MM-DD`).
    let commit: (_ when: String, _ offered: Bool) -> Void

    /// The calendar's day.
    @State private var day: Date
    /// The offer taken, while the calendar still shows its day.
    @State private var offered: String?
    @State private var query = ""
    @FocusState private var typing: Bool

    init(
        title: String, initial: String, range: ClosedRange<Date>?,
        offers: @escaping (String) -> [ItemDateEditor.Offer],
        cancel: @escaping () -> Void, commit: @escaping (String, Bool) -> Void
    ) {
        self.title = title
        self.range = range
        self.offers = offers
        self.cancel = cancel
        self.commit = commit
        _day = State(initialValue: WhenText.date(initial) ?? Date())
    }

    private var typed: String { query.trimmingCharacters(in: .whitespacesAndNewlines) }
    private var draft: String { offered ?? WhenText.day(day) }

    var body: some View {
        let rows = offers(typed)
        ScrollView {
            VStack(alignment: .leading, spacing: 12) {
                typedBox
                offerRows(rows)
                DatePicker(
                    title, selection: $day, in: range ?? Date.distantPast...Date.distantFuture,
                    displayedComponents: .date
                )
                .datePickerStyle(.graphical)
                .labelsHidden()
                .environment(\.calendar, WhenText.calendar)
            }
            .padding()
        }
        .scrollBounceBehavior(.basedOnSize)
        .scrollDismissesKeyboard(.interactively)
        .editorHeader(
            title: title, itemTitle: ItemView.whenText(draft), canFinish: typed.isEmpty || !rows.isEmpty,
            cancel: cancel, done: { commit(draft, offered != nil) }
        )
        .onChange(of: query) {
            // What is typed is the draft as soon as it names a date.
            if !typed.isEmpty, let first = offers(typed).first { take(first.when) }
        }
        .onChange(of: day) {
            // Another day tapped on the calendar: the offer is gone.
            if let offered, WhenText.day(day) != String(offered.prefix(10)) { self.offered = nil }
        }
    }

    private func take(_ when: String) {
        offered = when
        if let date = WhenText.date(String(when.prefix(10))) { day = date }
    }

    private var typedBox: some View {
        HStack(spacing: 8) {
            Image(systemName: "character.cursor.ibeam").foregroundStyle(.secondary).frame(width: 20)
            TextField("Type a date: fri 3pm", text: $query)
                .focused($typing)
                .submitLabel(.done)
                .autocorrectionDisabled()
                .textInputAutocapitalization(.never)
                .onSubmit { typing = false }
            if !query.isEmpty {
                Button {
                    query = ""
                } label: {
                    Image(systemName: "xmark.circle.fill").foregroundStyle(.tertiary)
                }
                .buttonStyle(.plain)
                .accessibilityLabel("Clear text")
            }
        }
        .padding(.horizontal, 12)
        .frame(minHeight: 40)
        .background(.quaternary.opacity(0.5), in: RoundedRectangle(cornerRadius: 10, style: .continuous))
    }

    /// The offers, each a row to tap, the draft's ticked; a muted line
    /// when the text names no date.
    @ViewBuilder
    private func offerRows(_ rows: [ItemDateEditor.Offer]) -> some View {
        if rows.isEmpty {
            if !typed.isEmpty {
                Text("No date in that yet")
                    .foregroundStyle(.secondary)
                    .frame(minHeight: 36)
            }
        } else {
            ForEach(rows, id: \.when) { offer in
                Button {
                    take(offer.when)
                } label: {
                    HStack(alignment: .firstTextBaseline, spacing: 12) {
                        if let name = offer.name {
                            Text(name)
                            Spacer(minLength: 8)
                            Text(ItemView.whenText(offer.when)).foregroundStyle(.secondary)
                        } else {
                            Text(ItemView.whenText(offer.when))
                            Spacer(minLength: 8)
                        }
                        if offered == offer.when {
                            Image(systemName: "checkmark").foregroundStyle(.tint)
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
}

/// A time's picker, the half sheet under the date editor's boxes: the
/// system's wheel, a draft until the tick.
private struct TimeSheet: View {
    let title: String
    let cancel: () -> Void
    let commit: (Date) -> Void
    @State private var time: Date

    init(title: String, initial: Date, cancel: @escaping () -> Void, commit: @escaping (Date) -> Void) {
        self.title = title
        self.cancel = cancel
        self.commit = commit
        _time = State(initialValue: initial)
    }

    var body: some View {
        DatePicker(title, selection: $time, displayedComponents: .hourAndMinute)
            .datePickerStyle(.wheel)
            .labelsHidden()
            .frame(maxWidth: .infinity)
            .editorHeader(title: title, cancel: cancel, done: { commit(time) })
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

    /// A day as a narrow box shows it: "Wed 23 Sept".
    static func dayLabel(_ day: String) -> String { WhenSpan.dayLabel(day, calendar: calendar) }
}
