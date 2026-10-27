// The app as the profile's host (spec/daemon-plan.md, amended
// 2026-10-27). One process: this controller runs the Rust host loop
// (`Host` from MonoplanCore, the same loop as `monoplan daemon`) on a
// background thread, and the tray's `DaemonClient` talks to it over the
// socket like any other client. The app only ever talks to its own
// host; it is never a client of some other process.
//
// The app hosts whenever it runs:
// - Whenever the socket is down and the profile is signed in, start.
//   `Host.start` takes a headless `monoplan daemon` over (asks it to
//   exit, then claims the profile) and blocks while the doc boots, so
//   it runs off the main thread and the menu says "starting" meanwhile.
// - A client's `Shutdown` ends our host like any other: `logout`,
//   `login`, `password` from the terminal stop it, hold the profile
//   while they rewrite it, and we re-host once it is free again.
// - "Busy" is the one thing we wait out rather than take over: another
//   copy of the app, a command holding the profile, or a host still
//   starting. The menu says so and the client's backoff retries.

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
    /// Sign-out in progress: do not re-host when the socket drops.
    private var signingOut = false

    init(location: ProfileLocation, client: DaemonClient) {
        self.location = location
        self.client = client
        client.hostProvider = { [weak self] in self?.ensureHosting() }
    }

    /// Called by the client whenever its socket is down. Returns what
    /// the menu should say about it, or nil for the socket's own error.
    private func ensureHosting() -> String? {
        if let host, host.isRunning() { return nil }
        host = nil
        if signingOut { return "signing out" }
        if starting { return "starting" }
        if let failure {
            self.failure = nil
            return failure
        }
        guard location.isSignedIn else { return "not signed in" }
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
        case .failure(HostError.NotLoggedIn):
            break
        case .failure(HostError.Busy):
            failure = "profile in use by another process"
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

    /// Sign out: stop our host, wipe the profile (what `monoplan
    /// logout` does), and let the client see "not signed in". Off the
    /// main thread, since the host's clean exit persists and folds.
    /// `signingOut` keeps the socket-down path from starting a fresh
    /// host between the stop and the wipe.
    func signOut(completion: @escaping (Error?) -> Void) {
        signingOut = true
        let dir = location.dir.path
        DispatchQueue.global(qos: .userInitiated).async { [weak self] in
            self?.host?.shutdown()
            let result = Result { try MonoplanCore.logout(profileDir: dir) }
            DispatchQueue.main.async {
                guard let self else { return }
                self.host = nil
                self.signingOut = false
                self.client.poke()
                completion(result.failure)
            }
        }
    }
}

private extension Result {
    var failure: Failure? {
        if case .failure(let e) = self { return e }
        return nil
    }
}
