// The one observable the views read: the client's state (what the
// host says about sync), the host's identity, the session (account,
// server, open docs) and the in-flight account work. It owns the
// `DaemonClient` and the `HostController`, exactly as the macOS app's
// delegate does; the views never touch either.

import Combine
import Foundation
import MonoplanCore
import MonoplanMacKit

final class AppModel: ObservableObject {
    /// Connected with the host's current sync status, or why not yet.
    @Published private(set) var clientState: DaemonClient.State = .connecting("starting")
    /// What the host said in its hello: its build id and pid.
    @Published private(set) var hostInfo: DaemonClient.DaemonInfo?
    /// Identity, server and open docs, from `Host.session`; nil without
    /// a host.
    @Published private(set) var session: HostSession?
    /// The profile on disk carries a device token.
    @Published private(set) var hasAccount = false
    /// A sign-in or sign-out in progress.
    @Published private(set) var busy: String?
    /// `Changed` events since the client connected, and the last one's
    /// time: the cheapest proof that remote ops are arriving.
    @Published private(set) var changeCount = 0
    @Published private(set) var lastChangeAt: Date?
    @Published var lastError: String?
    @Published var signingIn = false
    @Published var signInError: String?

    let location: ProfileLocation
    private let client: DaemonClient
    private let hostController: HostController

    init() {
        location = ProfileLocation.resolve()
        // Application Support does not exist in a fresh iOS container;
        // the Rust side creates the profile dir itself but not its
        // parents.
        try? FileManager.default.createDirectory(at: location.dir, withIntermediateDirectories: true)
        client = DaemonClient(clientId: "monoplan-ios")
        hostController = HostController(location: location, client: client)
        hasAccount = location.hasAccount
        client.onState = { [weak self] state in self?.stateChanged(state) }
        client.onEvent = { [weak self] event in self?.eventArrived(event) }
        client.start()
    }

    /// The sync status while connected.
    var sync: SyncStatus? {
        if case .connected(let sync) = clientState { return sync }
        return nil
    }

    private func stateChanged(_ state: DaemonClient.State) {
        clientState = state
        hostInfo = client.daemon
        hasAccount = location.hasAccount
        if case .connected = state {
            loadSession()
        } else {
            session = nil
        }
    }

    private func eventArrived(_ event: WatchEvent) {
        if case .changed = event {
            changeCount += 1
            lastChangeAt = Date()
        }
    }

    /// `Host.session` blocks for the loop's turn, so off the main thread.
    func loadSession() {
        guard let host = hostController.runningHost else {
            session = nil
            return
        }
        DispatchQueue.global(qos: .userInitiated).async { [weak self] in
            let result = try? host.session()
            DispatchQueue.main.async { self?.session = result }
        }
    }

    /// `Request::Sync`: push now and report whether the server answers.
    func syncNow() { send(.sync) }

    /// `Request::Reconnect`: drop the socket and dial again now.
    func reconnect() { send(.reconnect) }

    private func send(_ request: Request) {
        client.send(request) { [weak self] result in
            if case .failure(let error) = result {
                self?.lastError = error.localizedDescription
            }
        }
    }

    func signIn(_ creds: SignInCredentials, completion: @escaping (Bool) -> Void) {
        signingIn = true
        signInError = nil
        busy = "Signing in"
        hostController.signIn(creds) { [weak self] error in
            guard let self else { return }
            signingIn = false
            busy = nil
            hasAccount = location.hasAccount
            if let error {
                signInError = error.localizedDescription
                completion(false)
            } else {
                completion(true)
            }
        }
    }

    func signOut() {
        busy = "Signing out"
        hostController.signOut { [weak self] error in
            guard let self else { return }
            busy = nil
            hasAccount = location.hasAccount
            if let error { lastError = error.localizedDescription }
        }
    }

    /// Back in the foreground: the socket is dead after a suspension
    /// of any length. A local-only workspace has nothing to dial.
    func didBecomeActive() {
        guard let sync, !sync.localOnly else { return }
        reconnect()
    }

    /// Clean exit is not a thing on iOS (the process is killed, not
    /// quit); the host persists on its own tick and the WAL covers the
    /// rest. Kept for completeness and tests.
    func stopHost() { hostController.stop() }
}
