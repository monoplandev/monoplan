// The app running the profile's engine: the iOS edition of
// `MonoplanMac/Sources/MonoplanMac/EngineController.swift`. Same rules,
// fewer cases: on iOS nothing else can hold the profile (no CLI, no
// headless daemon, one process per container), so there is no takeover
// and "busy" can only be an engine of ours still starting. The engine serves
// no socket here (`monoplan-ffi`, `SERVE_SOCKET`); the app reaches it in
// process, through the typed calls (`EngineCalls`) and `Engine.observe`
// (spec/swift-bindings-plan.md). There is no client object between the
// two: this says what the engine's state is and when a doc changed.
//
// - Whenever there is no engine, start one. No profile (first
//   launch, or just after a sign-out's wipe) first gets a local-only
//   workspace, so the app works without an account as the web client
//   does (spec/auth.md); a sign-in replaces it. A failed start is shown
//   and tried again, backing off to ten seconds.
// - Sign-in and sign-out stop our engine *here*, before the Rust side
//   claims the profile. On macOS the claim stops the engine itself over
//   the socket; with no socket there is nothing for it to signal, and
//   it would wait out its deadline ("daemon did not stop within 10s").
//   The engine is started again once the profile is free.
//
// Sharing this with the macOS app is the obvious follow-up: it would
// be a third target beside the Kit, depending on MonoplanCore.

import Foundation
import MonoplanCore
import MonoplanMacKit

/// What the app knows of its engine: running, with the sync state it
/// last reported, or not yet and why.
enum EngineState: Equatable {
    case connecting(String)
    case connected(SyncState)
}

final class EngineController {
    private let location: ProfileLocation
    private var engine: MonoplanCore.Engine?
    private var engineCalls: EngineCalls?
    private var watcher: Watcher?
    private var starting = false
    private var stopped = false
    /// A sign-in or sign-out in progress: what the state says, and
    /// a reason not to start an engine meanwhile.
    private var accountWork: String?
    private var backoff: TimeInterval = 1
    private var retryScheduled = false

    /// The engine's state, as it changes. Main queue.
    var onState: ((EngineState) -> Void)?
    /// A doc changed: read again what is on screen. Main queue.
    var onChanged: (() -> Void)?
    private(set) var state: EngineState = .connecting("starting") {
        didSet { if state != oldValue { onState?(state) } }
    }

    init(location: ProfileLocation) {
        self.location = location
    }

    /// Start the engine, and keep one running from here on.
    func start() {
        stopped = false
        ensure()
    }

    /// The engine while it serves; nil between engines.
    var runningEngine: MonoplanCore.Engine? {
        if let engine, engine.isRunning() { return engine }
        return nil
    }

    /// The running engine's typed calls (spec/swift-bindings-plan.md);
    /// nil between engines. One queue per engine, kept for its life, so
    /// the calls made to it stay in the order they were made.
    var calls: EngineCalls? {
        guard let engine = runningEngine else {
            engineCalls = nil
            return nil
        }
        if let engineCalls, engineCalls.engine === engine { return engineCalls }
        let calls = EngineCalls(engine)
        engineCalls = calls
        return calls
    }

    /// Start an engine unless one runs, is starting, or account work
    /// holds the profile.
    private func ensure() {
        if stopped || starting || runningEngine != nil { return }
        engine = nil
        if let accountWork {
            state = .connecting(accountWork)
            return
        }
        starting = true
        if case .connected = state { state = .connecting("starting") }
        let dir = location.dir.path
        DispatchQueue.global(qos: .userInitiated).async { [weak self] in
            let result = Result {
                try MonoplanCore.ensureLocalWorkspace(profileDir: dir)
                return try MonoplanCore.Engine.start(profileDir: dir)
            }
            DispatchQueue.main.async { self?.didStart(result) }
        }
    }

    private func didStart(_ result: Result<MonoplanCore.Engine, Error>) {
        starting = false
        switch result {
        case .success(let started):
            // Account work began while this one was starting: it owns
            // the profile now, and starts the next engine itself.
            if stopped || accountWork != nil {
                DispatchQueue.global(qos: .userInitiated).async { started.shutdown() }
                return
            }
            engine = started
            backoff = 1
            NSLog("MonoplanIOS: engine running on %@", location.dir.path)
            observe(started)
        case .failure(EngineError.NotLoggedIn):
            retry(showing: "not signed in")
        case .failure(EngineError.Busy):
            retry(showing: "profile in use")
        case .failure(let e):
            NSLog("MonoplanIOS: engine failed: %@", engineErrorMessage(e))
            retry(showing: "engine failed: \(engineErrorMessage(e))")
        }
    }

    /// Follow the engine: its sync state is ours while it runs (the
    /// first report arrives as this returns), and its end starts the
    /// next one.
    private func observe(_ engine: MonoplanCore.Engine) {
        do {
            watcher = try engine.observe(
                onChanged: { [weak self] in
                    guard let self, self.engine === engine else { return }
                    onChanged?()
                },
                onSync: { [weak self] sync in
                    guard let self, self.engine === engine else { return }
                    state = .connected(sync)
                },
                onStopped: { [weak self] in
                    guard let self, self.engine === engine else { return }
                    engineGone(reason: "engine stopped")
                })
        } catch {
            engineGone(reason: "engine stopped")
        }
    }

