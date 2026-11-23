// The item panel's time-zone box, under the time row and there only
// while the planned date has a time (`spec/calendar-plan.md` "Fixed
// instants"; the web's `js/web/src/ZonePicker.tsx`). At rest it is the
// zone's offset on the item's day and its city ("GMT+1 Paris"), or a
// muted "Time zone" while the time is floating: wall time wherever the
// device is. Picking a zone fixes the time to it: the digits in the
// date and time boxes over this one stay as they are and now read on
// that zone's clock, which is what those boxes show and edit from then
// on. The ✕ at the trailing edge under the pointer makes the time
// floating again, at the moment it had.
//
// The time boxes' way of working (`TimeField.swift`): a click turns the
// box into a text box and drops a list under it, floating over the
// panel's rows: every zone by city, the device's own first, until
// something is typed, then the zones the text names (`Zones` in the
// Kit: city, id, name or offset, by word prefix). Each row is the city
// with the zone's name dim after it and its offset on the item's day
// at the trailing edge. Return takes the highlighted row, a click
// takes its row, the arrow keys move the highlight; Escape, or the
// caret leaving the box, shuts it with the zone as it was.

import AppKit
import MonoplanMacKit
import SwiftUI

struct ZoneField: View {
    /// The IANA zone the time is fixed to; nil is floating.
    let zone: String?
    /// The item's day (`YYYY-MM-DD`, or a whole `when`): offsets are
    /// quoted as of it.
    let day: String
    /// A zone was picked, or taken off (nil).
    let commit: (String?) -> Void

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
    private var zones: [ZoneOption] {
        Zones.suggestions(query, in: Zones.options(day: day), first: Zones.deviceZone())
    }
    /// What the box reads at rest; nil while floating.
    private var shownLabel: String? { zone.map { Zones.option($0, day: day).label } }

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
                        .padding(.horizontal, -PanelBox.inset)
                }
            } else {
                shown
            }
        }
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
        let placeholder = String(localized: "Time zone")
        return Button {
            picking = true
        } label: {
            HStack(alignment: .firstTextBaseline, spacing: 5) {
                Image(systemName: "globe")
                    .foregroundStyle(zone == nil ? .tertiary : .secondary)
                    .frame(width: 20)
                Text(verbatim: shownLabel ?? placeholder)
                    .lineLimit(1)
                    .foregroundStyle(zone == nil ? .tertiary : .primary)
            }
            .panelBox(filled: false)
        }
        .buttonStyle(.plain)
        .accessibilityLabel(Text(verbatim: placeholder))
        .accessibilityValue(Text(verbatim: shownLabel ?? ""))
        .overlay(alignment: .trailing) {
            if zone != nil, hovered {
                let label = String(localized: "Remove time zone")
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
                .help(label)
                .accessibilityLabel(Text(verbatim: label))
            }
        }
        .onHover { hovered = $0 }
    }

    private var textBox: some View {
        let placeholder = String(localized: "Time zone")
        return HStack(alignment: .firstTextBaseline, spacing: 5) {
            Image(systemName: "globe")
                .foregroundStyle(.secondary)
                .frame(width: 20)
            // The stored zone stays in sight, dim, until typed over.
            TextField(text: $query, prompt: Text(verbatim: shownLabel ?? placeholder)) {
                Text(verbatim: placeholder)
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

    /// The zones in a scrolling list.
    private var dropdown: some View {
        let zones = zones
        let highlight = highlight(in: zones)
        return VStack(alignment: .leading, spacing: 0) {
            ScrollViewReader { proxy in
                ScrollView {
                    // Some hundreds of rows while nothing is typed.
                    LazyVStack(alignment: .leading, spacing: 0) {
                        ForEach(Array(zones.enumerated()), id: \.offset) { index, option in
                            ZoneRow(option: option, highlighted: highlight == index) { pick(option.id) }
                                .onHover { if $0 { hover(index) } }
                                .id(index)
                        }
                    }
                }
                .frame(maxHeight: 208)
                // The long blank list opens on the stored zone; the
                // keys keep their row in sight.
                .onAppear { if let highlight { proxy.scrollTo(highlight, anchor: .center) } }
                .onChange(of: stepped) { if let chosen { proxy.scrollTo(chosen) } }
            }
            if zones.isEmpty {
                Text("No time zone in that yet")
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

    /// The row Return takes: the chosen one, else the stored zone's in
    /// the blank list (none while floating), else the first once
    /// something is typed.
    private func highlight(in zones: [ZoneOption]) -> Int? {
        guard !zones.isEmpty else { return nil }
        if let chosen, zones.indices.contains(chosen) { return chosen }
        return blank ? zones.firstIndex { $0.id == zone } : 0
    }

    private func hover(_ index: Int) {
        let mouse = NSEvent.mouseLocation
        guard mouse != lastMouse else { return }
        lastMouse = mouse
        chosen = index
    }

    private func step(_ by: Int) -> KeyPress.Result {
        let zones = zones
        guard !zones.isEmpty else { return .ignored }
        let from = highlight(in: zones) ?? (by > 0 ? -1 : 0)
        chosen = (from + by + zones.count) % zones.count
        stepped += 1
        return .handled
    }

    private func submit() {
        let zones = zones
        if let highlight = highlight(in: zones) {
            pick(zones[highlight].id)
        } else if blank {
            finish()
        }
        // Text that names no zone stays up to be fixed.
    }

    private func pick(_ id: String) {
        if id != zone { commit(id) }
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

/// One row of the list: the city, the zone's name dim after it, and
/// its offset at the trailing edge.
private struct ZoneRow: View {
    let option: ZoneOption
    let highlighted: Bool
    let action: () -> Void

    var body: some View {
        Button(action: action) {
            HStack(alignment: .firstTextBaseline, spacing: 8) {
                Text(verbatim: option.city).lineLimit(1).layoutPriority(1)
                Text(verbatim: option.name)
                    .font(.footnote)
                    .foregroundStyle(highlighted ? .white.opacity(0.8) : .secondary)
                    .lineLimit(1)
                Spacer(minLength: 0)
                Text(verbatim: option.offset)
                    .font(.footnote)
                    .monospacedDigit()
                    .foregroundStyle(highlighted ? .white.opacity(0.8) : .secondary)
                    .lineLimit(1)
                    .layoutPriority(1)
            }
            .foregroundStyle(highlighted ? .white : .primary)
            .padding(.horizontal, 8)
            .padding(.vertical, 4)
            .frame(maxWidth: .infinity, alignment: .leading)
            .background(
                highlighted ? Color.accentColor : .clear,
                in: RoundedRectangle(cornerRadius: 5, style: .continuous))
            .contentShape(Rectangle())
            .accessibilityElement(children: .combine)
        }
        .buttonStyle(.plain)
    }
}
