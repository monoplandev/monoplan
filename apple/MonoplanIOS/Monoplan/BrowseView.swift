// The Browse tab: the index. A section per open workspace (Personal
// first, as `Engine.session` orders them), each with its Inbox and then
// its active lists in the doc's order, each led by its emoji (a folder
// when it has none); a list opens its Open items.
// Underneath, the pages that are not worth a tab: Done and Debug.
// The tab keeps its place across tab switches; tapping it again pops to
// the index (the system's behaviour). Item rows tick and open as in the
// lenses. Archived lists and the reserved `events` list (calendar-only,
// spec/events-plan.md) are left out.

import MonoplanCore
import MonoplanMacKit
import SwiftUI

/// One list in one workspace: what a Browse row opens.
struct ListRef: Hashable {
    let docId: String
    let listId: String
    let name: String
    /// The list's emoji, when it has one.
    var icon: String? = nil

    var key: String { "\(docId)/\(listId)" }
}

/// A list's icon: its emoji when the user picked one, else the built-in
/// glyph (a tray for the Inbox, a folder for any other list).
struct ListIcon: View {
    let listId: String
    let emoji: String?

    var body: some View {
        if let emoji, !emoji.isEmpty {
            Text(emoji)
        } else {
            Image(systemName: listId == "inbox" ? "tray" : "folder")
        }
    }
}

struct BrowseView: View {
    @EnvironmentObject private var model: AppModel

    private enum Page: Hashable {
        case done
        case debug
    }

    var body: some View {
        NavigationStack {
            List {
                ForEach(model.session?.docs ?? [], id: \.docId) { doc in
                    Section(doc.name ?? "Personal") {
                        row(ListRef(docId: doc.docId, listId: "inbox", name: "Inbox"))
                        ForEach(Lists.active(model.listsByDoc[doc.docId] ?? [])) { list in
                            row(ListRef(docId: doc.docId, listId: list.id, name: list.name, icon: list.icon))
                        }
                    }
                }
                if model.session == nil, case .connecting(let why) = model.clientState {
                    Section { Text(why).foregroundStyle(.secondary) }
                }
                Section {
                    NavigationLink(value: Page.done) {
                        Label("Done", systemImage: "checkmark.circle")
                    }
                    NavigationLink(value: Page.debug) {
                        Label("Debug", systemImage: "ladybug")
                    }
                }
            }
            .listStyle(.insetGrouped)
            .navigationTitle("Browse")
            .navigationDestination(for: ListRef.self) { ListItemsView(list: $0) }
            .navigationDestination(for: Page.self) { page in
                switch page {
                case .done: DoneView()
                case .debug: DebugView()
                }
            }
            .refreshable { model.loadSession() }
            .onAppear { model.loadSession() }
        }
        .opensItems()
    }

    private func row(_ ref: ListRef) -> some View {
        NavigationLink(value: ref) {
            Label {
                Text(ref.name)
            } icon: {
                ListIcon(listId: ref.listId, emoji: ref.icon)
            }
        }
    }
}

/// One list's Open items, in list order: the Inbox tab's page for any
/// list in any workspace. Refetched on every doc change while shown.
struct ListItemsView: View {
    @EnvironmentObject private var model: AppModel
    let list: ListRef

    var body: some View {
        Group {
            if let items = model.listItems[list.key], !items.isEmpty {
                List(items) { item in
                    ItemLink(ItemRef(docId: list.docId, itemId: item.id, title: item.text)) {
                        ItemRowView(item: item)
                    }
                }
                .listStyle(.plain)
                .animation(.default, value: items)
            } else {
                emptyState
            }
        }
        .navigationTitle(title)
        .navigationBarTitleDisplayMode(.inline)
        .refreshable { model.refreshList(list) }
        .onAppear { model.refreshList(list) }
        .onChange(of: model.changeCount) { model.refreshList(list) }
    }

    /// The header: the list's emoji, when it has one, before its name.
    private var title: String {
        guard let icon = list.icon, !icon.isEmpty else { return list.name }
        return "\(icon) \(list.name)"
    }

    private var emptyState: some View {
        ScrollView {
            if let error = model.listErrors[list.key] {
                ContentUnavailableView(
                    "\(list.name) unavailable",
                    systemImage: "exclamationmark.triangle",
                    description: Text(error)
                )
            } else if model.listItems[list.key] == nil {
                ProgressView().padding(.top, 80)
            } else {
                ContentUnavailableView(
                    "Nothing open",
                    systemImage: "folder",
                    description: Text("Open items in \(list.name) show here.")
                )
            }
        }
    }
}
