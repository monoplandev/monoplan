// What the item sheet's views share: the small round button a field
// row ends with, the navigation bar of an editor that has taken the
// sheet over (cross, title over the item's name, tick), and the bar's
// close button and plain status label. The sheet's content sits in one
// `NavigationStack` (`ItemSheet`), so all of these are toolbar items
// and the system draws them as Liquid Glass.

import SwiftUI

/// The round pencil at the trailing edge of a field row that has a
/// value: the one way to change it, the row itself display only. A row
/// with no value has no button and is the target itself (`addsOnTap`).
struct EditBubble: View {
    /// What it edits, for VoiceOver: "date", "place".
    let what: String
    let action: () -> Void

    var body: some View {
        Button(action: action) {
            Image(systemName: "pencil")
                .font(.footnote.weight(.semibold))
                .frame(width: 18, height: 18)
        }
        .buttonStyle(.bordered)
        .buttonBorderShape(.circle)
        .accessibilityLabel("Change \(what)")
    }
}

extension View {
    /// A field row with no value is all one target, with no button of
    /// its own: a tap anywhere on it opens the editor, and VoiceOver
    /// reads the row as the button ("Add date"). Once it has a value
    /// only the pencil edits, so a set value is not changed by a stray
    /// tap.
    @ViewBuilder
    func addsOnTap(_ empty: Bool, what: String, action: @escaping () -> Void) -> some View {
        if empty {
            contentShape(Rectangle())
                .onTapGesture(perform: action)
                .accessibilityElement(children: .ignore)
                .accessibilityLabel("Add \(what)")
                .accessibilityAddTraits(.isButton)
        } else {
            self
        }
    }

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

/// The "x" at the top left of the item sheet's pages, where an editor
/// has its cross: closes the sheet, for anyone who does not find the
/// drag down. Declared first, so it is the item in the corner.
struct SheetCloseItem: ToolbarContent {
    let action: () -> Void

    var body: some ToolbarContent {
        ToolbarItem(placement: .topBarLeading) {
            Button(role: .close, action: action)
        }
    }
}

/// A checkbox that does nothing and a word beside it, after the close
/// button, where an item's status control goes: "New item" on a
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
