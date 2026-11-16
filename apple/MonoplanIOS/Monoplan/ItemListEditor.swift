// The list editor, in place of the item's page for as long as it is
// open: a header (cross, "List" over the item's title, tick), then the
// workspace's lists as full-width rows, the current one ticked, each
// with the workspace's name when it is a shared one, then every other
// workspace's lists under its name, as the web's list picker
// (spec/sharing-plan.md "Cross-doc move"). A tap
// on a row picks it and closes; the cross and the tick both close with
// nothing changed. It takes the sheet over, as the date and place
// editors do, rather than dropping a system menu from the list field:
// the menu covered the item it was moving, and gave no context.
// What a pick does is the caller's: the item page moves the item
// (`item set --list`; across workspaces for another one's list), a
// capture with no item yet aims itself.

import SwiftUI

/// Another workspace's lists, for the list editor: the name over them
/// and whether it is a shared one.
struct ListGroup: Identifiable {
    let docId: String
    let name: String
    let shared: Bool
    let lists: [ListRef]

    var id: String { docId }
}

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
    /// Every other open workspace's lists, under the item's own.
    var others: [ListGroup] = []
    let pick: (ListRef) -> Void
    /// A list of another workspace was picked.
    var pickOther: (ListRef) -> Void = { _ in }
    let done: () -> Void

    var body: some View {
        ScrollView {
            VStack(spacing: 0) {
                ForEach(lists, id: \.listId) { list in
                    row(list, ticked: list.listId == current, badge: workspace, pick: pick)
                    Divider()
                }
                ForEach(others) { group in
                    heading(group)
                    ForEach(group.lists, id: \.key) { list in
                        row(list, ticked: false, badge: nil, pick: pickOther)
                        Divider()
                    }
                }
            }
            .padding(.horizontal)
        }
        .scrollBounceBehavior(.basedOnSize)
        .editorHeader(title: "List", itemTitle: itemTitle, cancel: done, done: done)
    }

    /// Another workspace's name over its lists, with the shared glyph
    /// when it is one.
    private func heading(_ group: ListGroup) -> some View {
        HStack(spacing: 4) {
            if group.shared { SharedIcon() }
            Text(group.name).lineLimit(1)
        }
        .font(.footnote.weight(.semibold))
        .foregroundStyle(.secondary)
        .frame(maxWidth: .infinity, alignment: .leading)
        .padding(.top, 24)
        .padding(.bottom, 6)
        .accessibilityElement(children: .combine)
        .accessibilityAddTraits(.isHeader)
    }

    private func row(
        _ list: ListRef, ticked: Bool, badge: String?, pick: @escaping (ListRef) -> Void
    ) -> some View {
        Button {
            pick(list)
            done()
        } label: {
            HStack(spacing: 6) {
                FieldRow(text: list.name) {
                    ListIcon(listId: list.listId, emoji: list.icon, calendar: list.calendar)
                }
                Spacer(minLength: 8)
                SharedWorkspaceBadge(name: badge)
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
