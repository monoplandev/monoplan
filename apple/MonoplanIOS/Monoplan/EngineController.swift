// The app running the profile's engine: the iOS edition of
// `MonoplanMac/Sources/MonoplanMac/EngineController.swift`. Same rules,
// fewer cases: on iOS nothing else can hold the profile (no CLI, no
// headless daemon, one process per container), so there is no takeover
// and "busy" can only be an engine of ours still starting. The engine serves
// no socket here (`monoplan-ffi`, `SERVE_SOCKET`); the client reaches
// it in process through `FFITransport`.
//
// - Whenever the client has no engine, start one. No profile (first
//   launch, or just after a sign-out's wipe) first gets a local-only
//   workspace, so the app works without an account as the web client
//   does (spec/auth.md); a sign-in replaces it.
// - Sign-in and sign-out stop our engine *here*, before the Rust side
//   claims the profile. On macOS the claim stops the engine itself over
//   the socket; with no socket there is nothing for it to signal, and
//   it would wait out its deadline ("daemon did not stop within 10s").
//   The client starts the engine again once the profile is free.
//
// Sharing this with the macOS app is the obvious follow-up: it would
// be a third target beside the Kit, depending on MonoplanCore.

import Foundation
import MonoplanCore
import MonoplanMacKit

final class EngineController {
    private let location: ProfileLocation
    private let client: EngineClient
    private var engine: MonoplanCore.Engine?
    private var starting = false
    /// The last failed start, shown once and then retried.
    private var failure: String?
    /// A sign-in or sign-out in progress: what the client is told, and
    /// a reason not to start an engine meanwhile.
    private var accountWork: String?

    init(location: ProfileLocation, client: EngineClient) {
        self.location = location
        self.client = client
        client.engineProvider = { [weak self] in
            self?.provision() ?? .pending("not signed in")
        }
    }

    /// The engine while it serves; nil between engines.
    var runningEngine: MonoplanCore.Engine? {
        if let engine, engine.isRunning() { return engine }
        return nil
    }

    /// Called by the client whenever it has no engine: a transport to the
    /// running one, else what to show while there is none.
    private func provision() -> EngineProvision {
        if let engine, engine.isRunning() { return .transport(FFITransport(engine: engine)) }
        engine = nil
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
                return try MonoplanCore.Engine.start(profileDir: dir)
            }
            DispatchQueue.main.async { self?.didStart(result) }
        }
        return .pending("starting")
    }

    private func didStart(_ result: Result<MonoplanCore.Engine, Error>) {
        starting = false
        switch result {
        case .success(let h):
            engine = h
            NSLog("MonoplanIOS: engine running on %@", location.dir.path)
        case .failure(EngineError.NotLoggedIn):
            break
        case .failure(EngineError.Busy):
            failure = "profile in use"
        case .failure(let e):
            failure = "engine failed: \(engineErrorMessage(e))"
            NSLog("MonoplanIOS: engine failed: %@", engineErrorMessage(e))
        }
        client.poke()
    }

    /// Clean exit: persist, fold, release the profile. Blocks.
    func stop() {
        engine?.shutdown()
        engine = nil
    }

    /// Sign out: stop our engine and wipe the profile (`monoplan auth
    /// logout`). The client is then told the engine is gone, finds no
    /// profile, and the no-engine path seeds a fresh local-only workspace.
    func signOut(completion: @escaping (Error?) -> Void) {
        accountWork = "signing out"
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

    /// Sign in (`monoplan auth login`), off the main thread for the key
    /// derivation. Our engine is stopped first: the Rust side's claim
    /// cannot reach a socket-less engine, so it would never see the
    /// profile freed. A wrong password therefore costs an engine restart
    /// (the client's poke starts the engine on the untouched local-only workspace
    /// again), which the macOS edition avoids; on success the profile
    /// is rewritten and the poke starts it on the account's.
    func signIn(_ creds: SignInCredentials, completion: @escaping (Error?) -> Void) {
        accountWork = "signing in"
        let dir = location.dir.path
        DispatchQueue.global(qos: .userInitiated).async { [weak self] in
            self?.engine?.shutdown()
            let result = Result {
                try MonoplanCore.login(
                    profileDir: dir, server: creds.server, email: creds.email,
                    password: creds.password, deviceName: creds.deviceName)
            }
            DispatchQueue.main.async {
                guard let self else { return }
                self.engine = nil
                self.accountWork = nil
                self.client.engineGone()
                completion(result.failure.map(EngineFailure.init))
            }
        }
    }

    /// Create an account (`monoplan auth signup`, `MonoplanCore.signup`),
    /// off the main thread. Our engine is stopped first, as for
    /// `signIn`; its shutdown persists the local-only workspace, which
    /// signup then adopts as the account's Home doc (same id, same key;
    /// `spec/auth.md` §Device pairing), so nothing captured before is
    /// lost. The engine the client's poke starts afterwards pushes that
    /// history on its first connect; after a failure it starts on the
    /// untouched local-only workspace again.
    func signUp(_ creds: SignInCredentials, completion: @escaping (Result<SignupOutcome, Error>) -> Void) {
        accountWork = "signing in"
        let dir = location.dir.path
        DispatchQueue.global(qos: .userInitiated).async { [weak self] in
            self?.engine?.shutdown()
            let result = Result {
                try MonoplanCore.signup(
                    profileDir: dir, server: creds.server, email: creds.email,
                    password: creds.password, deviceName: creds.deviceName)
            }
            DispatchQueue.main.async {
                guard let self else { return }
                self.engine = nil
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
