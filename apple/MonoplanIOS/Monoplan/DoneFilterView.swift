// The Done page's filter: which workspaces and which lists its
// finished items are drawn from, the web Done view's filter
// (js/web/src/doneFilter.ts, DoneFilterMenu.tsx) as a sheet. Two
// facets of checkmarks, ANDed; none checked in a facet filters nothing,
// so the default is every open workspace's finished items. The web has
// a menu of two submenus; a sheet here, since a menu closes on each
// tap and has no room for the lists' search box.
//
// The Lists section offers the lists of the checked workspaces (all of
// them when none is checked). Archived lists stay out of the way until
// searched for or checked, and carry an Archived badge when shown.
// Searching narrows the lists alone, so the Workspaces section steps
// aside while there is a query.
//
// Device-local, like the Inbox order: a filter is "what am I looking at
// right now", not a setting to sync. Kept in `UserDefaults` so it
// survives a relaunch. The state and its narrowing live in `AppModel`
// (`doneFilter`); this is the surface.

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
    private static let key = "doneFilter"

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

struct DoneFilterView: View {
    @EnvironmentObject private var model: AppModel
    @Environment(\.dismiss) private var dismiss
    @State private var query = ""
    @ScaledMetric(relativeTo: .body) private var sharedIconSide: CGFloat = 18

    var body: some View {
        let filter = model.doneFilter
        let workspaces = model.doneWorkspaceOptions
        let typed = query.trimmingCharacters(in: .whitespaces)
        let lists = shownLists(typed, checked: filter.lists)
        NavigationStack {
            List {
                // Only worth offering when there is more than one.
                if workspaces.count > 1, typed.isEmpty {
                    Section("Workspaces") {
                        ForEach(workspaces) { workspace in
                            row(checked: filter.docs.contains(workspace.docId)) {
                                model.toggleDoneWorkspace(workspace.docId)
                            } label: {
                                Text(workspace.name).lineLimit(1)
                                if workspace.shared { sharedGlyph }
                            }
                        }
                    }
                }
                Section("Lists") {
                    ForEach(lists) { list in
                        row(checked: filter.lists.contains(list.key)) {
                            model.toggleDoneList(list.key)
                        } label: {
                            ListIcon(listId: list.listId, emoji: list.icon)
                                .foregroundStyle(.secondary)
                                .frame(width: 24)
                            Text(list.name).lineLimit(1)
                            if list.archived {
                                Text("Archived")
                                    .font(.caption)
                                    .foregroundStyle(.secondary)
                                    .padding(.horizontal, 6)
                                    .padding(.vertical, 2)
                                    .background(.quaternary, in: Capsule())
                            }
                            if let workspace = list.workspace {
                                workspaceTag(workspace)
                            }
                        }
                    }
                    if lists.isEmpty {
                        Text("No lists match").foregroundStyle(.secondary)
                    }
                }
            }
            .searchable(text: $query, prompt: Text("Search lists"))
            .autocorrectionDisabled()
            .textInputAutocapitalization(.never)
            .navigationTitle("Filter")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .topBarLeading) {
                    Button("Clear") { model.clearDoneFilter() }
                        .disabled(filter.isEmpty)
                }
                ToolbarItem(placement: .confirmationAction) {
                    Button("Done", systemImage: "checkmark", role: .confirm) { dismiss() }
                }
            }
        }
        .presentationDetents([.medium, .large])
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

    /// One checkable row: its label, then a tick while checked.
    private func row<Label: View>(
        checked: Bool, toggle: @escaping () -> Void, @ViewBuilder label: () -> Label
    ) -> some View {
        Button(action: toggle) {
            HStack(spacing: 8) {
                HStack(alignment: .firstTextBaseline, spacing: 12) { label() }
                Spacer(minLength: 0)
                if checked {
                    Image(systemName: "checkmark")
                        .font(.body.weight(.semibold))
                        .foregroundStyle(.tint)
                }
            }
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .accessibilityAddTraits(checked ? .isSelected : [])
    }

    private var sharedGlyph: some View {
        Image("Users")
            .resizable()
            .scaledToFit()
            .frame(width: sharedIconSide, height: sharedIconSide)
            .foregroundStyle(.secondary)
            .accessibilityLabel("Shared")
    }

    /// A shared workspace, after its list's name, as the list editor's
    /// rows have it: the first thing to truncate when short of room.
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
