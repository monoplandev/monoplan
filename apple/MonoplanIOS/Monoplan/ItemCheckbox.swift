// The checkbox an item row leads with: an empty rounded square while the
// item is open, filled and ticked once Done, crossed when Cancelled
// (spec/data-model.md "Lifecycle"). `ItemCheckbox` is the drawing,
// `ItemCheckButton` the control that ticks and reopens.

import MonoplanCore
import SwiftUI
import UIKit

struct ItemCheckbox: View {
    let state: ItemState

    /// Drawn rather than an SF Symbol, whose corner radius and stroke
    /// are not ours to set. Sized with the text (Dynamic Type).
    @ScaledMetric(relativeTo: .body) private var side: CGFloat = 20
    private static let radius: CGFloat = 0.32
    private static let border: CGFloat = 1.25
    private static let nudge: CGFloat = 1.5

    var body: some View {
        let shape = RoundedRectangle(cornerRadius: side * Self.radius, style: .continuous)
        ZStack {
            if state == .done {
                shape.fill(.tint)
                mark(cancelled: false).foregroundStyle(.white)
            } else {
                shape.strokeBorder(.tertiary, lineWidth: Self.border)
                if state == .cancelled {
                    mark(cancelled: true).foregroundStyle(.tertiary)
                }
            }
        }
        .frame(width: side, height: side)
        // A shape has no baseline of its own: sit the box so its middle
        // is level with the middle of the first line's capitals, then a
        // touch lower, where it looks centred on the line as a whole.
        .alignmentGuide(.firstTextBaseline) { $0[.bottom] - side * 0.2 - Self.nudge }
        .accessibilityElement()
        .accessibilityLabel(label)
    }

    private func mark(cancelled: Bool) -> some View {
        CheckGlyph(cancelled: cancelled).stroked(side)
    }

    private var label: String {
        switch state {
        case .done: return String(localized: "Done")
        case .cancelled: return String(localized: "Cancelled")
        default: return String(localized: "Not done")
        }
    }
}

/// The checkbox as a control: a tap ticks an open item and reopens a
/// closed one. The hit area is larger than the drawn box.
struct ItemCheckButton: View {
    @EnvironmentObject private var model: AppModel
    let ref: ItemRef
    let state: ItemState

    private var closed: Bool { state.isClosed }

    var body: some View {
        Button {
            UIImpactFeedbackGenerator(style: .light).impactOccurred()
            model.setDone(ref, done: !closed)
        } label: {
            ItemCheckbox(state: state)
                .padding(10)
                .contentShape(Rectangle())
                .padding(-10)
        }
        .buttonStyle(.plain)
        .accessibilityLabel(state == .cancelled ? String(localized: "Reopen") : closed ? String(localized: "Mark not done") : String(localized: "Mark done"))
    }
}

/// An item row's layout: the checkbox, then whatever the row says,
/// aligned to its first line. Two taps: the checkbox ticks, the rest of
/// the row opens the item's sheet. The item is the enclosing
/// `ItemLink`'s; without one the row is inert.
struct CheckboxRow<Content: View>: View {
    @Environment(\.itemRef) private var ref
    @Environment(\.openItem) private var openItem
    let state: ItemState
    /// False where the content places the star itself (`RowStar`), as
    /// the agenda row does beside its text.
    var trailingStar = true
    /// The date moves the row's menu offers (`AgendaEntry.quickMoves`);
    /// none on a row that is not the calendar's.
    var moves: [WhenMove] = []
    /// False on the calendar's row for an event with no box
    /// (`AgendaEntry.showsCheck`): the event pill holds the box's place.
    var check = true
    @ViewBuilder let content: Content

