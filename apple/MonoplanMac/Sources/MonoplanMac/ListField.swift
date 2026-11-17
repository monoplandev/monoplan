// The item panel's list row, and its picker. At rest it is the item's
// list drawn as a field, as on the iOS page (`ItemView.listField`):
// full width, a faint fill, the panel's box (`panelBox`); the date
// and place rows under it have the fill only under the pointer. A click turns the
// field into a search box and drops the lists under it, floating over
// the panel's rows as the place field's list does (`PlaceField.swift`):
// one run of rows, the item's workspace's lists with the current one
// ticked, then every other workspace's, as the iOS list editor
// (apple/MonoplanIOS/Monoplan/ItemListEditor.swift). A shared
// workspace's list names its workspace in the row, after the list's
// own name, muted; Personal's lists name none. Typing narrows the rows
// to the lists (or workspaces) whose name contains the text.
//
// A click on a row moves the item there, across workspaces when the
// list is another one's (spec/sharing-plan.md "Cross-doc move"). The
// arrow keys move the highlight; Return takes the highlighted row,
// else the first one left by what was typed; Escape, or the caret
// leaving the box, shuts it with nothing changed. A binned item stays
// put.

import MonoplanCore
import SwiftUI

struct ListField: View {
    let item: ItemInfo
    /// The item's workspace when it is a shared one.
    let workspaceName: String?
    /// The item's workspace's lists.
    let lists: [PanelList]
    /// Every other open workspace's lists, after the item's own.
    let others: [PanelListGroup]
    /// Another list of the item's workspace was picked: its id.
    let pick: (String) -> Void
    /// A list of another workspace was picked: the workspace (as a
    /// typed call takes it, nil being Home) and the list's id.
    let pickAcross: (String?, String) -> Void

    @State private var picking = false
    @State private var query = ""
    @State private var highlight: Int?
    @FocusState private var focused: Bool
    /// The search box has had the caret since it opened: only then is
    /// the caret leaving it a reason to shut.
    @State private var armed = false

    /// One row: a list, the workspace it is in when that is another
    /// than the item's, and the workspace's name when it is shared.
    private struct Entry: Identifiable {
        let list: PanelList
        let other: Bool
        let workspace: String?
        let workspaceName: String?

        var id: String { "\(other ? workspace ?? "home" : "")/\(list.id)" }
    }

    private var dated: Bool { item.item.when != nil }

    private var entries: [Entry] {
        lists.map { Entry(list: $0, other: false, workspace: nil, workspaceName: workspaceName) }
            + others.flatMap { group in
                group.lists.map {
                    Entry(
                        list: $0, other: true, workspace: group.workspace,
                        workspaceName: group.shared ? group.name : nil)
                }
            }
    }

    /// The rows the typed text leaves: those whose list or workspace
    /// name contains it, case and accents aside.
    private var matches: [Entry] {
        let typed = query.trimmingCharacters(in: .whitespaces)
        guard !typed.isEmpty else { return entries }
        return entries.filter {
            $0.list.name.localizedStandardContains(typed)
                || ($0.workspaceName?.localizedStandardContains(typed) ?? false)
        }
    }

    var body: some View {
        Group {
            if picking {
                // The list hangs from a zero-height strip on the box's
                // bottom edge, so it takes no room and grows downwards
                // at its own height (as `PlaceField`'s). The panel
                // puts this row above its later ones (`zIndex`).
                searchBox.overlay(alignment: .bottomLeading) {
                    Color.clear
                        .frame(height: 0)
                        .overlay(alignment: .topLeading) {
                            dropdown.padding(.top, 6)
                        }
                }
            } else {
                shown
            }
        }
        .onChange(of: query) { highlight = nil }
        .onChange(of: focused) {
            if focused {
                armed = true
            } else if armed {
                finish()
            }
        }
    }

    // MARK: Rows

    /// The item's list, and its workspace's name when that is a shared
    /// one: the shared glyph, then the name in the same font, muted.
    private var shown: some View {
        Button {
            picking = true
        } label: {
            HStack(alignment: .firstTextBaseline, spacing: 10) {
                Group {
                    if let emoji = item.listIcon {
                        Text(verbatim: emoji)
                    } else {
                        ItemPanel.listIcon(item.item.listId, dated: dated)
                    }
                }
                .foregroundStyle(.secondary)
                .frame(width: 20)
                Text(verbatim: item.listLabel).lineLimit(1)
                if let workspaceName {
                    WorkspaceTag(name: workspaceName, highlighted: false)
                }
                Spacer(minLength: 8)
                Image(systemName: "chevron.up.chevron.down")
                    .font(.footnote)
                    .foregroundStyle(.secondary)
            }
            .panelBox()
        }
        .buttonStyle(.plain)
        .disabled(item.item.binnedAt != nil)
        .accessibilityHint(Text("Moves the item to another list"))
    }

    private var searchBox: some View {
        HStack(alignment: .firstTextBaseline, spacing: 10) {
            Image(systemName: "magnifyingglass")
                .foregroundStyle(.secondary)
                .frame(width: 20)
            TextField("Search lists", text: $query)
                .textFieldStyle(.plain)
                .autocorrectionDisabled()
                .focused($focused)
                .onSubmit(submit)
                .onExitCommand(perform: finish)
                .onKeyPress(.downArrow) { step(1) }
                .onKeyPress(.upArrow) { step(-1) }
        }
        .panelBox()
        // The caret, a turn after the box appears: asked for while the
        // field is still being installed, the window hands the caret to
        // its first field instead (the item's title).
        .onAppear { DispatchQueue.main.async { focused = true } }
    }

