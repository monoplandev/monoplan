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
    /// The Inbox's Open items, in list order, refetched on `Changed`.
    @Published private(set) var inbox: [ItemRow] = []
    /// Why the last Inbox fetch failed, until one succeeds.
    @Published private(set) var inboxError: String?
    /// The agenda's day groups (`agenda --days 14 --json`), refetched
    /// on `Changed` beside the Inbox.
    @Published private(set) var agenda: [AgendaDay] = []
    @Published private(set) var agendaError: String?
    private var refreshQueued = false
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
            refreshInbox()
            refreshAgenda()
        } else {
            session = nil
        }
    }

    private func eventArrived(_ event: WatchEvent) {
        if case .changed = event {
            changeCount += 1
            lastChangeAt = Date()
            scheduleRefresh()
        }
    }

    /// Coalesce a burst of `Changed` events into one round of fetches,
    /// as the Mac tray does for its agenda.
    private func scheduleRefresh() {
        if refreshQueued { return }
        refreshQueued = true
        DispatchQueue.main.asyncAfter(deadline: .now() + 0.25) { [weak self] in
            self?.refreshQueued = false
            self?.refreshInbox()
            self?.refreshAgenda()
        }
    }

    /// `item ls --list inbox --json` through the host.
    func refreshInbox() {
        fetch(.items(list: "inbox"), what: "inbox", parse: Items.parse) { [weak self] rows, error in
            if let rows { self?.inbox = rows }
            self?.inboxError = error
        }
    }

    /// `agenda --days 14 --all --json`: every workspace merged, Today
    /// first, then each non-empty day. Workspaces hidden from the
    /// calendar (the Home doc's setting) are already left out.
    func refreshAgenda() {
        fetch(.agenda(days: 14, all: true), what: "agenda", parse: Agenda.parse) { [weak self] days, error in
            if let days { self?.agenda = days }
            self?.agendaError = error
        }
    }

    /// One read through the host, decoded. No host is not an error:
    /// the rows stay until the next host's refresh replaces them, so
    /// `done` gets neither rows nor an error then.
    private func fetch<T>(
        _ request: Request, what: String, parse: @escaping (String) throws -> T,
        done: @escaping (T?, String?) -> Void
    ) {
        client.send(request) { result in
            switch result {
            case .success(.output(let stdout, _)):
                do {
                    done(try parse(stdout), nil)
                } catch {
                    done(nil, "could not read \(what): \(error.localizedDescription)")
                }
            case .success(let other):
                done(nil, "unexpected reply: \(String(describing: other))")
            case .failure(let error):
                if case ClientError.notConnected = error { return }
                if case ClientError.hostStopped = error { return }
                if case HostError.Stopped = error { return }
                done(nil, hostErrorMessage(error))
            }
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
