// The root: a tab per surface. The system tab bar is the floating
// Liquid Glass pill on iOS 26 (the selection slides between tabs); on
// iOS 17 and 18 it is the flat bar. `Tab` is the iOS 18 API; the
// `tabItem` branch keeps the 17.0 deployment target buildable and can
// go with it.

import SwiftUI

struct ContentView: View {
    enum Page: Hashable {
        case inbox
        case debug
    }

    @State private var page: Page = .inbox

    var body: some View {
        if #available(iOS 18, *) {
            TabView(selection: $page) {
                Tab("Inbox", systemImage: "tray", value: .inbox) {
                    InboxView()
                }
                Tab("Debug", systemImage: "ladybug", value: .debug) {
                    DebugView()
                }
            }
        } else {
            TabView(selection: $page) {
                InboxView()
                    .tabItem { Label("Inbox", systemImage: "tray") }
                    .tag(Page.inbox)
                DebugView()
                    .tabItem { Label("Debug", systemImage: "ladybug") }
                    .tag(Page.debug)
            }
        }
    }
}