    /// The lists, in a box under the search box and as wide: as tall
    /// as its rows up to a cap, scrolling past it.
    private var dropdown: some View {
        let rows = matches
        return ScrollViewReader { proxy in
            ScrollView {
                VStack(alignment: .leading, spacing: 0) {
                    ForEach(Array(rows.enumerated()), id: \.element.id) { index, entry in
                        ListRow(
                            entry.list, dated: dated, workspaceName: entry.workspaceName,
                            ticked: !entry.other && entry.list.id == item.item.listId,
                            highlighted: highlight == index
                        ) {
                            take(entry)
                        }
                        .onHover { if $0 { highlight = index } }
                        .id(entry.id)
                    }
                    if rows.isEmpty {
                        Text("No lists match")
                            .foregroundStyle(.secondary)
                            .padding(.horizontal, 8)
                            .padding(.vertical, 4)
                    }
                }
                .padding(4)
                .frame(maxWidth: .infinity, alignment: .leading)
            }
            .frame(maxHeight: 320)
            .fixedSize(horizontal: false, vertical: true)
            // The arrow keys can walk the highlight out of sight.
            .onChange(of: highlight) {
                guard let highlight, rows.indices.contains(highlight) else { return }
                proxy.scrollTo(rows[highlight].id)
            }
        }
        .background(Color(nsColor: .controlBackgroundColor), in: RoundedRectangle(cornerRadius: 8, style: .continuous))
        .overlay(RoundedRectangle(cornerRadius: 8, style: .continuous).strokeBorder(.separator))
        .shadow(color: .black.opacity(0.18), radius: 10, y: 4)
    }

    // MARK: Picking

    private func step(_ by: Int) -> KeyPress.Result {
        let count = matches.count
        guard count > 0 else { return .ignored }
        highlight = max(0, min(count - 1, (highlight.map { $0 + by }) ?? (by > 0 ? 0 : count - 1)))
        return .handled
    }

    private func submit() {
        let rows = matches
        if let highlight, rows.indices.contains(highlight) {
            take(rows[highlight])
        } else if !query.trimmingCharacters(in: .whitespaces).isEmpty, let first = rows.first {
            take(first)
        } else {
            finish()
        }
    }

    private func take(_ entry: Entry) {
        if entry.other {
            pickAcross(entry.workspace, entry.list.id)
        } else if entry.list.id != item.item.listId {
            pick(entry.list.id)
        }
        finish()
    }

    /// Back to the field as it was: no text, no list.
    private func finish() {
        query = ""
        highlight = nil
        picking = false
        armed = false
        focused = false
    }
}

/// One list in the picker: its icon, its name, its workspace after it
/// when that is a shared one, and a tick on the item's own.
private struct ListRow: View {
    let list: PanelList
    let dated: Bool
    let workspaceName: String?
    let ticked: Bool
    let highlighted: Bool
    let action: () -> Void

    init(
        _ list: PanelList, dated: Bool, workspaceName: String?, ticked: Bool, highlighted: Bool,
        action: @escaping () -> Void
    ) {
        self.list = list
        self.dated = dated
        self.workspaceName = workspaceName
        self.ticked = ticked
        self.highlighted = highlighted
        self.action = action
    }

    var body: some View {
        Button(action: action) {
            HStack(alignment: .firstTextBaseline, spacing: 8) {
                Group {
                    if let emoji = list.icon {
                        Text(verbatim: emoji)
                    } else {
                        ItemPanel.listIcon(list.id, dated: dated)
                    }
                }
                .foregroundStyle(highlighted ? .white : .secondary)
                .frame(width: 20)
                Text(verbatim: list.name)
                    .lineLimit(1)
                    .foregroundStyle(highlighted ? .white : .primary)
                if let workspaceName {
                    WorkspaceTag(name: workspaceName, highlighted: highlighted)
                }
                Spacer(minLength: 0)
                if ticked {
                    Image(systemName: "checkmark")
                        .font(.footnote.weight(.semibold))
                        .foregroundStyle(highlighted ? .white : .secondary)
                }
            }
            .padding(.horizontal, 4)
            .padding(.vertical, 4)
            .frame(maxWidth: .infinity, alignment: .leading)
            .background(
                highlighted ? Color.accentColor : .clear,
                in: RoundedRectangle(cornerRadius: 5, style: .continuous))
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .accessibilityAddTraits(ticked ? .isSelected : [])
    }
}

/// A shared workspace after its list's name: the shared glyph, then
/// the name in the list name's font, muted. First to truncate.
private struct WorkspaceTag: View {
    let name: String
    let highlighted: Bool

    var body: some View {
        HStack(alignment: .firstTextBaseline, spacing: 4) {
            PanelGlyph(image: NavArt.shared, size: 13).accessibilityHidden(true)
            Text(verbatim: name).lineLimit(1)
        }
        .foregroundStyle(highlighted ? AnyShapeStyle(.white.opacity(0.8)) : AnyShapeStyle(.secondary))
        .layoutPriority(-1)
    }
}
