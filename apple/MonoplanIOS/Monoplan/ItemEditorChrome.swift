// What the item sheet's views share: the small round button a field
// row ends with, the navigation bar of an editor that has taken the
// sheet over (cross, title over the item's name, tick), and the bar's
// close button and plain status label. The sheet's content sits in one
// `NavigationStack` (`ItemSheet`), so all of these are toolbar items
// and the system draws them as Liquid Glass.

import SwiftUI

/// The round button at the trailing edge of a field row: a plus to add
/// a value, a pencil to change one. The row itself is display only.
struct EditBubble: View {
    /// The field has no value yet.
    let adding: Bool
    /// What it edits, for VoiceOver: "date", "place".
    let what: String
    let action: () -> Void

    var body: some View {
        Button(action: action) {
            Image(systemName: adding ? "plus" : "pencil")
                .font(.footnote.weight(.semibold))
                .frame(width: 18, height: 18)
        }
        .buttonStyle(.bordered)
        .buttonBorderShape(.circle)
        .accessibilityLabel(adding ? "Add \(what)" : "Change \(what)")
    }
}

extension View {
    /// An editor's navigation bar: a cross on the left, the field's
    /// name over the item's title (nil where the editor shows the
    /// title itself), a tick on the right, off while there is nothing
    /// valid to keep.
    func editorHeader(
        title: String, itemTitle: String? = nil, canFinish: Bool = true,
        cancel: @escaping () -> Void, done: @escaping () -> Void
    ) -> some View {
        navigationTitle(title)
            .navigationSubtitle(itemTitle ?? "")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button("Cancel", systemImage: "xmark", role: .cancel, action: cancel)
                }
                ToolbarItem(placement: .confirmationAction) {
                    Button("Done", systemImage: "checkmark", role: .confirm, action: done)
                        .disabled(!canFinish)
                }
            }
    }
}

/// The "x" at the top right of the item sheet's pages: closes the
/// sheet, for anyone who does not find the drag down.
struct SheetCloseItem: ToolbarContent {
    let action: () -> Void

    var body: some ToolbarContent {
        ToolbarItem(placement: .topBarTrailing) {
            Button(role: .close, action: action)
        }
    }
}

/// A checkbox that does nothing and a word beside it, at the top left
/// of the sheet where an item's status control goes: "New item" on a
/// capture, "Binned" on a binned item. No glass behind it: it is not a
/// button.
struct SheetStatusLabel: ToolbarContent {
    let state: String
    let text: String

    var body: some ToolbarContent {
        ToolbarItem(placement: .topBarLeading) {
            HStack(spacing: 10) {
                ItemCheckbox(state: state)
                Text(text)
                    .font(.subheadline.weight(.medium))
                    .foregroundStyle(.secondary)
            }
            .fixedSize()
        }
        .sharedBackgroundVisibility(.hidden)
    }
}
