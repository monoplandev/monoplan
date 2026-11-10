// The app's transport to its own engine: `MonoplanCore.Engine` (the Rust
// loop running in this process) as an `EngineTransport`. Requests block
// in Rust until the loop has answered, so they run on a serial work
// queue, one at a time, as the socket served them; the watch is a Rust
// task calling the sink from the engine's threads, and the client hops
// to its own queue. Never the socket (spec/mac-window-plan.md).

import Foundation
import MonoplanCore
import MonoplanMacKit

final class FFITransport: EngineTransport {
    private let engine: MonoplanCore.Engine
    private var watcher: MonoplanCore.Watcher?
    private let work = DispatchQueue(label: "app.monoplan.mac.engine", qos: .userInitiated)

    init(engine: MonoplanCore.Engine) {
        self.engine = engine
    }

    func request(_ frame: Data, completion: @escaping (Result<Data, Error>) -> Void) {
        work.async { [engine] in
            completion(Result { try engine.request(frame: frame) })
        }
    }

    func watch(onFrame: @escaping (Data) -> Void, onStopped: @escaping () -> Void) {
        let sink = Sink(onFrame: onFrame, onStopped: onStopped)
        do {
            watcher = try engine.watch(sink: sink)
        } catch {
            onStopped()
        }
    }

    func close() {
        watcher?.cancel()
        watcher = nil
    }

    /// The uniffi callback object. Rust calls it from the engine's
    /// runtime threads; the two closures are immutable and hop to the
    /// client's queue themselves, hence unchecked.
    private final class Sink: EngineSink, @unchecked Sendable {
        private let onFrame: (Data) -> Void
        private let onStopped: () -> Void

        init(onFrame: @escaping (Data) -> Void, onStopped: @escaping () -> Void) {
            self.onFrame = onFrame
            self.onStopped = onStopped
        }

        func frame(frame: Data) { onFrame(frame) }
        func stopped() { onStopped() }
    }
}
