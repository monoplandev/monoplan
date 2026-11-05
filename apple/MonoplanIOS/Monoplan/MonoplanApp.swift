// Monoplan for iOS: the first native surface (2026-11-05). The app is
// the profile's host, as the macOS app is (spec/daemon-plan.md,
// amendment 2026-10-27): the Rust loop from MonoplanCore runs in this
// process, with no socket (nothing on iOS would dial one) and no
// webview. What this surface does: sign in, sign out, run a sync, and
// show what the host reports about its sync state. It exists to drive
// the architecture of the native clients, not to be used yet.
//
// Build and run on the simulator with `bun run ios` from the repo
// root, after `bun run build:apple` has produced the XCFramework.

import SwiftUI

@main
struct MonoplanApp: App {
    @StateObject private var model = AppModel()
    @Environment(\.scenePhase) private var scenePhase

    var body: some Scene {
        WindowGroup {
            ContentView()
                .environmentObject(model)
        }
        .onChange(of: scenePhase) { _, phase in
            // iOS suspends the process in the background and the
            // WebSocket dies with it; coming back, reconnect now rather
            // than waiting out the host's liveness ping and backoff
            // (spec/daemon-plan.md "Mobile hosts").
            if phase == .active { model.didBecomeActive() }
        }
    }
}
