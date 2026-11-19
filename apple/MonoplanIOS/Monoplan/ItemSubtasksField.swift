// The item page's subtasks (spec/subtasks.md): text nodes with done /
// not done, owned by the item. The Mac panel's field
// (`MonoplanMac/SubtasksField.swift`) for touch: a column of rows, each
// a checkbox and a one-line text field edited in place, over a trailing
// "Add subtask" field that keeps the keyboard after Return so a list
// can be typed straight down.
//
// Return commits and moves down. A row emptied and then left is
// removed: the software keyboard does not report a Backspace in an
// empty field, so there is no "Backspace again" to wait for (a hardware
// keyboard's does remove it there and then, and its Up / Down walk the
// rows, Option+Up / Option+Down reorder).
//
// The grip at a row's end reorders, always shown as there is no
// pointer to hover: the row follows the finger, the rows it passes step
// aside, and the release is one move. Plain offsets over rows of one
// height.

import MonoplanCore
import SwiftUI
import UIKit

struct ItemSubtasksField: View {
    let subtasks: [Subtask]
    /// A binned item's: shown, not edited.
    var locked = false
    let add: (String) -> Void
    /// A subtask's id and its new text, trimmed and not empty.
    let setText: (String, String) -> Void
    let setDone: (String, Bool) -> Void
    /// Move the subtask so it ends at the index.
    let move: (String, Int) -> Void
    let remove: (String) -> Void

    enum Focus: Hashable {
        case row(String)
        case add
    }

    /// A grip drag in flight: the row that was at `from` is `dy` points
    /// off its slot and would land at `to`.
    private struct Drag: Equatable {
        let id: String
        let from: Int
        var to: Int
        var dy: CGFloat
    }

    /// An order just asked for, shown until the reread brings it: a
    /// write comes back a moment later, and without this the rows would
    /// jump back to the old order and then to the new. It holds only
    /// while the engine's order is still the one it was made over.
    private struct Pending {
        let base: [String]
        let order: [String]
    }

    /// A full-height touch target, growing with the text (Dynamic Type).
    @ScaledMetric(relativeTo: .body) private var rowHeight: CGFloat = 44

    @FocusState private var focus: Focus?
    @State private var drag: Drag?
    @State private var pending: Pending?
    @State private var addDraft = ""

    private var shown: [Subtask] {
        guard let pending, pending.base == subtasks.map(\.id) else { return subtasks }
        let byId = Dictionary(subtasks.map { ($0.id, $0) }, uniquingKeysWith: { first, _ in first })
        return pending.order.compactMap { byId[$0] }
    }

    var body: some View {
        let rows = shown
        VStack(alignment: .leading, spacing: 0) {
            ForEach(Array(rows.enumerated()), id: \.element.id) { index, sub in
                SubtaskRow(
                    sub: sub, focus: $focus, locked: locked, height: rowHeight,
                    dragged: drag?.id == sub.id,
                    commit: { setText(sub.id, $0) },
                    toggle: { setDone(sub.id, $0) },
                    step: { focusAt(index + $0, rows) },
                    shift: {
                        let to = index + $0
                        if rows.indices.contains(to) { reorder(sub.id, to: to, rows) }
                    },
                    removeRow: { remove(sub.id) },
                    removeAndStepUp: {
                        // The row above takes the caret; the add field
                        // when it was the first.
                        focus = index > 0 ? .row(rows[index - 1].id) : .add
                        remove(sub.id)
                    },
                    gripMoved: { delta in
                        if drag?.id != sub.id { UIImpactFeedbackGenerator(style: .light).impactOccurred() }
                        let from = drag?.id == sub.id ? drag?.from ?? index : index
                        let target = Self.dragTarget(from: from, delta: delta, rowHeight: rowHeight, count: rows.count)
                        drag = Drag(id: sub.id, from: from, to: target.to, dy: target.dy)
                    },
                    gripReleased: { drop(rows) }
                )
                .offset(y: shift(of: sub.id, at: index))
                .zIndex(drag?.id == sub.id ? 1 : 0)
                // The rows stepping aside ease; the dragged one tracks
                // the finger, and nothing eases as it lands.
                .animation(
                    drag == nil || drag?.id == sub.id ? nil : .easeOut(duration: 0.12), value: drag?.to)
            }
            if !locked {
                addRow(rows)
            }
        }
        .accessibilityElement(children: .contain)
        .accessibilityLabel(Text("Subtasks"))
    }

