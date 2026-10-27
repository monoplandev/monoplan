// The app as the profile's host (spec/daemon-plan.md, amended
// 2026-10-27). On macOS the normal path is one process: this controller
// runs the Rust host loop (`Host` from MonoplanCore, the same loop as
// `monoplan daemon`) on a background thread, and the tray's
// `DaemonClient` talks to it over the socket like any other client.
//
// The app and the headless daemon are alternatives, not partners:
// - Host when the lock is free and the profile is signed in.
//   `Host.start` blocks while the doc boots, so it runs off the main
//   thread and the menu says "starting" meanwhile.
// - If something else holds the lock (a `monoplan daemon`, another
//   copy of the app), stay a client of it: the menu still works, and
//   Settings says who is hosting. Never take over. Host once it goes.
// - Our host ends only when the app quits (`stop`). It refuses a
//   client's `Shutdown`, so `daemon stop`, `logout`, `login` and
//   `password` from the terminal tell the user to quit the app first.

import Foundation
import MonoplanCore
import MonoplanMacKit

final class HostController {
    private let location: ProfileLocation
    private let client: DaemonClient
    private var host: MonoplanCore.Host?
    private var starting = false
    /// The last failed start, shown once and then retried.
    private var failure: String?

    init(location: ProfileLocation, client: DaemonClient) {
        self.location = location
        self.client = client
        client.hostProvider = { [weak self] in self?.ensureHosting() }
    }

    /// Whether the connected host is this process.
    var isHosting: Bool { host?.isRunning() == true }

    /// Called by the client whenever its socket is down. Returns what
    /// the menu should say about it, or nil for the socket's own error.
    private func ensureHosting() -> String? {
        if let host, host.isRunning() { return nil }
        host = nil
        if starting { return "starting" }
        if let failure {
            self.failure = nil
            return failure
        }
        guard location.isSignedIn else { return "not signed in" }
        // Someone else holds the profile: the client connects to them.
        if location.daemonIsRunning() { return nil }
        starting = true
        let dir = location.dir.path
        DispatchQueue.global(qos: .userInitiated).async { [weak self] in
            let result = Result { try MonoplanCore.Host.start(profileDir: dir) }
            DispatchQueue.main.async { self?.didStart(result) }
        }
        return "starting"
    }

    private func didStart(_ result: Result<MonoplanCore.Host, Error>) {
        starting = false
        switch result {
        case .success(let h):
            host = h
            NSLog("MonoplanMac: hosting %@", location.dir.path)
        case .failure(HostError.AlreadyRunning), .failure(HostError.NotLoggedIn):
            break
        case .failure(let e):
            failure = "host failed: \(e.localizedDescription)"
            NSLog("MonoplanMac: host failed: %@", e.localizedDescription)
        }
        client.poke()
    }

    /// Clean exit on quit: persist, fold, release the profile. Blocks.
    func stop() {
        host?.shutdown()
        host = nil
    }
}
