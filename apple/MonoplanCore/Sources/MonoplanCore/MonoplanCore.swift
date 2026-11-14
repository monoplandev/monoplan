// Hand-written Swift over the uniffi-generated API (`Generated/`:
// `monoplan_ffi.swift`, the exports, and `monoplan_cli.swift`, the row
// types its typed calls return; spec/swift-bindings-plan.md). The
// generated symbols (`Engine`, `EngineError`, `Item`, `ListInfo`,
// `login(...)`, ...) are already `public` and part of this module. What
// is here is the little the generated surface cannot say itself: the
// typed calls block until the engine loop has answered, and its events
// arrive on one of the engine's threads.

import Foundation

/// The engine's typed calls, off the caller's thread and in the order
/// they were made:
///
///     let rows = try await calls.run { try $0.items(workspace: nil, list: "inbox", includeDone: false) }
///
/// One serial queue, so a read made after a write sees it. The engine
/// loop orders calls from every client anyway; the queue only keeps
/// this client's own in sequence.
public final class EngineCalls: @unchecked Sendable {
    public let engine: Engine
    private let queue = DispatchQueue(label: "app.monoplan.engine.calls", qos: .userInitiated)

    public init(_ engine: Engine) {
        self.engine = engine
    }

    public func run<T>(_ body: @escaping (Engine) throws -> T) async throws -> T {
        try await withCheckedThrowingContinuation { continuation in
            queue.async { [engine] in
                continuation.resume(with: Result { try body(engine) })
            }
        }
    }

    /// `run` for a caller that is not async: `completion` gets the
    /// result on `queue` (the main queue unless told otherwise). Same
    /// serial queue, so the two forms keep one order between them.
    public func run<T>(
        _ body: @escaping (Engine) throws -> T,
        on queue: DispatchQueue = .main,
        completion: @escaping (Result<T, Error>) -> Void
    ) {
        self.queue.async { [engine] in
            let result = Result { try body(engine) }
            queue.async { completion(result) }
        }
    }
}

// The rows carry their own id, so SwiftUI can list them as they are.
extension Item: Identifiable {}
extension ListInfo: Identifiable {}

extension Engine {
    /// `observe` with closures. Each is called on `queue` (the main
    /// queue unless told otherwise): `onSync` once with the current
    /// state as this returns, then per change; `onChanged` whenever a
    /// doc changed and what is on screen should be read again;
    /// `onStopped` once if the engine loop ends. Keep the returned
    /// `Watcher`: dropping or cancelling it ends the calls.
    public func observe(
        on queue: DispatchQueue = .main,
        onChanged: @escaping () -> Void,
        onSync: @escaping (SyncState) -> Void,
        onStopped: @escaping () -> Void = {}
    ) throws -> Watcher {
        try observe(observer: ClosureObserver(
            queue: queue, onChanged: onChanged, onSync: onSync, onStopped: onStopped))
    }
}

private final class ClosureObserver: EngineObserver, @unchecked Sendable {
    private let queue: DispatchQueue
    private let onChanged: () -> Void
    private let onSync: (SyncState) -> Void
    private let onStopped: () -> Void

    init(
        queue: DispatchQueue,
        onChanged: @escaping () -> Void,
        onSync: @escaping (SyncState) -> Void,
        onStopped: @escaping () -> Void
    ) {
        self.queue = queue
        self.onChanged = onChanged
        self.onSync = onSync
        self.onStopped = onStopped
    }

    func changed() { queue.async(execute: onChanged) }
    func sync(state: SyncState) { queue.async { [onSync] in onSync(state) } }
    func stopped() { queue.async(execute: onStopped) }
}