    var body: some View {
        HStack(alignment: .firstTextBaseline, spacing: 12) {
            if !check {
                EventSlot()
            } else if let ref {
                ItemCheckButton(ref: ref, state: state)
            } else {
                ItemCheckbox(state: state)
            }
            HStack(alignment: .firstTextBaseline, spacing: 8) {
                content
                    .frame(maxWidth: .infinity, alignment: .leading)
                if trailingStar { RowStar(state: state) }
            }
            .contentShape(Rectangle())
            .onTapGesture { if let ref { openItem(ref) } }
            .accessibilityAddTraits(.isButton)
        }
        // The pill runs the height of the row, so it spans the lines
        // under the text too: drawn over the row, in the box's column.
        .overlay(alignment: .leading) {
            if !check { EventPill() }
        }
        .modifier(ItemRowMenu(ref: ref, state: state, moves: moves))
        // No dividers between item rows, on any page that lists them.
        .itemListRow()
    }
}

/// The checkbox's place on a calendar row that has none (an event's,
/// spec/events-plan.md): empty, as wide as the box, so the texts stay
/// in one column. `EventPill` is drawn over it.
struct EventSlot: View {
    /// `ItemCheckbox`'s side.
    @ScaledMetric(relativeTo: .body) private var side: CGFloat = 20

    var body: some View {
        Color.clear
            .frame(width: side, height: 1)
            .accessibilityHidden(true)
    }
}

/// What an event's calendar row leads with instead of a checkbox: the
/// short upright bar a calendar leads an event with, grey, as tall as
/// the row and centred in the box's column. Display only; the row's
/// menu and the item's page still mark it done.
struct EventPill: View {
    @ScaledMetric(relativeTo: .body) private var side: CGFloat = 20

    var body: some View {
        Capsule()
            .fill(.tertiary)
            .frame(width: 3)
            .padding(.vertical, 2)
            .frame(width: side)
            .accessibilityHidden(true)
    }
}

/// A row's star, drawn when its item (the enclosing `ItemLink`'s) is
/// starred, and the page is not Starred itself, where every row would
/// carry one. `CheckboxRow` ends in it unless the content places it.
struct RowStar: View {
    @EnvironmentObject private var model: AppModel
    @Environment(\.itemRef) private var ref
    @Environment(\.duplicatesStarred) private var onStarredPage
    let state: ItemState

    private var shown: Bool {
        guard let ref, !onStarredPage, !state.isClosed else { return false }
        return model.starred.contains { $0.id == ref.itemId }
    }

    var body: some View {
        if shown { StarMark() }
    }
}

/// The mark a starred item's row carries: the item page's star, filled
/// (`StarFill` under `StarStroke`), sized with the text. Display only;
/// the row's menu and the item's page do the starring.
struct StarMark: View {
    @ScaledMetric(relativeTo: .body) private var side: CGFloat = 15

    /// The web's `#ffe785`.
    static let fill = Color(red: 1.0, green: 0.906, blue: 0.522)

    var body: some View {
        ZStack {
            Image("StarFill")
                .resizable()
                .foregroundStyle(Self.fill)
            Image("StarStroke")
                .resizable()
                .foregroundStyle(Color.primary.opacity(0.7))
        }
        .frame(width: side, height: side)
        // Level with the first line's capitals, as the checkbox is.
        .alignmentGuide(.firstTextBaseline) { $0[.bottom] - side * 0.1 }
        .accessibilityElement()
        .accessibilityLabel("Starred")
    }
}

private struct DuplicatesStarredKey: EnvironmentKey {
    static let defaultValue = false
}

extension EnvironmentValues {
    /// Rows under here are Starred's: duplicating one stars the copy,
    /// directly below it. Set by `StarredView`.
    var duplicatesStarred: Bool {
        get { self[DuplicatesStarredKey.self] }
        set { self[DuplicatesStarredKey.self] = newValue }
    }
}

/// The menu a long press on an item row opens, the same on every row
/// surface: tick or reopen, star or unstar, copy the title, duplicate,
/// bin (which asks first). A calendar row that is overdue or on today
/// also offers to move its date (`moves`: to today, to tomorrow).
/// Without an item (`ref` nil) there is no menu. On the Starred
/// page (`duplicatesStarred`) a duplicate is starred too, under the
/// row it came from.
struct ItemRowMenu: ViewModifier {
    @EnvironmentObject private var model: AppModel
    @Environment(\.duplicatesStarred) private var duplicatesStarred
    let ref: ItemRef?
    let state: ItemState
    var moves: [WhenMove] = []

