// Client of `monoplan daemon` over its Unix socket (cli/src/ipc.rs,
// spec/daemon-plan.md "IPC protocol"): length-prefixed MessagePack
// frames, hello, then `Watch` so the one connection carries both the
// replies to our requests and the daemon's `Event` frames. Requests are
// serial (one in flight), so a non-event frame is always the reply to
// the oldest pending request.
//
// Lifecycle: `start()` connects, with a capped backoff while nothing
// listens. If no daemon holds the profile's `daemon.lock`, one is
// started the way the CLI does it (`monoplan daemon --detach`), when a
// `monoplan` binary can be found. All callbacks run on `queue`.

import Foundation
import Network

public struct SyncStatus: Equatable {
    public let online: Bool
    public let lastSyncedAt: Int64?

    public init(online: Bool, lastSyncedAt: Int64?) {
        self.online = online
        self.lastSyncedAt = lastSyncedAt
    }

    /// Mirrors `SyncStatus::offline_notice` in cli/src/ipc.rs.
    public func offlineNotice(now: Date = Date()) -> String? {
        if online { return nil }
        guard let at = lastSyncedAt else { return "Offline · never synced" }
        let secs = max(0, Int(now.timeIntervalSince1970) - Int(at / 1000))
        let ago: String
        if secs < 60 { ago = "\(secs)s" }
        else if secs < 3600 { ago = "\(secs / 60)m" }
        else if secs < 86_400 { ago = "\(secs / 3600)h" }
        else { ago = "\(secs / 86_400)d" }
        return "Offline · last synced \(ago) ago"
    }

    static func from(_ v: MPValue?) -> SyncStatus? {
        guard let online = v?["online"]?.boolValue else { return nil }
        return SyncStatus(online: online, lastSyncedAt: v?["last_synced_at"]?.intValue)
    }
}

public enum Request {
    case hello(buildId: String)
    case watch
    case agenda(days: Int)
    /// `monoplan focus --json`: the Starred lens (spec/focus.md).
    case focus

    var value: MPValue {
        switch self {
        case .hello(let buildId):
            return .map(["Hello": .map(["build_id": .string(buildId)])])
        case .watch:
            return .string("Watch")
        case .agenda(let days):
            return .map(["Doc": .map(["Agenda": .map([
                "days": .int(Int64(days)), "today": .nil, "json": .bool(true),
            ])])])
        case .focus:
            return .map(["Doc": .map(["Focus": .map(["cmd": .nil, "json": .bool(true)])])])
        }
    }
}

public enum WatchEvent: Equatable {
    case changed
    case sync(SyncStatus)
}

public enum Reply: Equatable {
    case hello(buildId: String, pid: Int64, sync: SyncStatus)
    case output(stdout: String, sync: SyncStatus)
    case watching(sync: SyncStatus)
    case event(WatchEvent)
    case shuttingDown
    case error(String)

    public enum ParseError: Error { case unrecognised(MPValue) }

    static func parse(_ v: MPValue) throws -> Reply {
        if case .string("ShuttingDown") = v { return .shuttingDown }
        guard let map = v.mapValue, map.count == 1, let (name, body) = map.first else {
            throw ParseError.unrecognised(v)
        }
        switch name {
        case "Hello":
            if let id = body["build_id"]?.stringValue, let pid = body["pid"]?.intValue,
               let sync = SyncStatus.from(body["sync"]) {
                return .hello(buildId: id, pid: pid, sync: sync)
            }
        case "Output":
            if let out = body["stdout"]?.stringValue, let sync = SyncStatus.from(body["sync"]) {
                return .output(stdout: out, sync: sync)
            }
        case "Watching":
            if let sync = SyncStatus.from(body["sync"]) { return .watching(sync: sync) }
        case "Event":
            if case .string("Changed") = body { return .event(.changed) }
            if let sync = SyncStatus.from(body["Sync"]) { return .event(.sync(sync)) }
        case "Err":
            if let msg = body["message"]?.stringValue { return .error(msg) }
        default:
            break
        }
        throw ParseError.unrecognised(v)
    }
}

public enum ClientError: Error, LocalizedError {
    case notConnected
    case closed
    case daemon(String)
    case protocolError(String)

    public var errorDescription: String? {
        switch self {
        case .notConnected: return "not connected to the daemon"
        case .closed: return "the daemon closed the connection"
        case .daemon(let m): return m
        case .protocolError(let m): return "protocol: \(m)"
        }
    }
}

/// Where the profile lives and how to reach its daemon. Same rules as
/// `Profile` / `ipc::socket_path` in the CLI.
public struct ProfileLocation {
    public let dir: URL