    /// The engine is no longer ours to use (its loop ended, or account
    /// work stopped it): forget it and start the next.
    private func engineGone(reason: String) {
        watcher?.cancel()
        watcher = nil
        engine = nil
        engineCalls = nil
        state = .connecting(accountWork ?? reason)
        ensure()
    }

    private func retry(showing why: String) {
        state = .connecting(why)
        if retryScheduled { return }
        retryScheduled = true
        let delay = backoff
        backoff = min(backoff * 2, 10)
        DispatchQueue.main.asyncAfter(deadline: .now() + delay) { [weak self] in
            self?.retryScheduled = false
            self?.ensure()
        }
    }

    /// Account work is starting: no engine until `finishAccountWork`.
    /// Returns the engine to shut down first, off the main thread.
    private func beginAccountWork(_ what: String) -> MonoplanCore.Engine? {
        accountWork = what
        let running = engine
        watcher?.cancel()
        watcher = nil
        engine = nil
        engineCalls = nil
        state = .connecting(what)
        return running
    }

    private func finishAccountWork() {
        accountWork = nil
        backoff = 1
        ensure()
    }

    /// Clean exit: persist, fold, release the profile. Blocks.
    func stop() {
        stopped = true
        watcher?.cancel()
        watcher = nil
        engine?.shutdown()
        engine = nil
        engineCalls = nil
    }

    /// Sign out: stop our engine and wipe the profile (`monoplan auth
    /// logout`). The next engine then finds no profile, and its start
    /// seeds a fresh local-only workspace.
    func signOut(completion: @escaping (Error?) -> Void) {
        let running = beginAccountWork("signing out")
        let dir = location.dir.path
        DispatchQueue.global(qos: .userInitiated).async { [weak self] in
            running?.shutdown()
            let result = Result { try MonoplanCore.logout(profileDir: dir) }
            DispatchQueue.main.async {
                guard let self else { return }
                self.finishAccountWork()
                completion(result.failure.map(EngineFailure.init))
            }
        }
    }

    /// Sign in (`monoplan auth login`), off the main thread for the key
    /// derivation. Our engine is stopped first: the Rust side's claim
    /// cannot reach a socket-less engine, so it would never see the
    /// profile freed. A wrong password therefore costs an engine restart
    /// (the next one starts on the untouched local-only workspace
    /// again), which the macOS edition avoids; on success the profile
    /// is rewritten and the next engine starts on the account's.
    func signIn(_ creds: SignInCredentials, completion: @escaping (Error?) -> Void) {
        let running = beginAccountWork("signing in")
        let dir = location.dir.path
        DispatchQueue.global(qos: .userInitiated).async { [weak self] in
            running?.shutdown()
            let result = Result {
                try MonoplanCore.login(
                    profileDir: dir, server: creds.server, email: creds.email,
                    password: creds.password, deviceName: creds.deviceName)
            }
            DispatchQueue.main.async {
                guard let self else { return }
                self.finishAccountWork()
                completion(result.failure.map(EngineFailure.init))
            }
        }
    }

    /// Create an account (`monoplan auth signup`, `MonoplanCore.signup`),
    /// off the main thread. Our engine is stopped first, as for
    /// `signIn`; its shutdown persists the local-only workspace, which
    /// signup then adopts as the account's Home doc (same id, same key;
    /// `spec/auth.md` §Device pairing), so nothing captured before is
    /// lost. The engine started afterwards pushes that
    /// history on its first connect; after a failure it starts on the
    /// untouched local-only workspace again.
    func signUp(_ creds: SignInCredentials, completion: @escaping (Result<SignupOutcome, Error>) -> Void) {
        let running = beginAccountWork("signing in")
        let dir = location.dir.path
        DispatchQueue.global(qos: .userInitiated).async { [weak self] in
            running?.shutdown()
            let result = Result {
                try MonoplanCore.signup(
                    profileDir: dir, server: creds.server, email: creds.email,
                    password: creds.password, deviceName: creds.deviceName)
            }
            DispatchQueue.main.async {
                guard let self else { return }
                self.finishAccountWork()
                completion(result.mapError(EngineFailure.init))
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

/// An engine error as a sentence; the generated `EngineError` describes
/// itself by reflection.
func engineErrorMessage(_ error: Error) -> String {
    switch error {
    case EngineError.NotLoggedIn: return "no profile to run"
    case EngineError.Busy: return "profile in use"
    case EngineError.Stopped: return "the engine has stopped"
    case EngineError.Failed(let message): return message
    default: return error.localizedDescription
    }
}

struct EngineFailure: LocalizedError {
    let message: String
    init(_ error: Error) { message = engineErrorMessage(error) }
    var errorDescription: String? { message }
}

private extension Result {
    var failure: Failure? {
        if case .failure(let e) = self { return e }
        return nil
    }
}
