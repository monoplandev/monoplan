// The item panel's time boxes, under the planned date: when it starts
// and, once it has a start, when it ends. The web's typed time picker
// (`js/web/src/TimePicker.tsx`) in the panel's box, working the way the
// date and place rows do (`DateField.swift`, `PlaceField.swift`). At
// rest a box is its time ("14:00"), or a muted word when there is none
// ("All day", "End"). A click turns it into a text box and drops a list
// under it, floating over the panel's rows: every quarter hour, the
// stored time under the highlight, until something is typed, then only
// the times the text names (`TimeOffers`, the web's `timeSuggest.ts`).
// The start's box has a small ✕ at its trailing edge under the
// pointer while a time is set, which takes the time off.
//
// Return takes the highlighted row, a click takes its row, the arrow
// keys move the highlight round the list; Escape, or the caret leaving
// the box, shuts it with the time as it was. Times read on the 24-hour
// clock, as the rest of the panel writes them.

import AppKit
import SwiftUI

struct TimeField: View {
    /// The stored time as minutes past midnight; nil for none.
    let value: Int?
    /// The word at rest while there is no time.
    let placeholder: String
    /// The box's accessible name.
    let label: String
    let systemImage: String
    /// The start, for the end's box: its blank list runs from here
    /// round the clock, and typed readings order by how long after it
    /// they fall.
    var after: Int? = nil
    /// A muted note after each row; for the end, the length that row
    /// would give.
    var hint: ((Int) -> String?)? = nil
    /// The name of the ✕ at the box's trailing edge, shown under the
    /// pointer while a time is set, which takes the time off (nil).
    /// Without it the box has no ✕.
    var clearLabel: String? = nil
    /// A time was picked, or taken off (nil).
    let commit: (Int?) -> Void

    @State private var query = ""
    @State private var picking = false
    /// The text box has had the caret since it opened: only then is
    /// the caret leaving it a reason to shut.
    @State private var armed = false
    /// The row the arrow keys or the pointer chose; nil for the
    /// default.
    @State private var chosen: Int?
    /// Where the pointer last chose a row: rows sliding under a still
    /// pointer as the keys scroll the list do not choose again.
    @State private var lastMouse: CGPoint?
    /// Counts the arrow keys' steps: the list, which has the scroll
    /// position, brings the chosen row into sight on each.
    @State private var stepped = 0
    /// The pointer is over the box at rest: the ✕ shows.
    @State private var hovered = false
    @FocusState private var focused: Bool

    private var blank: Bool { query.trimmingCharacters(in: .whitespaces).isEmpty }
    private var times: [Int] { TimeOffers.suggestions(query, after: after) }

