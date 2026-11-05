// The app as the profile's host: the iOS edition of
// `MonoplanMac/Sources/MonoplanMac/HostController.swift`. Same rules,
// fewer cases: on iOS nothing else can hold the profile (no CLI, no
// headless daemon, one process per container), so there is no takeover
// and "busy" can only be a host of ours still starting. The host serves
// no socket here (`monoplan-ffi`, `SERVE_SOCKET`); the client reaches
// it in process through `FFITransport`.
//
// - Whenever the client has no host, start one. No profile (first
//   launch, or just after a sign-out's wipe) first gets a local-only
//   workspace, so the app works without an account as the web client
//   does (spec/auth.md); a sign-in replaces it.
// - Sign-in and sign-out stop our host *here*, before the Rust side
//   claims the profile. On macOS the claim stops the host itself over
//   the socket; with no socket there is nothing for it to signal, and
//   it would wait out its deadline ("daemon did not stop within 10s").
//   The client re-hosts once the profile is free again.
//
// Sharing this with the macOS app is the obvious follow-up: it would
// be a third target beside the Kit, depending on MonoplanCore.

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
    /// A sign-in or sign-out in progress: what the client is told, and
    /// a reason not to start a host meanwhile.
    private var accountWork: String?

    init(location: ProfileLocation, client: DaemonClient) {
        self.location = location
        self.client = client
        client.hostProvider = { [weak self] in
            self?.provision() ?? .pending("not signed in")
        }
    }

    /// The host while it serves; nil between hosts.
    var runningHost: MonoplanCore.Host? {
        if let host, host.isRunning() { return host }
        return nil
    }

    /// Called by the client whenever it has no host: a transport to the
    /// running one, else what to show while there is none.
    private func provision() -> HostProvision {
        if let host, host.isRunning() { return .transport(FFITransport(host: host)) }
        host = nil
        if let accountWork { return .pending(accountWork) }
        if starting { return .pending("starting") }
        if let failure {
            self.failure = nil
            return .pending(failure)
        }
        starting = true
        let dir = location.dir.path
        DispatchQueue.global(qos: .userInitiated).async { [weak self] in
            let result = Result {
                try MonoplanCore.ensureLocalWorkspace(profileDir: dir)
                return try MonoplanCore.Host.start(profileDir: dir)
            }
            DispatchQueue.main.async { self?.didStart(result) }
        }
        return .pending("starting")
    }

    private func didStart(_ result: Result<MonoplanCore.Host, Error>) {
        starting = false
        switch result {
        case .success(let h):
            host = h
            NSLog("MonoplanIOS: hosting %@", location.dir.path)
        case .failure(HostError.NotLoggedIn):
            break
        case .failure(HostError.Busy):
            failure = "profile in use"
        case .failure(let e):
            failure = "host failed: \(hostErrorMessage(e))"
            NSLog("MonoplanIOS: host failed: %@", hostErrorMessage(e))
        }
        client.poke()
    }

    /// Clean exit: persist, fold, release the profile. Blocks.
    func stop() {
        host?.shutdown()
        host = nil
    }

    /// Sign out: stop our host and wipe the profile (`monoplan auth
    /// logout`). The client is then told the host is gone, finds no
    /// profile, and the no-host path seeds a fresh local-only workspace.
    func signOut(completion: @escaping (Error?) -> Void) {
        accountWork = "signing out"
        let dir = location.dir.path
        DispatchQueue.global(qos: .userInitiated).async { [weak self] in
            self?.host?.shutdown()
            let result = Result { try MonoplanCore.logout(profileDir: dir) }
            DispatchQueue.main.async {
                guard let self else { return }
                self.host = nil
                self.accountWork = nil
                self.client.hostGone()
                completion(result.failure.map(HostFailure.init))
            }
        }
    }

    /// Sign in (`monoplan auth login`), off the main thread for the key
    /// derivation. Our host is stopped first: the Rust side's claim
    /// cannot reach a socket-less host, so it would never see the
    /// profile freed. A wrong password therefore costs a host restart
    /// (the client's poke hosts the untouched local-only workspace
    /// again), which the macOS edition avoids; on success the profile
    /// is rewritten and the poke hosts the account's.
    func signIn(_ creds: SignInCredentials, completion: @escaping (Error?) -> Void) {
        accountWork = "signing in"
        let dir = location.dir.path
        DispatchQueue.global(qos: .userInitiated).async { [weak self] in
            self?.host?.shutdown()
            let result = Result {
                try MonoplanCore.login(
                    profileDir: dir, server: creds.server, email: creds.email,
                    password: creds.password, deviceName: creds.deviceName)
            }
            DispatchQueue.main.async {
                guard let self else { return }
                self.host = nil
                self.accountWork = nil
                self.client.hostGone()
                completion(result.failure.map(HostFailure.init))
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

/// A host error as a sentence; the generated `HostError` describes
/// itself by reflection.
func hostErrorMessage(_ error: Error) -> String {
    switch error {
    case HostError.NotLoggedIn: return "no profile to host"
    case HostError.Busy: return "profile in use"
    case HostError.Stopped: return "the host has stopped"
    case HostError.Failed(let message): return message
    default: return error.localizedDescription
    }
}

struct HostFailure: LocalizedError {
    let message: String
    init(_ error: Error) { message = hostErrorMessage(error) }
    var errorDescription: String? { message }
}

private extension Result {
    var failure: Failure? {
        if case .failure(let e) = self { return e }
        return nil
    }
}
