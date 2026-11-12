// The Browse tab: the index. A section per open workspace (Personal
// first, as `Engine.session` orders them), each with its Inbox and then
// its active lists in the doc's order; a list opens its Open items.
// Underneath, the pages that are not worth a tab: Done and Debug.
// Read-only, like the lenses. Archived lists and the reserved `events`
// list (calendar-only, spec/events-plan.md) are left out.

import MonoplanCore
import MonoplanMacKit
import SwiftUI

/// One list in one workspace: what a Browse row opens.
struct ListRef: Hashable {
    let docId: String
    let listId: String
    let name: String

    var key: String { "\(docId)/\(listId)" }
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
                        row(ListRef(docId: doc.docId, listId: "inbox", name: "Inbox"), systemImage: "tray")
                        ForEach(Lists.active(model.listsByDoc[doc.docId] ?? [])) { list in
                            row(ListRef(docId: doc.docId, listId: list.id, name: list.name), systemImage: "list.bullet")
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
    }

    private func row(_ ref: ListRef, systemImage: String) -> some View {
        NavigationLink(value: ref) {
            Label(ref.name, systemImage: systemImage)
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
                    ItemRowView(item: item)
                }
                .listStyle(.plain)
            } else {
                emptyState
            }
        }
        .navigationTitle(list.name)
        .navigationBarTitleDisplayMode(.inline)
        .refreshable { model.refreshList(list) }
        .onAppear { model.refreshList(list) }
        .onChange(of: model.changeCount) { model.refreshList(list) }
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
                    systemImage: "list.bullet",
                    description: Text("Open items in \(list.name) show here.")
                )
            }
        }
    }
}
