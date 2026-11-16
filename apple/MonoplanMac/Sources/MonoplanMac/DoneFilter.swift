// The Done lens's filter: which workspaces and which lists its finished
// items are drawn from, the web Done view's filter
// (js/web/src/doneFilter.ts, DoneFilterMenu.tsx) and the iOS sheet
// (`DoneFilterView.swift`) as a popover under the toolbar's filter
// button, there while Done is shown (`MainSplitViewController`). Two
// facets of checkboxes, ANDed; none checked in a facet filters nothing,
// so the default is every open workspace's finished items.
//
// The Lists section offers the lists of the checked workspaces (all of
// them when none is checked). Archived lists stay out of the way until
// searched for or checked, and carry an Archived badge when shown.
// Searching narrows the lists alone, so the Workspaces section steps
// aside while there is a query.
//
// Device-local, like the Inbox order: a filter is "what am I looking at
// right now", not a setting to sync. Kept in `UserDefaults` so it
// survives a relaunch. The state and its narrowing live in
// `MainWindowModel` (`doneFilter`); this is the surface.

import MonoplanCore
import SwiftUI

/// One workspace the filter offers.
struct DoneWorkspaceOption: Identifiable {
    let docId: String
    let name: String
    /// Any workspace but Personal, marked with the shared glyph.
    let shared: Bool

    var id: String { docId }
}

/// One list the filter offers. `inbox` exists in every workspace, so a
/// list is identified by doc and id together.
struct DoneListOption: Identifiable {
    let docId: String
    let listId: String
    let name: String
    let icon: String?
    /// The list's shared workspace; nil for Personal.
    let workspace: String?
    let archived: Bool

    /// What `DoneFilter.lists` holds: `<doc id>/<list id>`.
    var key: String { "\(docId)/\(listId)" }
    var id: String { key }
}

/// The filter's place in `UserDefaults`.
enum DoneFilterStore {
    private static let key = "MonoplanDoneFilter"

    static func load() -> DoneFilter {
        let stored = UserDefaults.standard.dictionary(forKey: key)
        return DoneFilter(
            docs: stored?["docs"] as? [String] ?? [],
            lists: stored?["lists"] as? [String] ?? [])
    }

    static func save(_ filter: DoneFilter) {
        if filter.isEmpty {
            UserDefaults.standard.removeObject(forKey: key)
        } else {
            UserDefaults.standard.set(["docs": filter.docs, "lists": filter.lists], forKey: key)
        }
    }
}

extension DoneFilter {
    /// How many checks it holds; 0 is unfiltered.
    var count: Int { docs.count + lists.count }
    var isEmpty: Bool { count == 0 }
}

extension Array where Element == String {
    /// `id` flipped: removed when present, else appended.
    mutating func toggle(_ id: String) {
        if let at = firstIndex(of: id) { remove(at: at) } else { append(id) }
    }
}

/// The popover: a search box for the lists, the two facets, and Clear.
struct DoneFilterForm: View {
    @ObservedObject var model: MainWindowModel
    @State private var query = ""

    var body: some View {
        let filter = model.doneFilter
        let workspaces = model.doneWorkspaceOptions
        let typed = query.trimmingCharacters(in: .whitespaces)
        let lists = shownLists(typed, checked: filter.lists)
        VStack(alignment: .leading, spacing: 0) {
            TextField("Search lists", text: $query)
                .textFieldStyle(.roundedBorder)
                .padding(12)
            ScrollView {
                VStack(alignment: .leading, spacing: 6) {
                    // Only worth offering when there is more than one.
                    if workspaces.count > 1, typed.isEmpty {
                        heading(Text("Workspaces"))
                        ForEach(workspaces) { workspace in
                            row(checked: filter.docs.contains(workspace.docId)) {
                                model.toggleDoneWorkspace(workspace.docId)
                            } label: {
                                Text(verbatim: workspace.name).lineLimit(1)
                                if workspace.shared { sharedGlyph }
                            }
                        }
                        heading(Text("Lists")).padding(.top, 6)
                    }
                    ForEach(lists) { list in
                        row(checked: filter.lists.contains(list.key)) {
                            model.toggleDoneList(list.key)
                        } label: {
                            // A list's emoji leads its name, as in the sidebar.
                            Text(verbatim: [list.icon, list.name].compactMap { $0 }
                                .filter { !$0.isEmpty }.joined(separator: " "))
                                .lineLimit(1)
                            if list.archived {
                                Text("Archived")
                                    .font(.caption)
                                    .foregroundStyle(.secondary)
                                    .padding(.horizontal, 6)
                                    .padding(.vertical, 1)
                                    .background(.quaternary, in: Capsule())
                            }
                            if let workspace = list.workspace {
                                // First to truncate when short of room.
                                HStack(alignment: .firstTextBaseline, spacing: 4) {
                                    sharedGlyph
                                    Text(verbatim: workspace).lineLimit(1)
                                }
                                .foregroundStyle(.secondary)
                                .layoutPriority(-1)
                            }
                        }
                    }
                    if lists.isEmpty {
                        Text("No lists match").foregroundStyle(.secondary)
                    }
                }
                .frame(maxWidth: .infinity, alignment: .leading)
                .padding(.horizontal, 12)
                .padding(.bottom, 10)
            }
            .frame(maxHeight: 320)
            Divider()
            HStack {
                Spacer()
                Button("Clear") { model.clearDoneFilter() }
                    .disabled(filter.isEmpty)
            }
            .padding(10)
        }
        .frame(width: 280)
    }

    /// The Lists rows for what is typed. With nothing typed, archived
    /// lists are left out unless checked: a checked one must stay in
    /// sight to be unchecked. A query searches every list, archived
    /// included, by its name or its workspace's, case and accents aside.
    private func shownLists(_ typed: String, checked: [String]) -> [DoneListOption] {
        let offered = model.doneListOptions
        if typed.isEmpty {
            return offered.filter { !$0.archived || checked.contains($0.key) }
        }
        return offered.filter {
            $0.name.localizedStandardContains(typed)
                || ($0.workspace?.localizedStandardContains(typed) ?? false)
        }
    }

    private func heading(_ text: Text) -> some View {
        text.font(.footnote).fontWeight(.semibold).foregroundStyle(.secondary)
    }

    /// One checkbox with its label.
    private func row<Label: View>(
        checked: Bool, toggle: @escaping () -> Void, @ViewBuilder label: () -> Label
    ) -> some View {
        Toggle(isOn: Binding(get: { checked }, set: { _ in toggle() })) {
            HStack(alignment: .firstTextBaseline, spacing: 6) { label() }
        }
        .toggleStyle(.checkbox)
    }

    private var sharedGlyph: some View {
        PanelGlyph(image: NavArt.shared, size: 13).accessibilityHidden(true)
    }
}
