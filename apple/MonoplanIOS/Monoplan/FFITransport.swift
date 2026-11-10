// The app's transport to its own engine: `MonoplanCore.Engine` as a
// `EngineTransport`, byte for byte the macOS app's `FFITransport`.
// Requests block in Rust until the loop has answered, so they run on a
// serial queue; the watch is a Rust thread calling the sink, and the
// client hops to its own queue.

import Foundation
import MonoplanCore
import MonoplanMacKit

final class FFITransport: EngineTransport {
    private let engine: MonoplanCore.Engine
    private var watcher: MonoplanCore.Watcher?
    private let work = DispatchQueue(label: "app.monoplan.ios.engine", qos: .userInitiated)

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
