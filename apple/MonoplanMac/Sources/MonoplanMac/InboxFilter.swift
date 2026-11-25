// The Inbox lens's filter: one Inbox over every open workspace
// (spec/inbox-plan.md "One Inbox", `Engine.inbox`), narrowed to some of
// them or not, as a popover under the toolbar's filter button, there
// while the Inbox is shown and more than one workspace is open
// (`MainSplitViewController`), laid out as the Calendar's
// (`CalendarFilter.swift`). "All Workspaces" leads, checked by default;
// then a check per workspace (Personal for Home), any number at once.
// Checking "All Workspaces", or Clear, drops the picks; checking every
// workspace is "All Workspaces".
//
// The engine applies the filter (`Engine.inbox(filter:)`), so a change
// reads the rows again. The filter also says where a new item typed in
// the Inbox goes (`MainWindowModel.inboxCaptureTarget`).
//
// Device-local, like the Done filter and the Inbox order: kept in
// `UserDefaults` as the picked doc ids, none for all of them. A doc id
// no longer open is ignored (the engine's rule; `inboxPicked` for the
// checks) and kept, so a workspace that comes back is filtered again.
// The state is in `MainWindowModel`; this is the surface.

import SwiftUI

/// The filter's place in `UserDefaults`.
enum InboxFilterStore {
    private static let key = "MonoplanInboxFilter"

    static func load(from defaults: UserDefaults = .standard) -> [String] {
        defaults.stringArray(forKey: key) ?? []
    }

    static func save(_ docIds: [String], to defaults: UserDefaults = .standard) {
        if docIds.isEmpty {
            defaults.removeObject(forKey: key)
        } else {
            defaults.set(docIds, forKey: key)
        }
    }
}

struct InboxFilterForm: View {
    @ObservedObject var model: MainWindowModel

    var body: some View {
        let workspaces = model.doneWorkspaceOptions
        let picked = model.inboxPicked
        VStack(alignment: .leading, spacing: 0) {
            VStack(alignment: .leading, spacing: 6) {
                Text("Workspaces")
                    .font(.footnote).fontWeight(.semibold).foregroundStyle(.secondary)
                Toggle(isOn: Binding(
                    get: { picked.isEmpty },
                    set: { _ in model.clearInboxFilter() }
                )) {
                    Text("All Workspaces")
                }
                .toggleStyle(.checkbox)
                Divider().padding(.vertical, 2)
                ForEach(workspaces) { workspace in
                    Toggle(isOn: Binding(
                        get: { picked.contains(workspace.docId) },
                        set: { _ in model.toggleInboxWorkspace(workspace.docId) }
                    )) {
                        HStack(alignment: .firstTextBaseline, spacing: 6) {
                            Text(verbatim: workspace.name).lineLimit(1)
                            if workspace.shared {
                                PanelGlyph(image: NavArt.shared, size: 13).accessibilityHidden(true)
                            }
                        }
                    }
                    .toggleStyle(.checkbox)
                }
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            .padding(12)
            Divider()
            HStack {
                Spacer()
                Button("Clear") { model.clearInboxFilter() }
                    .disabled(picked.isEmpty)
            }
            .padding(10)
        }
        .frame(width: 240)
    }
}
