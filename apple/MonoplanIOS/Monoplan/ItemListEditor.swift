// The list editor, in place of the item's page for as long as it is
// open: a header (cross, "List" over the item's title, tick), a search
// box, then one run of full-width rows: the workspace's lists, the
// current one ticked, then every other workspace's lists
// (spec/sharing-plan.md "Cross-doc move"). A shared workspace's list
// names its workspace in the row, after the list's own name, muted;
// Personal's lists name none. Typing in the box narrows the rows to
// the lists (or workspaces) whose name contains it. A tap
// on a row picks it and closes; the cross and the tick both close with
// nothing changed. It takes the sheet over, as the date and place
// editors do, rather than dropping a system menu from the list field:
// the menu covered the item it was moving, and gave no context.
// What a pick does is the caller's: the item page moves the item
// (`item set --list`; across workspaces for another one's list), a
// capture with no item yet aims itself.

import SwiftUI

/// Another workspace's lists, for the list editor: its name and
/// whether it is a shared one (its rows name it only then).
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
    /// The lists' workspace when it is a shared one, named on each of
    /// their rows; nil for Personal.
    var workspace: String? = nil
    /// Every other open workspace's lists, after the item's own.
    var others: [ListGroup] = []
    let pick: (ListRef) -> Void
    /// A list of another workspace was picked.
    var pickOther: (ListRef) -> Void = { _ in }
    let done: () -> Void

    @State private var query = ""
    @ScaledMetric(relativeTo: .body) private var sharedIconSide: CGFloat = 18

    /// One row: a list, the shared workspace it is in (nil: Personal),
    /// and whether it is another workspace's than the item's.
    private struct Entry: Identifiable {
        let list: ListRef
        let workspace: String?
        let other: Bool

        var id: String { "\(list.docId)/\(list.listId)" }
    }

    private var entries: [Entry] {
        lists.map { Entry(list: $0, workspace: workspace, other: false) }
            + others.flatMap { group in
                group.lists.map { Entry(list: $0, workspace: group.shared ? group.name : nil, other: true) }
            }
    }

    /// The rows the typed text leaves: those whose list or workspace
    /// name contains it, case and accents aside.
    private var matches: [Entry] {
        let typed = query.trimmingCharacters(in: .whitespaces)
        guard !typed.isEmpty else { return entries }
        return entries.filter {
            $0.list.name.localizedStandardContains(typed)
                || ($0.workspace?.localizedStandardContains(typed) ?? false)
        }
    }

    var body: some View {
        let matches = matches
        VStack(spacing: 0) {
            searchBox
            ScrollView {
                LazyVStack(spacing: 0) {
                    ForEach(matches) { entry in
                        row(entry)
                        Divider()
                    }
                    if matches.isEmpty {
                        Text("No lists match")
                            .foregroundStyle(.secondary)
                            .frame(maxWidth: .infinity, minHeight: 48)
                    }
                }
                .padding(.horizontal)
            }
            .scrollBounceBehavior(.basedOnSize)
            .scrollDismissesKeyboard(.immediately)
        }
        .editorHeader(title: "List", itemTitle: itemTitle, cancel: done, done: done)
    }

    /// The place editor's box. Not focused on arrival: the keyboard
    /// would cover half the lists for someone who only means to tap.
    private var searchBox: some View {
        HStack(spacing: 8) {
            Image(systemName: "magnifyingglass").foregroundStyle(.secondary)
            TextField("Search lists", text: $query)
                .submitLabel(.done)
                .autocorrectionDisabled()
                .textInputAutocapitalization(.never)
            if !query.isEmpty {
                Button {
                    query = ""
                } label: {
                    Image(systemName: "xmark.circle.fill").foregroundStyle(.tertiary)
                }
                .buttonStyle(.plain)
                .accessibilityLabel("Clear text")
            }
        }
        .padding(.horizontal, 12)
        .frame(minHeight: 40)
        .background(.quaternary.opacity(0.5), in: RoundedRectangle(cornerRadius: 10, style: .continuous))
        .padding(.horizontal)
        .padding(.vertical, 8)
    }

    private func row(_ entry: Entry) -> some View {
        let list = entry.list
        let ticked = !entry.other && list.listId == current
        return Button {
            if entry.other { pickOther(list) } else { pick(list) }
            done()
        } label: {
            HStack(spacing: 8) {
                // `FieldRow`'s layout, with the workspace on the name's line.
                HStack(alignment: .firstTextBaseline, spacing: 12) {
                    ListIcon(listId: list.listId, emoji: list.icon, calendar: list.calendar)
                        .foregroundStyle(.secondary)
                        .frame(width: 24)
                    Text(list.name).lineLimit(1)
                    if let workspace = entry.workspace {
                        workspaceTag(workspace)
                    }
                }
                Spacer(minLength: 0)
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

    /// A shared workspace, after its list's name: the shared glyph and
    /// the workspace's name at the list name's size, muted. The first
    /// thing to truncate when the row is short of room.
    private func workspaceTag(_ name: String) -> some View {
        HStack(spacing: 5) {
            Image("Users")
                .resizable()
                .scaledToFit()
                .frame(width: sharedIconSide, height: sharedIconSide)
            Text(name).lineLimit(1)
        }
        .foregroundStyle(.secondary)
        .layoutPriority(-1)
        .accessibilityElement(children: .ignore)
        .accessibilityLabel("Shared workspace \(name)")
    }
}
