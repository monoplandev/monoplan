// The app running the profile's engine (spec/daemon-plan.md, amended
// 2026-10-27). One process: this controller runs the Rust engine loop
// (`Engine` from MonoplanCore, the same loop as `monoplan daemon`) on a
// background thread, and the app reaches it in process through the
// typed calls (`EngineCalls`) and `Engine.observe`
// (spec/swift-bindings-plan.md, phase 4). There is no client object or
// wire format between the two: this says what the engine's state is and
// when a doc changed. The socket the engine serves is for the CLI; the
// app never opens it, and it only ever talks to its own engine.
//
// The engine runs whenever the app does:
// - Whenever there is no engine, start one. No profile (a first run,
//   or just after a sign-out's wipe) first gets a local-only workspace
//   (`ensureLocalWorkspace`, what the CLI's first doc command does), so
//   the app never needs an account to be used, as the web client does
//   not (spec/auth.md). `Engine.start` takes a headless `monoplan daemon`
//   over (asks it to exit, then claims the profile) and blocks while
//   the doc boots, so it runs off the main thread and the menu says
//   "starting" meanwhile.
// - A client's `Shutdown` ends our engine like any other: `logout`,
//   `login`, `password` from the terminal stop it, hold the profile
//   while they rewrite it, and we start the engine again once it is
//   free. The observer hears the loop end and the next start follows.
// - "Busy" is the one thing we wait out rather than take over: another
//   copy of the app, a command holding the profile, or an engine still
//   starting. The menu says so and the start is retried, backing off to
//   ten seconds.
//
// The iOS app's `EngineController` is the same thing with fewer cases
// (no socket, so it stops its engine before a sign-in rather than
// letting the Rust side's claim do it). Sharing one is the follow-up.

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
    /// A sign-in or sign-out in progress: what the state says once the
    /// engine has gone, and a reason not to start one meanwhile.
    private var accountWork: String?
    private var backoff: TimeInterval = 1
    private var retryScheduled = false

    /// The engine's state, as it changes. Main queue.
    var onState: ((EngineState) -> Void)?
    /// A doc changed: read again what is on screen. Main queue.
    var onChanged: (() -> Void)?
    private(set) var state: EngineState = .connecting(String(localized: "starting")) {
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

    /// The running engine's typed calls; nil between engines. One queue
    /// per engine, kept for its life, so the calls made to it stay in
    /// the order they were made.
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
        if case .connected = state { state = .connecting(String(localized: "starting")) }
        let dir = location.dir.path
        DispatchQueue.global(qos: .userInitiated).async { [weak self] in
            let result = Result {
                // A no-op with a profile; a seeded local-only workspace
                // without one.
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
            // Quit, or account work that began while this one was
            // starting: the profile is not ours to run right now, and
            // the account work starts the next engine itself.
            if stopped || accountWork != nil {
                DispatchQueue.global(qos: .userInitiated).async { started.shutdown() }
                return
            }
            engine = started
            backoff = 1
            NSLog("MonoplanMac: engine running on %@", location.dir.path)
            observe(started)
        case .failure(EngineError.NotLoggedIn):
            retry(showing: String(localized: "not signed in"))
        case .failure(EngineError.Busy):
            retry(showing: String(localized: "profile in use by another process"))
        case .failure(let e):
            NSLog("MonoplanMac: engine failed: %@", engineErrorMessage(e))
            retry(showing: String(localized: "engine failed: \(engineErrorMessage(e))"))
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
                    self.onChanged?()
                },
                onSync: { [weak self] sync in
                    guard let self, self.engine === engine else { return }
                    self.state = .connected(sync)
                },
                onStopped: { [weak self] in
                    guard let self, self.engine === engine else { return }
                    self.engineGone(reason: String(localized: "the engine has stopped"))
                })
        } catch {
            engineGone(reason: String(localized: "the engine has stopped"))
        }
    }

    /// The engine is no longer ours to use (its loop ended: a terminal
    /// command's or a sign-in's claim): forget it and start the next.
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

    /// Account work is over. If it ended our engine (a sign-out, or a
    /// sign-in the server accepted), start the next one on whatever the
    /// profile now is; if it never got that far (a wrong password), the
    /// engine is still running and nothing changes.
    private func finishAccountWork() {
        accountWork = nil
        backoff = 1
        if runningEngine == nil { engineGone(reason: String(localized: "starting")) }
    }

    /// Clean exit on quit: persist, fold, release the profile. Blocks.
    func stop() {
        stopped = true
        watcher?.cancel()
        watcher = nil
        engine?.shutdown()
        engine = nil
        engineCalls = nil
    }

    /// Sign out: stop our engine and wipe the profile (what `monoplan
    /// auth logout` does). The next engine then finds no profile, and
    /// its start seeds a fresh local-only workspace, as the web client
    /// drops to a fresh anonymous session. Off the main thread, since
    /// the engine's clean exit persists and folds; `accountWork` keeps
    /// a fresh engine from starting between the stop and the wipe.
    func signOut(completion: @escaping (Error?) -> Void) {
        accountWork = String(localized: "signing out")
        let running = engine
        watcher?.cancel()
        watcher = nil
        engine = nil
        engineCalls = nil
        state = .connecting(String(localized: "signing out"))
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

    /// Sign in: what `monoplan auth login` does (`MonoplanCore.login`), off
    /// the main thread for the key derivation. The Rust side claims the
    /// profile only once the server has accepted the credentials, so a
    /// running engine (the local-only workspace's, or another account's)
    /// is stopped then and not before; the observer hears it end, and on
    /// return the profile is free and the next engine starts on it. A
    /// local-only workspace is replaced (the sheet says so first).
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
                self.finishAccountWork()
                completion(result.failure.map(EngineFailure.init))
            }
        }
    }

    /// Create an account: what `monoplan auth signup` does
    /// (`MonoplanCore.signup`), off the main thread. The local-only
    /// workspace the engine has been running is adopted as the account's
    /// Home doc (same id, same key; `spec/auth.md` §Device pairing), so
    /// nothing captured before is lost; the engine that comes up after
    /// pushes its whole history on the first connect.
    func signUp(_ creds: SignInCredentials, completion: @escaping (Result<SignupOutcome, Error>) -> Void) {
        accountWork = String(localized: "signing in")
        let dir = location.dir.path
        DispatchQueue.global(qos: .userInitiated).async { [weak self] in
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

/// An engine error as a sentence. The generated `EngineError` describes
/// itself by reflection ("MonoplanCore.EngineError.Failed(message: …)"),
/// which is not what a sheet, a menu or the window should show.
func engineErrorMessage(_ error: Error) -> String {
    switch error {
    case EngineError.NotLoggedIn: return String(localized: "no profile to run")
    case EngineError.Busy: return String(localized: "profile in use by another process")
    case EngineError.Stopped: return String(localized: "the engine has stopped")
    case EngineError.Failed(let message): return message
    default: return error.localizedDescription
    }
}

/// `engineErrorMessage` as an `Error`, for completions whose callers
/// read `localizedDescription`.
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
