// The app running the profile's engine (spec/daemon-plan.md, amended
// 2026-10-27). One process: this controller runs the Rust engine loop
// (`Engine` from MonoplanCore, the same loop as `monoplan daemon`) on a
// background thread, and the tray's `DaemonClient` talks to it in
// process through `FFITransport` (spec/mac-window-plan.md, phase 1).
// The socket the engine serves is for the CLI; the app never opens it,
// and it only ever talks to its own engine.
//
// The engine runs whenever the app does:
// - Whenever the client has no engine, start. No profile (a first run,
//   or just after a sign-out's wipe) first gets a local-only workspace
//   (`ensureLocalWorkspace`, what the CLI's first doc command does), so
//   the app never needs an account to be used, as the web client does
//   not (spec/auth.md). `Engine.start` takes a headless `monoplan daemon`
//   over (asks it to exit, then claims the profile) and blocks while
//   the doc boots, so it runs off the main thread and the menu says
//   "starting" meanwhile.
// - A client's `Shutdown` ends our engine like any other: `logout`,
//   `login`, `password` from the terminal stop it, hold the profile
//   while they rewrite it, and we start the engine again once it is free. The
//   client hears the loop end through its watch and asks here again.
// - "Busy" is the one thing we wait out rather than take over: another
//   copy of the app, a command holding the profile, or an engine still
//   starting. The menu says so and the client's backoff retries.

import Foundation
import MonoplanCore
import MonoplanMacKit

final class EngineController {
    private let location: ProfileLocation
    private let client: DaemonClient
    private var engine: MonoplanCore.Engine?
    private var starting = false
    /// The engine is being started (`Engine.start` off the main thread):
    /// the tray holds its launch-time window open until this settles.
    var isStarting: Bool { starting }
    /// The last failed start, shown once and then retried.
    private var failure: String?
    /// A sign-in or sign-out in progress: what the menu says, and a
    /// reason not to start an engine when the socket drops meanwhile.
    private var accountWork: String?
    /// Whether a sign-in or sign-out is in progress: the window's page
    /// shows it as waiting rather than signed out, whatever the profile
    /// on disk says mid-way.
    var isBusy: Bool { accountWork != nil }

    init(location: ProfileLocation, client: DaemonClient) {
        self.location = location
        self.client = client
        client.engineProvider = { [weak self] in
            self?.provision() ?? .pending(String(localized: "not signed in"))
        }
    }

    /// The engine while it serves; what the window mirrors
    /// (spec/mac-window-plan.md). Nil between engines.
    var runningEngine: MonoplanCore.Engine? {
        if let engine, engine.isRunning() { return engine }
        return nil
    }

    /// Called by the client whenever it has no engine: a transport to the
    /// running one, else what the menu should say while there is none.
    private func provision() -> EngineProvision {
        if let engine, engine.isRunning() { return .transport(FFITransport(engine: engine)) }
        engine = nil
        if let accountWork { return .pending(accountWork) }
        if starting { return .pending(String(localized: "starting")) }
        if let failure {
            self.failure = nil
            return .pending(failure)
        }
        starting = true
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
        return .pending(String(localized: "starting"))
    }

    private func didStart(_ result: Result<MonoplanCore.Engine, Error>) {
        starting = false
        switch result {
        case .success(let h):
            engine = h
            NSLog("MonoplanMac: engine running on %@", location.dir.path)
        case .failure(EngineError.NotLoggedIn):
            break
        case .failure(EngineError.Busy):
            failure = String(localized: "profile in use by another process")
        case .failure(let e):
            failure = String(localized: "engine failed: \(engineErrorMessage(e))")
            NSLog("MonoplanMac: engine failed: %@", engineErrorMessage(e))
        }
        client.poke()
    }

    /// Clean exit on quit: persist, fold, release the profile. Blocks.
    func stop() {
        engine?.shutdown()
        engine = nil
    }

    /// Sign out: stop our engine and wipe the profile (what `monoplan
    /// logout` does). The client is then told the engine is gone
    /// (`engineGone`, whether or not its watch said so yet), finds no
    /// profile, and the no-engine path seeds a fresh local-only workspace and runs it,
    /// as the web client drops to a fresh anonymous session. Off the
    /// main thread, since the engine's clean exit persists and folds.
    /// `accountWork` keeps the no-engine path from starting a fresh engine
    /// between the stop and the wipe.
    func signOut(completion: @escaping (Error?) -> Void) {
        accountWork = String(localized: "signing out")
        let dir = location.dir.path
        DispatchQueue.global(qos: .userInitiated).async { [weak self] in
            self?.engine?.shutdown()
            let result = Result { try MonoplanCore.logout(profileDir: dir) }
            DispatchQueue.main.async {
                guard let self else { return }
                self.engine = nil
                self.accountWork = nil
                self.client.engineGone()
                completion(result.failure.map(EngineFailure.init))
            }
        }
    }

    /// Sign in: what `monoplan auth login` does (`MonoplanCore.login`), off
    /// the main thread for the key derivation. The Rust side claims the
    /// profile only once the server has accepted the credentials, so a
    /// running engine (the local-only workspace's, or another account's)
    /// is stopped then and not before; on return the profile is free
    /// and the client's poke starts the engine on it. A local-only workspace is
    /// replaced (the sheet says so first).
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
                if result.failure == nil { self.engine = nil }
                self.accountWork = nil
                self.client.engineGone()
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
                if case .success = result { self.engine = nil }
                self.accountWork = nil
                self.client.engineGone()
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
/// which is not what a sheet, a menu or the page should show.
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
