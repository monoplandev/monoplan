// The root: a tab per lens, and Browse for everything else (every
// workspace's lists, Done, Debug). The system tab bar is the floating
// Liquid Glass pill on iOS 26 (the selection slides between tabs); on
// iOS 17 and 18 it is the flat bar. `Tab` is the iOS 18 API; the
// `tabItem` branch keeps the 17.0 deployment target buildable and can
// go with it.

import SwiftUI

struct ContentView: View {
    enum Page: Hashable {
        case inbox
        case starred
        case agenda
        case browse
    }

    @State private var page: Page = .inbox

    var body: some View {
        if #available(iOS 18, *) {
            TabView(selection: $page) {
                Tab("Inbox", systemImage: "tray", value: .inbox) {
                    InboxView()
                }
                Tab("Starred", systemImage: "star", value: .starred) {
                    StarredView()
                }
                Tab("Agenda", systemImage: "calendar", value: .agenda) {
                    AgendaView()
                }
                Tab("Browse", systemImage: "square.grid.2x2", value: .browse) {
                    BrowseView()
                }
            }
        } else {
            TabView(selection: $page) {
                InboxView()
                    .tabItem { Label("Inbox", systemImage: "tray") }
                    .tag(Page.inbox)
                StarredView()
                    .tabItem { Label("Starred", systemImage: "star") }
                    .tag(Page.starred)
                AgendaView()
                    .tabItem { Label("Agenda", systemImage: "calendar") }
                    .tag(Page.agenda)
                BrowseView()
                    .tabItem { Label("Browse", systemImage: "square.grid.2x2") }
                    .tag(Page.browse)
            }
        }
    }
}