    private func addRow(_ rows: [Subtask]) -> some View {
        HStack(alignment: .center, spacing: 12) {
            Image(systemName: "plus")
                .foregroundStyle(.tertiary)
                .frame(width: 24)
            TextField("Add subtask", text: $addDraft)
                .focused($focus, equals: .add)
                .submitLabel(.next)
                // Return adds and keeps the keyboard for the next one;
                // on an empty field it puts the keyboard away.
                .onSubmit {
                    guard commitAdd() else { return }
                    focus = .add
                }
                .onKeyPress(keys: [.delete, .upArrow], phases: .down) { _ in
                    guard addDraft.isEmpty, let last = rows.last else { return .ignored }
                    focus = .row(last.id)
                    return .handled
                }
                .onChange(of: focus) { if focus != .add { commitAdd() } }
        }
        .frame(height: rowHeight)
    }

    @discardableResult
    private func commitAdd() -> Bool {
        let text = addDraft.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !text.isEmpty else { return false }
        addDraft = ""
        add(text)
        return true
    }

    /// Focus the row at `index`, or the add field past the end.
    private func focusAt(_ index: Int, _ rows: [Subtask]) {
        if rows.indices.contains(index) {
            focus = .row(rows[index].id)
        } else if index >= rows.count, !locked {
            focus = .add
        }
    }

    private func reorder(_ id: String, to: Int, _ rows: [Subtask]) {
        var order = rows.map(\.id)
        order.removeAll { $0 == id }
        order.insert(id, at: min(to, order.count))
        pending = Pending(base: subtasks.map(\.id), order: order)
        move(id, to)
    }

    private func drop(_ rows: [Subtask]) {
        guard let landed = drag else { return }
        var still = Transaction()
        still.disablesAnimations = true
        withTransaction(still) {
            drag = nil
            // The list changing under the drag (another device) voids it.
            if landed.to != landed.from, rows.indices.contains(landed.from), rows[landed.from].id == landed.id {
                reorder(landed.id, to: landed.to, rows)
            }
        }
    }

    /// How far the row at `index` is drawn from its slot: the dragged
    /// one by the finger, those between its slot and its target one row
    /// towards the slot it left.
    private func shift(of id: String, at index: Int) -> CGFloat {
        guard let drag else { return 0 }
        if drag.id == id { return drag.dy }
        if index > drag.from, index <= drag.to { return -rowHeight }
        if index < drag.from, index >= drag.to { return rowHeight }
        return 0
    }

    /// Where a grip drag stands: the row that started at `from` of
    /// `count` rows, each `rowHeight` tall, with the finger `delta`
    /// points from where it went down. `dy` is the row's offset, held
    /// inside the list; `to` the index it would land at, changing as it
    /// crosses half a row. The web's `dragTarget`.
    static func dragTarget(from: Int, delta: CGFloat, rowHeight: CGFloat, count: Int) -> (dy: CGFloat, to: Int) {
        guard rowHeight > 0, count > 0 else { return (0, from) }
        let dy = min(max(delta, -CGFloat(from) * rowHeight), CGFloat(count - 1 - from) * rowHeight)
        return (dy, from + Int((dy / rowHeight).rounded()))
    }
}

/// One subtask: its checkbox, its text edited in place, and the grip
/// that drags it.
private struct SubtaskRow: View {
    let sub: Subtask
    var focus: FocusState<ItemSubtasksField.Focus?>.Binding
    let locked: Bool
    let height: CGFloat
    /// This row is the one being dragged.
    let dragged: Bool
    let commit: (String) -> Void
    let toggle: (Bool) -> Void
    /// Move the caret a row up (-1) or down (1).
    let step: (Int) -> Void
    /// Move this row up (-1) or down (1).
    let shift: (Int) -> Void
    /// Remove this row; and the same with the caret going to the row
    /// above, for a Backspace that has it.
    let removeRow: () -> Void
    let removeAndStepUp: () -> Void
    /// The grip dragged this far down from where it was pressed.
    let gripMoved: (CGFloat) -> Void
    let gripReleased: () -> Void
    @State private var draft: String

