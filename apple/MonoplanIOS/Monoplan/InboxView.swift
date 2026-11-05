// The Inbox tab: the primary capture list (`inbox`, spec/data-model.md).
// A stub until the host exposes item projections to this surface; the
// Debug tab is where the app does its work meanwhile.

import SwiftUI

struct InboxView: View {
    var body: some View {
        NavigationStack {
            ContentUnavailableView(
                "Inbox",
                systemImage: "tray",
                description: Text("Captured items will show here.")
            )
            .navigationTitle("Inbox")
        }
    }
}
