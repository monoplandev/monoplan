// Capture: the item sheet on an item that does not exist yet, opened by
// the bottom bar's add button with the keyboard up. It is laid out as
// the item's page (`ItemView`) so that becoming one moves nothing: a
// title field, the list it will land in, and the date and place rows.
//
// The rule is the web capture's (js/web/src/TaskDialog.tsx, `promote`):
// nothing is written while the title is being typed, and the item is
// created the moment the title is left with text in it. From then on
// the sheet is that item's page, so every other field is edited on a
// real item by the editors that already exist, and nothing typed is
// lost to a closed sheet or a killed app. "Left" is, on a phone:
//
// - Return: create and close the sheet (the quick capture).
// - The keyboard going down any other way: create, stay on the item.
// - The plus on Date or Place, or the list field: create, and open
//   that editor on it.
// - The sheet closing (drag down, the "x" in the bar): create.
// - The app going to the background: create, stay on the item.
//
// An empty title creates nothing, whichever way the sheet is left.
// A date phrase in the title ("dentist friday 8:30") is written on the
// new item as its date suggestion (`WhenPhrase`), answered on its page.
// The list is the one field held before the item exists: with no title
// yet, the list field opens the list editor (`ItemListEditor`) over the
// capture, and the item is created in what was picked.

import MonoplanMacKit
import SwiftUI

struct ItemCaptureView: View {
    @EnvironmentObject private var model: AppModel
    let target: CaptureTarget
    @Binding var detent: PresentationDetent
    @Environment(\.dismiss) private var dismiss
    @Environment(\.scenePhase) private var scenePhase
    @State private var text = ""
    @FocusState private var focused: Bool
    /// Where the item will be created; the picker changes it.
    @State private var list: ListRef
    /// The item this capture became: the sheet shows its page from
    /// then on.
    @State private var promoted: ItemRef?
    /// The editor to open on the item as it appears.
    @State private var startEditing: ItemView.Editor?
    @State private var error: String?
    /// The list editor has the sheet (no title yet, so no item).
    @State private var pickingList = false
    /// Whether the create has been sent. A reference, so the several
    /// ways of leaving the title that fire together (Return drops the
    /// focus and closes the sheet) see one another's claim at once.
    @State private var draft = Draft()

    private final class Draft {
        var sent = false
    }

    /// What follows the create.
    private enum After {
        /// Show the item's page, in this editor if one is named.
        case stay(ItemView.Editor?)
        /// Close the sheet.
        case close
        /// The sheet has already gone: nothing to show.
        case gone
    }

    init(target: CaptureTarget, detent: Binding<PresentationDetent>) {
        self.target = target
        _detent = detent
        _list = State(initialValue: target.list)
    }

    private var trimmed: String { text.trimmingCharacters(in: .whitespacesAndNewlines) }

    /// The title has been left: create the item if it has a title and
    /// has not been created already. A failed create leaves the capture
    /// as it was, with the reason under the title.
    private func settle(_ after: After) {
        guard promoted == nil, !draft.sent else { return }
        let title = trimmed
        guard !title.isEmpty else { return }
        draft.sent = true
        error = nil
        let suggested = WhenPhrase.find(in: title)?.when
        if case .gone = after {
            model.addItem(text: title, to: list, whenSuggested: suggested)
            return
        }
        model.addItem(text: title, to: list, whenSuggested: suggested) { ref, failure in
            guard let ref else {
                draft.sent = false
                error = failure
                return
            }
            switch after {
            case .stay(let editor):
                startEditing = editor
                promoted = ref
            case .close, .gone:
                dismiss()
            }
        }
    }

    var body: some View {
        Group {
            if let promoted {
                ItemView(ref: promoted, detent: $detent, startEditing: startEditing)
            } else if pickingList {
                ItemListEditor(
                    itemTitle: nil, lists: listOptions, current: list.listId,
                    workspace: model.sharedWorkspaceName(list.docId),
                    pick: { list = $0 },
                    done: { withAnimation(.easeOut(duration: 0.15)) { pickingList = false } })
                    .transition(.opacity)
            } else {
                page.transition(.opacity)
            }
        }
        .onDisappear { settle(.gone) }
        .onChange(of: scenePhase) {
            if scenePhase == .background { settle(.stay(nil)) }
        }
    }

    private var page: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 20) {
                // Wraps like the title it becomes. A wrapping field
                // takes Return as a line break, so Return is caught as
                // it lands: it creates the item and closes the sheet.
                TextField("Title", text: $text, axis: .vertical)
                    .font(.title2.weight(.semibold))
                    .focused($focused)
                    .submitLabel(.done)
                    .onChange(of: text) {
                        guard text.contains(where: \.isNewline) else { return }
                        text = text.split(whereSeparator: \.isNewline).joined(separator: " ")
                        settle(.close)
                    }
                    .onChange(of: focused) {
                        if !focused { settle(.stay(nil)) }
                    }
                if let error {
                    Text(error).font(.footnote).foregroundStyle(.red)
                }
                VStack(alignment: .leading, spacing: 14) {
                    listField
                    // The item's own rows, empty. Their buttons create
                    // the item and open the editor on it, so they wait
                    // for a title.
                    fieldRow("calendar", "Date", what: "date", opens: .date)
                    Divider()
                    fieldRow("map", "Place", what: "place", opens: .place)
                }
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            .padding()
        }
        .scrollBounceBehavior(.basedOnSize)
        .navigationBarTitleDisplayMode(.inline)
        // Where the item's status control and its buttons will be.
        .toolbar {
            SheetCloseItem { dismiss() }
            ToolbarSpacer(.fixed, placement: .topBarLeading)
            SheetStatusLabel(state: "backlog", text: "New item")
        }
        .onAppear { focused = true }
    }

    private func fieldRow(_ systemImage: String, _ name: String, what: String, opens editor: ItemView.Editor) -> some View {
        HStack(spacing: 8) {
            FieldRow(systemImage, text: name, empty: true)
            Spacer(minLength: 8)
            EditBubble(adding: true, what: what) { settle(.stay(editor)) }
                .disabled(trimmed.isEmpty)
        }
        .frame(maxWidth: .infinity, minHeight: 36, alignment: .leading)
    }

    /// The list the item will be created in, as the item page draws
    /// its list field. With a title typed, a tap leaves the title like
    /// any other: the item is created and the list editor opens on it.
    /// With none, the editor takes the sheet to pick where the item
    /// will land.
    private var listField: some View {
        Button {
            if trimmed.isEmpty {
                withAnimation(.easeOut(duration: 0.15)) { pickingList = true }
            } else {
                settle(.stay(.list))
            }
        } label: {
            HStack(spacing: 6) {
                FieldRow(text: list.name) { ListIcon(listId: list.listId, emoji: list.icon) }
                Spacer(minLength: 8)
                SharedWorkspaceBadge(name: model.sharedWorkspaceName(list.docId))
                Image(systemName: "chevron.right")
                    .font(.footnote)
                    .foregroundStyle(.secondary)
            }
            .itemField()
        }
        .buttonStyle(.plain)
        .itemFieldOutset()
        .accessibilityHint("Chooses the list the item goes in")
    }

    /// The workspace's lists, with the target itself when it is not
    /// among them yet (the index has not loaded), so there is always
    /// a row to tick.
    private var listOptions: [ListRef] {
        let lists = model.lists(inDoc: list.docId)
        return lists.contains(where: { $0.listId == list.listId }) ? lists : lists + [list]
    }
}