    init(
        sub: Subtask, focus: FocusState<ItemSubtasksField.Focus?>.Binding, locked: Bool, height: CGFloat,
        dragged: Bool,
        commit: @escaping (String) -> Void, toggle: @escaping (Bool) -> Void,
        step: @escaping (Int) -> Void, shift: @escaping (Int) -> Void,
        removeRow: @escaping () -> Void, removeAndStepUp: @escaping () -> Void,
        gripMoved: @escaping (CGFloat) -> Void, gripReleased: @escaping () -> Void
    ) {
        self.sub = sub
        self.focus = focus
        self.locked = locked
        self.height = height
        self.dragged = dragged
        self.commit = commit
        self.toggle = toggle
        self.step = step
        self.shift = shift
        self.removeRow = removeRow
        self.removeAndStepUp = removeAndStepUp
        self.gripMoved = gripMoved
        self.gripReleased = gripReleased
        _draft = State(initialValue: sub.text)
    }

    private var focused: Bool { focus.wrappedValue == .row(sub.id) }
    private var ink: HierarchicalShapeStyle { sub.done && !focused ? .secondary : .primary }

    var body: some View {
        HStack(alignment: .center, spacing: 12) {
            if locked {
                ItemCheckbox(state: sub.done ? .done : .backlog)
                    .frame(width: 24)
                Text(sub.text)
                    .lineLimit(1)
                    .foregroundStyle(ink)
                    .strikethrough(sub.done)
            } else {
                Button {
                    UIImpactFeedbackGenerator(style: .light).impactOccurred()
                    toggle(!sub.done)
                } label: {
                    ItemCheckbox(state: sub.done ? .done : .backlog)
                        .padding(10)
                        .contentShape(Rectangle())
                        .padding(-10)
                }
                .buttonStyle(.plain)
                .frame(width: 24)
                .accessibilityLabel(sub.done ? String(localized: "Mark not done") : String(localized: "Mark done"))
                field
                grip
            }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .frame(height: height)
        // Lifted off the page while it is dragged.
        .opacity(dragged ? 0.7 : 1)
    }

    private var field: some View {
        TextField("Subtask", text: $draft)
            .foregroundStyle(ink)
            .strikethrough(sub.done && !focused)
            .focused(focus, equals: .row(sub.id))
            .submitLabel(.next)
            // The focus leaving is the commit.
            .onSubmit { step(1) }
            .onKeyPress(keys: [.upArrow, .downArrow]) { press in
                let delta = press.key == .upArrow ? -1 : 1
                if press.modifiers.contains(.option) { shift(delta) } else { step(delta) }
                return .handled
            }
            // A hardware keyboard's Backspace, on a fresh press only: a
            // held one empties this row and stops.
            .onKeyPress(.delete, phases: .down) { _ in
                guard draft.isEmpty else { return .ignored }
                removeAndStepUp()
                return .handled
            }
            .onChange(of: focused) { if !focused { finish() } }
            // Changed elsewhere (another device): shown, unless this
            // one is being typed in.
            .onChange(of: sub.text) { if !focused { draft = sub.text } }
    }

    private var grip: some View {
        Image(systemName: "line.3.horizontal")
            .foregroundStyle(.tertiary)
            .frame(width: 36, height: height)
            .contentShape(Rectangle())
            // From the touch itself, and ahead of the page's scroll
            // and the sheet's pull: a drag that starts on the grip is
            // the row's. Global: the row moves with the drag, so its
            // own space would feed the offset back in.
            .highPriorityGesture(
                DragGesture(minimumDistance: 0, coordinateSpace: .global)
                    .onChanged { gripMoved($0.translation.height) }
                    .onEnded { _ in gripReleased() }
            )
            .accessibilityHidden(true)
    }

    /// The row left: its text kept when it changed, and the row itself
    /// removed when it was emptied.
    private func finish() {
        let trimmed = draft.trimmingCharacters(in: .whitespacesAndNewlines)
        if trimmed.isEmpty {
            removeRow()
        } else if trimmed != sub.text {
            draft = trimmed
            commit(trimmed)
        } else {
            draft = sub.text
        }
    }
}
