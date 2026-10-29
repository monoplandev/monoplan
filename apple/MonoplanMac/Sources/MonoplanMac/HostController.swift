// The app as the profile's host (spec/daemon-plan.md, amended
// 2026-10-27). One process: this controller runs the Rust host loop
// (`Host` from MonoplanCore, the same loop as `monoplan daemon`) on a
// background thread, and the tray's `DaemonClient` talks to it in
// process through `FFITransport` (spec/mac-window-plan.md, phase 1).
// The socket the host serves is for the CLI; the app never opens it,
// and it only ever talks to its own host.
//
// The app hosts whenever it runs:
// - Whenever the client has no host and the profile is signed in,
//   start. `Host.start` takes a headless `monoplan daemon` over (asks
//   it to exit, then claims the profile) and blocks while the doc
//   boots, so it runs off the main thread and the menu says "starting"
//   meanwhile.
// - A client's `Shutdown` ends our host like any other: `logout`,
//   `login`, `password` from the terminal stop it, hold the profile
//   while they rewrite it, and we re-host once it is free again. The
//   client hears the loop end through its watch and asks here again.
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
    /// A sign-in or sign-out in progress: what the menu says, and a
    /// reason not to start a host when the socket drops meanwhile.
    private var accountWork: String?

    init(location: ProfileLocation, client: DaemonClient) {
        self.location = location
        self.client = client
        client.hostProvider = { [weak self] in
            self?.provision() ?? .pending(String(localized: "not signed in"))
        }
    }

    /// Called by the client whenever it has no host: a transport to the
    /// running one, else what the menu should say while there is none.
    private func provision() -> HostProvision {
        if let host, host.isRunning() { return .transport(FFITransport(host: host)) }
        host = nil
        if let accountWork { return .pending(accountWork) }
        if starting { return .pending(String(localized: "starting")) }
        if let failure {
            self.failure = nil
            return .pending(failure)
        }
        guard location.isSignedIn else { return .pending(String(localized: "not signed in")) }
        starting = true
        let dir = location.dir.path
        DispatchQueue.global(qos: .userInitiated).async { [weak self] in
            let result = Result { try MonoplanCore.Host.start(profileDir: dir) }
            DispatchQueue.main.async { self?.didStart(result) }
        }
        return .pending(String(localized: "starting"))
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
            failure = String(localized: "profile in use by another process")
        case .failure(let e):
            failure = String(localized: "host failed: \(e.localizedDescription)")
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
    /// `accountWork` keeps the no-host path from starting a fresh host
    /// between the stop and the wipe.
    func signOut(completion: @escaping (Error?) -> Void) {
        accountWork = String(localized: "signing out")
        let dir = location.dir.path
        DispatchQueue.global(qos: .userInitiated).async { [weak self] in
            self?.host?.shutdown()
            let result = Result { try MonoplanCore.logout(profileDir: dir) }
            DispatchQueue.main.async {
                guard let self else { return }
                self.host = nil
                self.accountWork = nil
                self.client.poke()
                completion(result.failure)
            }
        }
    }

    /// Sign in: what `monoplan auth login` does (`MonoplanCore.login`), off
    /// the main thread for the key derivation. The Rust side claims the
    /// profile only once the server has accepted the credentials, so a
    /// running host for another account is stopped then and not before;
    /// on return the profile is free and the client's poke hosts it.
    func signIn(_ creds: SignInCredentials, completion: @escaping (Error?) -> Void) {
        accountWork = String(localized: "signing in")
        let dir = location.dir.path
        DispatchQueue.global(qos: .userInitiated).async { [weak self] in
            let result = Result {
                try MonoplanCore.login(
                    profileDir: dir, server: creds.server, email: creds.email,
                    password: creds.password, deviceName: creds.deviceName)
            }
            DispatchQueue.main.async {
                guard let self else { return }
                if result.failure == nil { self.host = nil }
                self.accountWork = nil
                self.client.poke()
                completion(result.failure)
            }
        }
    }
}

struct SignInCredentials {
    var server: String
    var email: String
    var password: String
    var deviceName: String
}

private extension Result {
    var failure: Failure? {
        if case .failure(let e) = self { return e }
        return nil
    }
}