    /// `MONOPLAN_DATA_DIR` verbatim, else the platform default the CLI
    /// uses (`dirs::data_local_dir()/monoplan`).
    public static func resolve(environment: [String: String] = ProcessInfo.processInfo.environment) -> ProfileLocation {
        if let raw = environment["MONOPLAN_DATA_DIR"], !raw.isEmpty {
            return ProfileLocation(dir: URL(fileURLWithPath: raw).standardizedFileURL)
        }
        let support = FileManager.default.urls(for: .applicationSupportDirectory, in: .userDomainMask)[0]
        return ProfileLocation(dir: support.appendingPathComponent("monoplan", isDirectory: true))
    }

    public init(dir: URL) { self.dir = dir }

    /// Beside the db when the path fits `sun_path`, otherwise the
    /// hash-named fallback in the temp dir; both computed exactly as
    /// `ipc::socket_path` does.
    public func socketPath(tempDir: String = ProcessInfo.processInfo.environment["TMPDIR"] ?? "/tmp") -> String {
        let direct = dir.path + "/daemon.sock"
        if direct.utf8.count <= 100 { return direct }
        let hash = String(format: "%016llx", Self.fnv1a(Array(dir.path.utf8)))
        let base = tempDir.hasSuffix("/") ? tempDir : tempDir + "/"
        return base + "monoplan-\(hash).sock"
    }

    public var lockPath: String { dir.path + "/daemon.lock" }

    /// Is a daemon holding the profile? Same probe as `daemon::is_running`:
    /// try the flock and release it at once.
    public func daemonIsRunning() -> Bool {
        let fd = open(lockPath, O_RDONLY)
        if fd < 0 { return false }
        defer { close(fd) }
        if flock(fd, LOCK_EX | LOCK_NB) == 0 {
            flock(fd, LOCK_UN)
            return false
        }
        return errno == EWOULDBLOCK
    }

    static func fnv1a(_ bytes: [UInt8]) -> UInt64 {
        var h: UInt64 = 0xcbf29ce484222325
        for b in bytes {
            h ^= UInt64(b)
            h = h &* 0x100000001b3
        }
        return h
    }
}

public final class DaemonClient {
    public enum State: Equatable {
        case connecting(String)
        case connected(SyncStatus)
    }

    public var onEvent: ((WatchEvent) -> Void)?
    public var onState: ((State) -> Void)?
    public private(set) var state: State = .connecting("starting") {
        didSet { if state != oldValue { onState?(state) } }
    }

    private let location: ProfileLocation
    private let queue: DispatchQueue
    private var connection: NWConnection?
    private var pending: [(Result<Reply, Error>) -> Void] = []
    private var backoff: TimeInterval = 1
    private var lastSpawnAttempt: Date = .distantPast
    private var stopped = false

    public init(location: ProfileLocation, queue: DispatchQueue = .main) {
        self.location = location
        self.queue = queue
    }

    public func start() {
        stopped = false
        connect()
    }

    public func stop() {
        stopped = true
        connection?.cancel()
        connection = nil
        fail(all: ClientError.closed)
    }

    /// One request, replied to in order. Fails immediately when there
    /// is no connection rather than queueing: the caller refreshes
    /// again on reconnect anyway.
    public func send(_ request: Request, completion: @escaping (Result<Reply, Error>) -> Void) {
        guard let connection, case .connected = state else {
            completion(.failure(ClientError.notConnected))
            return
        }
        pending.append(completion)
        write(request.value, on: connection)
    }

    // MARK: Connection

    private func connect() {
        if stopped { return }
        let path = location.socketPath()
        let conn = NWConnection(to: .unix(path: path), using: .tcp)
        connection = conn
        conn.stateUpdateHandler = { [weak self] st in
            guard let self, self.connection === conn else { return }
            switch st {
            case .ready:
                self.backoff = 1
                self.handshake(conn)
            case .failed(let e):
                self.dropConnection(reason: e.localizedDescription)
            case .cancelled:
                self.dropConnection(reason: "closed")
            case .waiting(let e):
                // No listener: a daemon may need starting.
                self.dropConnection(reason: e.localizedDescription)
            default:
                break
            }
        }
        conn.start(queue: queue)
    }

    private func handshake(_ conn: NWConnection) {
        // The CLI uses the build id to restart a stale daemon; we are
        // not built from the daemon's sources, so we cannot judge
        // staleness and just identify ourselves. The daemon does not
        // enforce a match.
        pending.append { [weak self] result in
            guard let self else { return }
            guard case .success(.hello) = result else {
                self.dropConnection(reason: "bad hello")
                return
            }
            self.pending.append { [weak self] result in
                guard let self else { return }
                guard case .success(.watching(let sync)) = result else {
                    self.dropConnection(reason: "watch refused")
                    return
                }
                self.state = .connected(sync)
            }
            self.write(Request.watch.value, on: conn)
        }
        write(Request.hello(buildId: "monoplan-mac").value, on: conn)
        readFrame(conn)
    }