    var body: some View {
        Group {
            if picking {
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
        // Its list draws over the box beside it.
        .zIndex(picking ? 1 : 0)
        .onChange(of: query) { chosen = nil }
        .onChange(of: focused) {
            if focused {
                armed = true
            } else if armed {
                finish()
            }
        }
    }

    // MARK: Rows

    private var shown: some View {
        Button {
            picking = true
        } label: {
            HStack(alignment: .firstTextBaseline, spacing: 5) {
                Image(systemName: systemImage)
                    .foregroundStyle(value == nil ? .tertiary : .secondary)
                    .frame(width: 20)
                Text(verbatim: value.map(TimeOffers.text) ?? placeholder)
                    .lineLimit(1)
                    .foregroundStyle(value == nil ? .tertiary : .primary)
            }
            .panelBox(filled: false)
        }
        .buttonStyle(.plain)
        .accessibilityLabel(Text(verbatim: label))
        .accessibilityValue(Text(verbatim: value.map(TimeOffers.text) ?? placeholder))
        .overlay(alignment: .trailing) {
            if let clearLabel, value != nil, hovered {
                Button {
                    commit(nil)
                } label: {
                    Image(systemName: "xmark")
                        .font(.caption.weight(.semibold))
                        .foregroundStyle(.secondary)
                        .frame(width: 20, height: 20)
                        .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
                .hoverCircle()
                .help(clearLabel)
                .accessibilityLabel(Text(verbatim: clearLabel))
            }
        }
        .onHover { hovered = $0 }
    }

    private var textBox: some View {
        HStack(alignment: .firstTextBaseline, spacing: 5) {
            Image(systemName: systemImage)
                .foregroundStyle(.secondary)
                .frame(width: 20)
            // The stored time stays in sight, dim, until typed over.
            TextField(text: $query, prompt: Text(verbatim: value.map(TimeOffers.text) ?? placeholder)) {
                Text(verbatim: label)
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
        // A turn after the box appears, as the place field's.
        .onAppear { DispatchQueue.main.async { focused = true } }
    }

    /// The times in a scrolling list.
    private var dropdown: some View {
        let times = times
        let highlight = highlight(in: times)
        return VStack(alignment: .leading, spacing: 0) {
            ScrollViewReader { proxy in
                ScrollView {
                    VStack(alignment: .leading, spacing: 0) {
                        ForEach(Array(times.enumerated()), id: \.offset) { index, time in
                            TimeRow(
                                text: TimeOffers.text(time), hint: hint?(time), highlighted: highlight == index
                            ) {
                                pick(time)
                            }
                            .onHover { if $0 { hover(index) } }
                            .id(index)
                        }
                    }
                }
                .frame(maxHeight: 208)
                // The long blank list opens with the stored time in
                // the middle; the keys keep their row in sight.
                .onAppear { if let highlight { proxy.scrollTo(highlight, anchor: .center) } }
                .onChange(of: stepped) { if let chosen { proxy.scrollTo(chosen) } }
            }
            if times.isEmpty {
                Text("No time in that yet")
                    .foregroundStyle(.secondary)
                    .padding(.horizontal, 8)
                    .padding(.vertical, 4)
            }
        }
        .padding(4)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(Color(nsColor: .controlBackgroundColor), in: RoundedRectangle(cornerRadius: 8, style: .continuous))
        .overlay(RoundedRectangle(cornerRadius: 8, style: .continuous).strokeBorder(.separator))
        .shadow(color: .black.opacity(0.18), radius: 10, y: 4)
    }

    // MARK: Picking

    /// The row Return takes: the chosen one, else the stored time's in
    /// the blank list, else the first.
    private func highlight(in times: [Int]) -> Int? {
        guard !times.isEmpty else { return nil }
        if let chosen, times.indices.contains(chosen) { return chosen }
        return blank ? min(times.count - 1, TimeOffers.nearestIndex(value, after: after)) : 0
    }

    private func hover(_ index: Int) {
        let mouse = NSEvent.mouseLocation
        guard mouse != lastMouse else { return }
        lastMouse = mouse
        chosen = index
    }

    private func step(_ by: Int) -> KeyPress.Result {
        let times = times
        guard !times.isEmpty else { return .ignored }
        let from = highlight(in: times) ?? (by > 0 ? -1 : 0)
        let next = (from + by + times.count) % times.count
        chosen = next
        stepped += 1
        return .handled
    }

    private func submit() {
        let times = times
        if let highlight = highlight(in: times) { pick(times[highlight]) }
        // Text that names no time stays up to be fixed.
    }

    private func pick(_ time: Int?) {
        commit(time)
        finish()
    }

    private func finish() {
        query = ""
        chosen = nil
        picking = false
        armed = false
        hovered = false
        focused = false
    }
}

/// One row of the list: a time, and its note at the trailing edge.
private struct TimeRow: View {
    let text: String
    let hint: String?
    let highlighted: Bool
    let action: () -> Void

    var body: some View {
        Button(action: action) {
            HStack(alignment: .firstTextBaseline, spacing: 8) {
                Text(verbatim: text).lineLimit(1).monospacedDigit()
                Spacer(minLength: 0)
                if let hint {
                    Text(verbatim: hint)
                        .font(.footnote)
                        .foregroundStyle(highlighted ? .white.opacity(0.8) : .secondary)
                        .lineLimit(1)
                }
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
}

/// What typed text could mean as a time of day, as minutes past
/// midnight: the web's `timeSuggest.ts`, on the 24-hour clock.
///
///   ""       every quarter hour from midnight, in day order
///   "1"      13:00 and 01:00; "13" is 13:00 alone
///   "1:3"    1:30 in both halves: a single minute digit is the tens
///   "130"    1:30 as well, "1330" 13:30: with no separator the last
///            two digits are the minutes. "." also separates.
///   "1p"     the afternoon reading alone (a / p / am / pm, any case)
///
/// Two readings are ordered by distance from noon, so the daytime one
/// leads: 1 to 6 and 12 the afternoon first, 7 to 11 the morning.
/// With an `after` time (the end's list, anchored on the start) the
/// blank list runs from `after` + 15 minutes round the clock to `after`
/// itself a day later, and two readings order by how long after the
/// start they fall, so "3" after 14:00 is 15:00 before 03:00.
enum TimeOffers {
    /// Minutes between rows of the blank list.
    static let step = 15
    static let day = 24 * 60
    /// The core's ceiling on `duration`: a year of days.
    static let maxDuration = 365 * day

    private static let query = try! NSRegularExpression(
        pattern: #"^\s*(\d{1,4})(?:[:.](\d{0,2}))?\s*(?:([ap])\.?\s*m?\.?)?\s*$"#,
        options: .caseInsensitive)

    static func suggestions(_ text: String, after: Int? = nil) -> [Int] {
        if text.trimmingCharacters(in: .whitespaces).isEmpty {
            guard let after else { return Array(stride(from: 0, to: day, by: step)) }
            return stride(from: step, through: day, by: step).map { (after + $0) % day }
        }
        let string = text as NSString
        guard let match = query.firstMatch(in: text, range: NSRange(location: 0, length: string.length))
        else { return [] }
        func group(_ index: Int) -> String? {
            let range = match.range(at: index)
            return range.location == NSNotFound ? nil : string.substring(with: range)
        }
        var digits = group(1) ?? ""
        var minuteText = group(2)
        let half = group(3)?.lowercased()

        // No separator: the last two digits are the minutes. Three or
        // four digits only ever mean that; one or two are an hour.
        if minuteText == nil, digits.count > 2 {
            minuteText = String(digits.suffix(2))
            digits = String(digits.dropLast(2))
        } else if digits.count > 2 {
            return []
        }
        guard let hour = Int(digits) else { return [] }
        let minute: Int
        switch minuteText {
        case nil, "": minute = 0
        case let text? where text.count == 1: minute = (Int(text) ?? 0) * 10
        case let text?: minute = Int(text) ?? 0
        }
        guard minute <= 59 else { return [] }

        let times = hours(hour, half: half).map { $0 * 60 + minute }
        guard let after else { return times }
        return times.sorted { minutes(from: after, to: $0) < minutes(from: after, to: $1) }
    }

    /// The hours a typed hour can mean, likeliest first.
    private static func hours(_ hour: Int, half: String?) -> [Int] {
        if hour > 23 { return [] }
        // A PM suffix is redundant past 12, an AM one contradictory.
        if hour > 12 { return half == "a" ? [] : [hour] }
        if hour == 0 { return half == "p" ? [] : [0] }
        let am = hour == 12 ? 0 : hour
        let pm = hour == 12 ? 12 : hour + 12
        if half == "a" { return [am] }
        if half == "p" { return [pm] }
        return abs(pm - 12) <= abs(am - 12) ? [pm, am] : [am, pm]
    }

    /// Minutes from `after` forward to `time` on the clock; the same
    /// time is a full day later, never zero.
    private static func minutes(from after: Int, to time: Int) -> Int {
        let diff = time - after
        return diff > 0 ? diff : diff + day
    }

    /// The blank list's row nearest to (at or before) `time`, so the
    /// stored value is under the highlight when the list opens.
    /// Without a time: 09:00, or an hour on in an `after` list.
    static func nearestIndex(_ time: Int?, after: Int? = nil) -> Int {
        guard let after else { return time.map { $0 / step } ?? 9 * 4 }
        guard let time else { return 60 / step - 1 }
        return max(0, minutes(from: after, to: time) / step - 1)
    }

    /// Length in minutes from a start to an end typed as a clock time,
    /// the end landing `dayOffset` days after the start day. On the
    /// start day an end at or before the start on the clock means the
    /// next day; on a later day the clock reading is taken as is, so
    /// the day holds when the time is retyped.
    static func duration(from start: Int, to end: Int, dayOffset: Int = 0) -> Int {
        let total = dayOffset * day + end - start
        return min(total > 0 ? total : total + day, maxDuration)
    }

    /// "09:05".
    static func text(_ minutes: Int) -> String {
        String(format: "%02d:%02d", (minutes / 60) % 24, minutes % 60)
    }

    /// A `when`'s time of day as minutes past midnight; nil when it is
    /// all-day.
    static func minutes(of when: String) -> Int? {
        guard when.count >= 16,
              let hour = Int(when.dropFirst(11).prefix(2)), let minute = Int(when.dropFirst(14).prefix(2))
        else { return nil }
        return hour * 60 + minute
    }
}
