// The Inbox lens's filter: one Inbox over every open workspace
// (spec/inbox-plan.md "One Inbox", `Engine.inbox`), narrowed to one
// workspace's or not, as a popover under the toolbar's filter button,
// there while the Inbox is shown and more than one workspace is open
// (`MainSplitViewController`), laid out as the Calendar's
// (`CalendarFilter.swift`). One choice, not checks: All Workspaces, or
// one workspace (Personal for Home).
//
// The filter also says where a new item typed in the Inbox goes: Home's
// Inbox unfiltered, the workspace's when filtered
// (`MainWindowModel.inboxCaptureTarget`).
//
// Device-local, like the Done filter and the Inbox order: kept in
// `UserDefaults` as the workspace's doc id, absent for all of them. A
// doc id no longer open reads as no filter (`MainWindowModel.inboxFilter`)
// and is kept, so a workspace that comes back is filtered again. The
// state is in `MainWindowModel`; this is the surface.

import SwiftUI

/// The filter's place in `UserDefaults`.
enum InboxFilterStore {
    private static let key = "MonoplanInboxFilter"

    static func load(from defaults: UserDefaults = .standard) -> String? {
        defaults.string(forKey: key)
    }

    static func save(_ docId: String?, to defaults: UserDefaults = .standard) {
        if let docId {
            defaults.set(docId, forKey: key)
        } else {
            defaults.removeObject(forKey: key)
        }
    }
}

struct InboxFilterForm: View {
    @ObservedObject var model: MainWindowModel

    var body: some View {
        let workspaces = model.doneWorkspaceOptions
        VStack(alignment: .leading, spacing: 6) {
            Text("Workspaces")
                .font(.footnote).fontWeight(.semibold).foregroundStyle(.secondary)
            // "" is All: a tag cannot be nil.
            Picker(selection: Binding(
                get: { model.inboxFilter ?? "" },
                set: { model.setInboxFilter($0.isEmpty ? nil : $0) }
            )) {
                Text("All Workspaces").tag("")
                ForEach(workspaces) { workspace in
                    Text(verbatim: workspace.name).lineLimit(1).tag(workspace.docId)
                }
            } label: {
                Text("Workspaces")
            }
            .pickerStyle(.radioGroup)
            .labelsHidden()
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .padding(12)
        .frame(width: 240)
    }
}