    private func dropConnection(reason: String) {
        connection?.cancel()
        connection = nil
        fail(all: ClientError.closed)
        if stopped { return }
        maybeSpawnDaemon()
        state = .connecting(reason)
        let delay = backoff
        backoff = min(backoff * 2, 10)
        queue.asyncAfter(deadline: .now() + delay) { [weak self] in
            guard let self, self.connection == nil else { return }
            self.connect()
        }
    }

    private func fail(all error: Error) {
        let waiting = pending
        pending.removeAll()
        for cb in waiting { cb(.failure(error)) }
    }

    // MARK: Framing

    private func write(_ value: MPValue, on conn: NWConnection) {
        let payload = MessagePack.encode(value)
        var frame = Data()
        var len = UInt32(payload.count).bigEndian
        withUnsafeBytes(of: &len) { frame.append(contentsOf: $0) }
        frame.append(payload)
        conn.send(content: frame, completion: .contentProcessed { [weak self] error in
            if let error { self?.dropConnection(reason: error.localizedDescription) }
        })
    }

    private func readFrame(_ conn: NWConnection) {
        conn.receive(minimumIncompleteLength: 4, maximumLength: 4) { [weak self] data, _, complete, error in
            guard let self, self.connection === conn else { return }
            if let error { return self.dropConnection(reason: error.localizedDescription) }
            guard let data, data.count == 4 else {
                return self.dropConnection(reason: complete ? "closed" : "short length prefix")
            }
            let len = Int(data.withUnsafeBytes { $0.loadUnaligned(as: UInt32.self) }.bigEndian)
            guard len > 0, len <= 64 * 1024 * 1024 else {
                return self.dropConnection(reason: "bad frame length \(len)")
            }
            conn.receive(minimumIncompleteLength: len, maximumLength: len) { [weak self] data, _, _, error in
                guard let self, self.connection === conn else { return }
                if let error { return self.dropConnection(reason: error.localizedDescription) }
                guard let data, data.count == len else {
                    return self.dropConnection(reason: "closed mid-frame")
                }
                self.dispatch(data)
                self.readFrame(conn)
            }
        }
    }

    private func dispatch(_ payload: Data) {
        let reply: Reply
        do {
            reply = try Reply.parse(try MessagePack.decode(payload))
        } catch {
            dropConnection(reason: "undecodable frame: \(error)")
            return
        }
        switch reply {
        case .event(let ev):
            if case .sync(let sync) = ev, case .connected = state {
                state = .connected(sync)
            }
            onEvent?(ev)
        case .shuttingDown:
            dropConnection(reason: "daemon shutting down")
        default:
            guard !pending.isEmpty else {
                dropConnection(reason: "unsolicited reply")
                return
            }
            let cb = pending.removeFirst()
            if case .output(_, let sync) = reply, case .connected = state {
                state = .connected(sync)
            }
            if case .error(let m) = reply {
                cb(.failure(ClientError.daemon(m)))
            } else {
                cb(.success(reply))
            }
        }
    }

    // MARK: Spawning

    /// Start a daemon the way the CLI would, at most once per backoff
    /// window, and only when nothing holds the lock. Readiness is not
    /// awaited: the connect loop retries and picks it up.
    private func maybeSpawnDaemon() {
        guard !location.daemonIsRunning(),
              Date().timeIntervalSince(lastSpawnAttempt) > 5,
              let bin = Self.findBinary()
        else { return }
        lastSpawnAttempt = Date()
        let proc = Process()
        proc.executableURL = URL(fileURLWithPath: bin)
        proc.arguments = ["daemon", "--detach"]
        var env = ProcessInfo.processInfo.environment
        env["MONOPLAN_DATA_DIR"] = location.dir.path
        proc.environment = env
        proc.standardInput = FileHandle.nullDevice
        proc.standardOutput = FileHandle.nullDevice
        if let log = try? FileHandle(forWritingTo: URL(fileURLWithPath: location.dir.path + "/daemon.log")) {
            log.seekToEndOfFile()
            proc.standardError = log
        } else {
            proc.standardError = FileHandle.nullDevice
        }
        do {
            try proc.run()
            NSLog("MonoplanMac: started %@ daemon --detach (pid %d)", bin, proc.processIdentifier)
        } catch {
            NSLog("MonoplanMac: could not start %@: %@", bin, error.localizedDescription)
        }
    }

    /// `MONOPLAN_BIN`, then the usual install spots, then a debug build
    /// in the current directory (running from the repo).
    static func findBinary(environment: [String: String] = ProcessInfo.processInfo.environment) -> String? {
        var candidates: [String] = []
        if let bin = environment["MONOPLAN_BIN"], !bin.isEmpty { candidates.append(bin) }
        let home = FileManager.default.homeDirectoryForCurrentUser.path
        candidates += [
            home + "/.cargo/bin/monoplan",
            "/opt/homebrew/bin/monoplan",
            "/usr/local/bin/monoplan",
            FileManager.default.currentDirectoryPath + "/target/debug/monoplan",
        ]
        return candidates.first { FileManager.default.isExecutableFile(atPath: $0) }
    }
}