    /// "Move to Bin" is waiting on its confirmation.
    @State private var confirmingBin = false

    private var closed: Bool { state.isClosed }

    func body(content: Content) -> some View {
        if let ref {
            content
                .contextMenu { items(ref) }
                .confirmationDialog("Move this item to the Bin?", isPresented: $confirmingBin, titleVisibility: .visible) {
                    Button("Move to Bin", role: .destructive) { model.setBinned(ref, binned: true) }
                } message: {
                    Text("It can be restored from the Bin.")
                }
        } else {
            content
        }
    }

    @ViewBuilder
    private func items(_ ref: ItemRef) -> some View {
        Button {
            model.setDone(ref, done: !closed)
        } label: {
            if closed {
                Label(state == .cancelled ? String(localized: "Reopen") : String(localized: "Mark not done"), systemImage: "arrow.uturn.backward")
            } else {
                Label("Mark done", systemImage: "checkmark")
            }
        }
        // Starred holds Open items only (`spec/focus.md`), so a closed
        // row has nothing to star.
        if !closed {
            let starred = model.starred.contains { $0.id == ref.itemId }
            Button {
                model.setStarred(ref, starred: !starred)
            } label: {
                Label(starred ? String(localized: "Unstar") : String(localized: "Star"), systemImage: starred ? "star.slash" : "star")
            }
        }
        // The length stays as it is: only the day moves.
        ForEach(moves, id: \.when) { move in
            Button {
                model.setWhen(ref, when: move.when)
            } label: {
                switch move.day {
                case .today: Label("Move to Today", systemImage: "calendar")
                case .tomorrow: Label("Move to Tomorrow", systemImage: "calendar")
                }
            }
        }
        Button {
            UIPasteboard.general.string = ref.title
        } label: {
            Label("Copy", systemImage: "doc.on.doc")
        }
        Button {
            model.duplicateItem(ref, star: duplicatesStarred)
        } label: {
            Label("Duplicate", systemImage: "plus.square.on.square")
        }
        Button(role: .destructive) {
            confirmingBin = true
        } label: {
            Label("Move to Bin", systemImage: "trash")
        }
    }
}

extension View {
    /// An item's text in a row: one line, with anything past it cut
    /// off by an ellipsis, so rows stay compact and share a height.
    func itemTextSlot() -> some View {
        lineLimit(1)
            .truncationMode(.tail)
    }

    /// An item row's place in its list: no divider, and less room
    /// above and below than a list gives by default. The system still
    /// holds a row to its minimum height (44 pt), so a row of one line
    /// is that tall whatever is asked here.
    func itemListRow() -> some View {
        listRowSeparator(.hidden)
            .listRowInsets(EdgeInsets(top: 4, leading: 20, bottom: 4, trailing: 20))
    }
}

/// The web's tick and cross (`js/web/src/icons/check.svg`, and the
/// `.task-check` cross in `styles.css`): strokes on a 16-unit grid
/// that spans the whole box, stroked an eighth of its side, rounded.
struct CheckGlyph: Shape {
    let cancelled: Bool

    /// The strokes as polylines in grid units, y down.
    static func strokes(cancelled: Bool) -> [[CGPoint]] {
        cancelled
            ? [[CGPoint(x: 5, y: 5), CGPoint(x: 11, y: 11)], [CGPoint(x: 11, y: 5), CGPoint(x: 5, y: 11)]]
            : [[CGPoint(x: 3.5, y: 8.5), CGPoint(x: 6.5, y: 11.5), CGPoint(x: 12.5, y: 5.5)]]
    }

    func path(in rect: CGRect) -> Path {
        let unit = rect.width / 16
        var path = Path()
        for stroke in Self.strokes(cancelled: cancelled) {
            path.addLines(stroke.map { CGPoint(x: rect.minX + $0.x * unit, y: rect.minY + $0.y * unit) })
        }
        return path
    }

    /// Stroked for a box `side` across.
    func stroked(_ side: CGFloat) -> some Shape {
        stroke(style: StrokeStyle(lineWidth: side / 8, lineCap: .round, lineJoin: .round))
    }
}
