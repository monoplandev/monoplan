// What the item sheet's field editors share: the small round button a
// field row ends with, and the header of an editor that has taken the
// sheet over (cross, title over the item's name, tick).

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

/// An editor's header: a cross on the left, the field's name over the
/// item's title, a tick on the right.
struct EditorHeader: View {
    let title: String
    let itemTitle: String
    let cancel: () -> Void
    let done: () -> Void

    var body: some View {
        VStack(spacing: 0) {
            HStack(spacing: 12) {
                button("xmark", label: "Cancel", action: cancel)
                VStack(spacing: 2) {
                    Text(title).font(.headline)
                    Text(itemTitle)
                        .font(.footnote)
                        .foregroundStyle(.secondary)
                        .lineLimit(1)
                }
                .frame(maxWidth: .infinity)
                button("checkmark", label: "Done", action: done)
                    .tint(.accentColor)
            }
            .padding(.horizontal)
            .padding(.top, 20)
            .padding(.bottom, 12)
            Divider()
        }
    }

    private func button(_ systemImage: String, label: String, action: @escaping () -> Void) -> some View {
        Button(action: action) {
            Image(systemName: systemImage)
                .font(.body.weight(.semibold))
                .frame(width: 22, height: 22)
        }
        .buttonStyle(.bordered)
        .buttonBorderShape(.circle)
        .accessibilityLabel(label)
    }
}
