// The item panel's subtasks (spec/subtasks.md): text nodes with done /
// not done, owned by the item. A column of rows, each a checkbox and a
// one-line text field, over a trailing "Add subtask" field that keeps
// the caret after Return so a list can be typed straight down. The
// web's `SubtasksField.tsx`, key for key:
//
// Return commits and moves down, Backspace in an emptied row removes it
// (the only way to remove one), Up / Down walk the rows, Option+Up /
// Option+Down reorder, Escape puts the text back.
//
// The pointer reorders by the grip at a row's end: the row follows the
// pointer, the rows it passes step aside, and the release is one move.
// Plain SwiftUI offsets over rows of one height, no table and no system
// drag session (spec/mac-dnd-plan.md is about the item lists).

import MonoplanCore
import SwiftUI

struct SubtasksField: View {
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

    static let rowHeight: CGFloat = 28

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
                    sub: sub, focus: $focus, locked: locked,
                    dragged: drag?.id == sub.id, dragging: drag != nil,
                    commit: { setText(sub.id, $0) },
                    toggle: { setDone(sub.id, $0) },
                    step: { focusAt(index + $0, rows) },
                    shift: {
                        let to = index + $0
                        if rows.indices.contains(to) { reorder(sub.id, to: to, rows) }
                    },
                    removeRow: {
                        // The row above takes the caret; the add field
                        // when it was the first.
                        focus = index > 0 ? .row(rows[index - 1].id) : .add
                        remove(sub.id)
                    },
                    gripMoved: { delta in
                        let from = drag?.id == sub.id ? drag?.from ?? index : index
                        let target = Self.dragTarget(
                            from: from, delta: delta, rowHeight: Self.rowHeight, count: rows.count)
                        drag = Drag(id: sub.id, from: from, to: target.to, dy: target.dy)
                    },
                    gripReleased: { drop(rows) }
                )
                .offset(y: shift(of: sub.id, at: index))
                .zIndex(drag?.id == sub.id ? 1 : 0)
                // The rows stepping aside ease; the dragged one tracks
                // the pointer, and nothing eases as it lands.
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
        HStack(alignment: .center, spacing: 5) {
            Image(systemName: "plus")
                .foregroundStyle(.tertiary)
                .frame(width: 20)
            // The placeholder is our own `Text`, in the ink of the
            // panel's unset "Date" and "Place" rows: a field's own
            // prompt is drawn in the system's placeholder colour.
            ZStack(alignment: .leading) {
                if addDraft.isEmpty {
                    Text("Add subtask")
                        .lineLimit(1)
                        .foregroundStyle(.tertiary)
                        .allowsHitTesting(false)
                        .accessibilityHidden(true)
                }
                TextField("Add subtask", text: $addDraft, prompt: Text(verbatim: ""))
                    .textFieldStyle(.plain)
                    .focused($focus, equals: .add)
                    .onSubmit {
                        commitAdd()
                        focus = .add
                    }
                    .onKeyPress(keys: [.delete, .upArrow], phases: .down) { _ in
                        guard addDraft.isEmpty, let last = rows.last else { return .ignored }
                        focus = .row(last.id)
                        return .handled
                    }
                    .onKeyPress(.escape) {
                        guard !addDraft.isEmpty else { return .ignored }
                        addDraft = ""
                        return .handled
                    }
                    .onChange(of: focus) { if focus != .add { commitAdd() } }
            }
        }
        .frame(height: Self.rowHeight)
    }

    private func commitAdd() {
        let text = addDraft.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !text.isEmpty else { return }
        addDraft = ""
        add(text)
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
    /// one by the pointer, those between its slot and its target one row
    /// towards the slot it left.
    private func shift(of id: String, at index: Int) -> CGFloat {
        guard let drag else { return 0 }
        if drag.id == id { return drag.dy }
        if index > drag.from, index <= drag.to { return -Self.rowHeight }
        if index < drag.from, index >= drag.to { return Self.rowHeight }
        return 0
    }

    /// Where a grip drag stands: the row that started at `from` of
    /// `count` rows, each `rowHeight` tall, with the pointer `delta`
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
/// that drags it, shown under the pointer.
private struct SubtaskRow: View {
    let sub: Subtask
    var focus: FocusState<SubtasksField.Focus?>.Binding
    let locked: Bool
    /// This row is the one being dragged; some row is.
    let dragged: Bool
    let dragging: Bool
    let commit: (String) -> Void
    let toggle: (Bool) -> Void
    /// Move the caret a row up (-1) or down (1).
    let step: (Int) -> Void
    /// Move this row up (-1) or down (1).
    let shift: (Int) -> Void
    let removeRow: () -> Void
    /// The grip dragged this far down from where it was pressed.
    let gripMoved: (CGFloat) -> Void
    let gripReleased: () -> Void
    @State private var draft: String
    @State private var hovered = false

    init(
        sub: Subtask, focus: FocusState<SubtasksField.Focus?>.Binding, locked: Bool,
        dragged: Bool, dragging: Bool,
        commit: @escaping (String) -> Void, toggle: @escaping (Bool) -> Void,
        step: @escaping (Int) -> Void, shift: @escaping (Int) -> Void,
        removeRow: @escaping () -> Void,
        gripMoved: @escaping (CGFloat) -> Void, gripReleased: @escaping () -> Void
    ) {
        self.sub = sub
        self.focus = focus
        self.locked = locked
        self.dragged = dragged
        self.dragging = dragging
        self.commit = commit
        self.toggle = toggle
        self.step = step
        self.shift = shift
        self.removeRow = removeRow
        self.gripMoved = gripMoved
        self.gripReleased = gripReleased
        _draft = State(initialValue: sub.text)
    }

    private var focused: Bool { focus.wrappedValue == .row(sub.id) }
    private var ink: HierarchicalShapeStyle { sub.done && !focused ? .secondary : .primary }

    var body: some View {
        HStack(alignment: .center, spacing: 5) {
            if locked {
                ItemCheckbox(state: sub.done ? .done : .backlog)
                    .frame(width: 20)
                Text(verbatim: sub.text)
                    .lineLimit(1)
                    .foregroundStyle(ink)
                    .strikethrough(sub.done)
                    .textSelection(.enabled)
            } else {
                Button {
                    toggle(!sub.done)
                } label: {
                    ItemCheckbox(state: sub.done ? .done : .backlog).contentShape(Rectangle())
                }
                .buttonStyle(.plain)
                .frame(width: 20)
                .accessibilityLabel(sub.done ? Text("Mark as not done") : Text("Mark as done"))
                field
                grip
            }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .frame(height: SubtasksField.rowHeight)
        .contentShape(Rectangle())
        .onHover { hovered = $0 }
    }

    private var field: some View {
        TextField("Subtask", text: $draft)
            .textFieldStyle(.plain)
            .foregroundStyle(ink)
            .strikethrough(sub.done && !focused)
            .focused(focus, equals: .row(sub.id))
            // The focus leaving is the commit.
            .onSubmit { step(1) }
            .onKeyPress(keys: [.upArrow, .downArrow]) { press in
                let delta = press.key == .upArrow ? -1 : 1
                if press.modifiers.contains(.option) { shift(delta) } else { step(delta) }
                return .handled
            }
            // A fresh press only: a held Backspace empties this row and
            // stops, it does not run on up the list.
            .onKeyPress(.delete, phases: .down) { _ in
                guard draft.isEmpty else { return .ignored }
                removeRow()
                return .handled
            }
            // Only the edit is cancelled; a second Escape is the panel's.
            .onKeyPress(.escape) {
                guard draft != sub.text else { return .ignored }
                draft = sub.text
                return .handled
            }
            .onChange(of: focused) { if !focused { finish() } }
            // Changed elsewhere (another device, an undo): shown,
            // unless this one is being typed in.
            .onChange(of: sub.text) { if !focused { draft = sub.text } }
    }

    /// Pointer only: the keyboard reorders with Option+Up / Down.
    private var grip: some View {
        Image(systemName: "line.3.horizontal")
            .font(.system(size: 11, weight: .medium))
            .foregroundStyle(.tertiary)
            .frame(width: 20, height: SubtasksField.rowHeight)
            .contentShape(Rectangle())
            .opacity(dragged || (hovered && !dragging) ? 1 : 0)
            // Global: the row moves with the drag, so its own space
            // would feed the offset back into the translation.
            .gesture(
                DragGesture(minimumDistance: 1, coordinateSpace: .global)
                    .onChanged { gripMoved($0.translation.height) }
                    .onEnded { _ in gripReleased() }
            )
            .accessibilityHidden(true)
    }

    /// An emptied row is not a delete on its own (Backspace is): the
    /// text comes back.
    private func finish() {
        let trimmed = draft.trimmingCharacters(in: .whitespacesAndNewlines)
        if trimmed.isEmpty || trimmed == sub.text {
            draft = sub.text
        } else {
            draft = trimmed
            commit(trimmed)
        }
    }
}
