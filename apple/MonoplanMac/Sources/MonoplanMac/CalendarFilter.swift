// The Calendar's filter, the web Upcoming view's filter menu
// (`Workspace.tsx`, `DoneFilterMenu.tsx`) as a popover under the
// toolbar's filter button, there while the Calendar is shown
// (`MainSplitViewController`), laid out as Done's (`DoneFilter.swift`).
//
// "Hide Empty Days" leads it: a view option of this Mac
// (`agendaHideEmptyDaysKey`), not counted and not cleared. Then the
// workspaces, once there is more than one: none checked is every
// workspace. Behind the checks is Home's synced hidden set
// (`calendar_hidden_docs`, spec/sharing-plan.md), the web's too, so a
// workspace left out here is left out on every device. The state is
// in `MainWindowModel` (`calendarCheckedDocs`); this is the surface.

import SwiftUI

struct CalendarFilterForm: View {
    @ObservedObject var model: MainWindowModel
    @AppStorage(agendaHideEmptyDaysKey) private var hideEmptyDays = false

    var body: some View {
        let workspaces = model.doneWorkspaceOptions
        let checked = model.calendarCheckedDocs
        VStack(alignment: .leading, spacing: 0) {
            VStack(alignment: .leading, spacing: 6) {
                Toggle("Hide Empty Days", isOn: $hideEmptyDays)
                    .toggleStyle(.checkbox)
                if workspaces.count > 1 {
                    Text("Workspaces")
                        .font(.footnote).fontWeight(.semibold).foregroundStyle(.secondary)
                        .padding(.top, 6)
                    ForEach(workspaces) { workspace in
                        Toggle(isOn: Binding(
                            get: { checked.contains(workspace.docId) },
                            set: { _ in model.toggleCalendarWorkspace(workspace.docId) }
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
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            .padding(12)
            if workspaces.count > 1 {
                Divider()
                HStack {
                    Spacer()
                    Button("Clear") { model.clearCalendarFilter() }
                        .disabled(checked.isEmpty)
                }
                .padding(10)
            }
        }
        .frame(width: 240)
    }
}
