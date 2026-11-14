// Search, drawn over Home's navigation stack while the bottom bar's
// field has focus or text. It works as the web's Find does
// (spec/search.md; `find --json` through the engine): the built-in
// pages whose names match, then lists, then items, ranked across every
// open workspace, every word matching and the last one as a prefix.
// Unlike the web there is no default menu: with nothing typed the page
// is blank. A page or list result closes search and pushes it; an item
// result opens the item's sheet over the results, which stay.

import MonoplanCore
import MonoplanMacKit
import SwiftUI

/// What a result that is not an item opens.
enum SearchPick {
    case page(HomePage)
    case list(ListRef)
}

struct SearchView: View {
    @EnvironmentObject private var model: AppModel
    @Environment(\.openItem) private var openItem
    let query: String
    let pick: (SearchPick) -> Void

    private var trimmed: String { query.trimmingCharacters(in: .whitespacesAndNewlines) }

    /// A built-in page: not a doc entity, so the index never has it and
    /// its label is matched here, with the index's own name rule.
    private struct BuiltIn: Identifiable {
        let title: String
        /// The page's symbol; nil is Starred, which has the web's star.
        let systemImage: String?
        let pick: SearchPick
        var id: String { title }
    }

    private var builtIns: [BuiltIn] {
        var all = [
            BuiltIn(title: "Starred", systemImage: nil, pick: .page(.starred)),
            BuiltIn(title: "Calendar", systemImage: "calendar", pick: .page(.agenda)),
            BuiltIn(title: "Map", systemImage: "map", pick: .page(.map)),
            BuiltIn(title: "Done", systemImage: "checkmark.circle", pick: .page(.done)),
            BuiltIn(title: "Bin", systemImage: "trash", pick: .page(.bin)),
        ]
        if let home = model.session?.primaryDocId {
            all.append(BuiltIn(
                title: "Inbox", systemImage: "tray",
                pick: .list(ListRef(docId: home, listId: "inbox", name: "Inbox"))
            ))
        }
        return all.filter { matchesName(name: $0.title, query: trimmed) }
    }

    var body: some View {
        ZStack {
            Color(.systemGroupedBackground).ignoresSafeArea()
            if !trimmed.isEmpty {
                results
            }
        }
    }

    @ViewBuilder
    private var results: some View {
        let pages = builtIns
        // The hits of the query before this one stay up until this
        // one's arrive, so typing does not flicker; "no results" is
        // only said once the engine has answered for what is typed.
        let hits = model.searchHits
        if pages.isEmpty && hits.isEmpty {
            if let error = model.searchError {
                ContentUnavailableView(
                    "Search unavailable",
                    systemImage: "exclamationmark.triangle",
                    description: Text(error)
                )
            } else if model.searchedQuery == trimmed {
                ContentUnavailableView.search(text: trimmed)
            }
        } else {
            List {
                Section {
                    ForEach(pages) { page in
                        Button { pick(page.pick) } label: {
                            Label {
                                Text(page.title)
                            } icon: {
                                if let symbol = page.systemImage {
                                    Image(systemName: symbol)
                                } else {
                                    StarIcon()
                                }
                            }
                                .frame(maxWidth: .infinity, alignment: .leading)
                                .contentShape(Rectangle())
                        }
                    }
                    ForEach(hits) { hit in
                        Button { open(hit) } label: { SearchHitRow(hit: hit) }
                            .modifier(ItemRowMenu(ref: menuItem(hit), state: hit.state ?? "backlog"))
                    }
                } header: {
                    // The count is of the rows shown: pages and hits.
                    let count = pages.count + hits.count
                    HStack(alignment: .firstTextBaseline) {
                        Text("Search results")
                            .font(.subheadline.weight(.semibold))
                            .foregroundStyle(.primary)
                        Spacer(minLength: 8)
                        Text(count == 1 ? "1 result" : "\(count) results")
                            .font(.subheadline.monospacedDigit())
                            .foregroundStyle(.secondary)
                    }
                    .textCase(nil)
                }
            }
            // Plain, so a row reads as a row rather than a tinted link.
            .buttonStyle(.plain)
            .listStyle(.insetGrouped)
            .scrollDismissesKeyboard(.interactively)
        }
    }

    /// The item a hit's long-press menu acts on: none for a list, and
    /// none for a binned item, which the menu's actions do not fit.
    private func menuItem(_ hit: SearchHit) -> ItemRef? {
        guard hit.kind == .item, hit.state != "binned" else { return nil }
        return ItemRef(docId: hit.docId, itemId: hit.itemId, title: hit.title)
    }

    private func open(_ hit: SearchHit) {
        switch hit.kind {
        case .item:
            openItem(ItemRef(docId: hit.docId, itemId: hit.itemId, title: hit.title))
        case .list:
            guard let docId = hit.docId ?? model.session?.primaryDocId else { return }
            pick(.list(ListRef(docId: docId, listId: hit.itemId, name: hit.title, icon: hit.icon)))
        }
    }
}

/// One result: a list under its icon, or an item under a checkbox that
/// mirrors its state (display only, as in the web's palette; the sheet
/// ticks). Underneath, where the result lives: the item's list, the
/// workspace when it is not Personal, and "Archived" for an archived
/// list or an item in one. A binned item is struck through.
private struct SearchHitRow: View {
    let hit: SearchHit

    private var binned: Bool { hit.state == "binned" }

    var body: some View {
        HStack(alignment: .firstTextBaseline, spacing: 12) {
            Group {
                switch hit.kind {
                case .item: ItemCheckbox(state: hit.state ?? "backlog")
                case .list: ListIcon(listId: hit.itemId, emoji: hit.icon)
                }
            }
            .frame(minWidth: 24)
            VStack(alignment: .leading, spacing: 2) {
                Text(hit.title)
                    .strikethrough(binned)
                    .foregroundStyle(binned ? .secondary : .primary)
                    .itemTextSlot()
                if !note.isEmpty {
                    Text(note)
                        .font(.caption)
                        .foregroundStyle(.secondary)
                        .lineLimit(1)
                }
            }
            .frame(maxWidth: .infinity, alignment: .leading)
        }
        .contentShape(Rectangle())
    }

    private var note: String {
        var parts = [hit.listName, hit.docName].compactMap { $0 }
        if binned { parts.append("Bin") }
        if hit.archived { parts.append("Archived") }
        return parts.joined(separator: " · ")
    }
}
