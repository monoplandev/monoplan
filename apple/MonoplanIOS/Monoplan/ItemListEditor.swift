// The list editor, in place of the item's page for as long as it is
// open: a header (cross, "List" over the item's title, tick), then the
// workspace's lists as full-width rows, the current one ticked, each
// with the workspace's name when it is a shared one. A tap
// on a row picks it and closes; the cross and the tick both close with
// nothing changed. It takes the sheet over, as the date and place
// editors do, rather than dropping a system menu from the list field:
// the menu covered the item it was moving, and gave no context.
// What a pick does is the caller's: the item page moves the item
// (`item set --list`), a capture with no item yet aims itself.

import SwiftUI

struct ItemListEditor: View {
    /// The item's title, under the header; nil for a capture that has
    /// none yet.
    let itemTitle: String?
    let lists: [ListRef]
    /// The id of the list to tick.
    let current: String
    /// The lists' workspace when it is a shared one, named on every
    /// row; nil for Personal.
    var workspace: String? = nil
    let pick: (ListRef) -> Void
    let done: () -> Void

    var body: some View {
        ScrollView {
            VStack(spacing: 0) {
                ForEach(lists, id: \.listId) { list in
                    row(list)
                    Divider()
                }
            }
            .padding(.horizontal)
        }
        .scrollBounceBehavior(.basedOnSize)
        .editorHeader(title: "List", itemTitle: itemTitle, cancel: done, done: done)
    }

    private func row(_ list: ListRef) -> some View {
        let ticked = list.listId == current
        return Button {
            pick(list)
            done()
        } label: {
            HStack(spacing: 6) {
                FieldRow(text: list.name) { ListIcon(listId: list.listId, emoji: list.icon) }
                Spacer(minLength: 8)
                SharedWorkspaceBadge(name: workspace)
                if ticked {
                    Image(systemName: "checkmark")
                        .font(.body.weight(.semibold))
                        .foregroundStyle(.tint)
                }
            }
            .frame(maxWidth: .infinity, minHeight: 48, alignment: .leading)
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .accessibilityAddTraits(ticked ? .isSelected : [])
    }
}
