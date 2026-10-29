// The app's transport to its own host: `MonoplanCore.Host` (the Rust
// loop running in this process) as a `HostTransport`. Requests block
// in Rust until the loop has answered, so they run on a serial work
// queue, one at a time, as the socket served them; the watch is a Rust
// task calling the sink from the host's threads, and the client hops
// to its own queue. Never the socket (spec/mac-window-plan.md).

import Foundation
import MonoplanCore
import MonoplanMacKit

final class FFITransport: HostTransport {
    private let host: MonoplanCore.Host
    private var watcher: MonoplanCore.Watcher?
    private let work = DispatchQueue(label: "app.monoplan.mac.host", qos: .userInitiated)

    init(host: MonoplanCore.Host) {
        self.host = host
    }

    func request(_ frame: Data, completion: @escaping (Result<Data, Error>) -> Void) {
        work.async { [host] in
            completion(Result { try host.request(frame: frame) })
        }
    }

    func watch(onFrame: @escaping (Data) -> Void, onStopped: @escaping () -> Void) {
        let sink = Sink(onFrame: onFrame, onStopped: onStopped)
        do {
            watcher = try host.watch(sink: sink)
        } catch {
            onStopped()
        }
    }

    func close() {
        watcher?.cancel()
        watcher = nil
    }

    /// The uniffi callback object. Rust calls it from the host's
    /// runtime threads; the two closures are immutable and hop to the
    /// client's queue themselves, hence unchecked.
    private final class Sink: HostSink, @unchecked Sendable {
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
