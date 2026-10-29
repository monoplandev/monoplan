// Client of the app's own host (cli/src/daemon.rs `LocalClient`, reached
// through `MonoplanCore.Host`): the same MessagePack `Request` / `Reply`
// payloads the CLI sends over the Unix socket (cli/src/ipc.rs), as
// function calls on a `HostTransport`. Hello, then a watch so the
// host's `Event` frames arrive as they happen; requests are answered
// one by one with their own completion.
//
// The socket is the CLI's path. The app never opens it
// (spec/mac-window-plan.md): the transport is the in-process host, and
// when there is none the app's host controller is asked for one
// (`hostProvider`), starts it, and says what the menu should show
// meanwhile. All callbacks run on `queue`.

import Foundation

public struct SyncStatus: Equatable {
    public let online: Bool
    public let lastSyncedAt: Int64?
    /// Why the host is offline, when it knows (a failed connect, a
    /// dropped socket); nil while online or before the first attempt.
    public let offlineReason: String?

    public init(online: Bool, lastSyncedAt: Int64?, offlineReason: String? = nil) {
        self.online = online
        self.lastSyncedAt = lastSyncedAt
        self.offlineReason = offlineReason
    }

    /// Mirrors `SyncStatus::offline_notice` in cli/src/ipc.rs.
    public func offlineNotice(now: Date = Date()) -> String? {
        if online { return nil }
        var line: String
        if let at = lastSyncedAt {
            let secs = max(0, Int(now.timeIntervalSince1970) - Int(at / 1000))
            let ago: String
            if secs < 60 { ago = String(localized: "\(secs)s") }
            else if secs < 3600 { ago = String(localized: "\(secs / 60)m") }
            else if secs < 86_400 { ago = String(localized: "\(secs / 3600)h") }
            else { ago = String(localized: "\(secs / 86_400)d") }
            line = String(localized: "Offline · last synced \(ago) ago")
        } else {
            line = String(localized: "Offline · never synced")
        }
        if let offlineReason { line += " · \(offlineReason)" }
        return line
    }

    static func from(_ v: MPValue?) -> SyncStatus? {
        guard let online = v?["online"]?.boolValue else { return nil }
        return SyncStatus(
            online: online, lastSyncedAt: v?["last_synced_at"]?.intValue,
            offlineReason: v?["offline_reason"]?.stringValue)
    }
}

public enum Request {
    case hello(buildId: String)
    case watch
    case agenda(days: Int)
    /// `monoplan focus --json`: the Starred lens (spec/focus.md).
    case focus
    /// `monoplan lists ls --json --all`: id → name, archived included so
    /// every list id an item can carry resolves.
    case lists
    /// Who is logged in, from the daemon's account row.
    case account

    var value: MPValue {
        switch self {
        case .hello(let buildId):
            return .map(["Hello": .map(["build_id": .string(buildId)])])
        case .watch:
            return .string("Watch")
        // `Doc { workspace?, cmd }`: no `workspace` means the Personal
        // workspace (`spec/cli.md` §Workspaces), which is all the app shows.
        case .agenda(let days):
            return .map(["Doc": .map(["cmd": .map(["Agenda": .map([
                "days": .int(Int64(days)), "today": .nil, "json": .bool(true),
            ])])])])
        case .focus:
            return .map(["Doc": .map(["cmd": .map(["Focus": .map(["cmd": .nil, "json": .bool(true)])])])])
        case .lists:
            return .map(["Doc": .map(["cmd": .map(["Lists": .map(["cmd": .map(["Ls": .map([
                "json": .bool(true), "archived": .bool(false), "all": .bool(true),
            ])])])])])])
        case .account:
            return .string("Account")
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
    case account(accountId: String, email: String, deviceId: String)
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
        case "Account":
            if let id = body["account_id"]?.stringValue, let email = body["email"]?.stringValue,
               let device = body["device_id"]?.stringValue {
                return .account(accountId: id, email: email, deviceId: device)
            }
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
    case hostStopped
    case daemon(String)
    case protocolError(String)

    public var errorDescription: String? {
        switch self {
        case .notConnected: return String(localized: "not connected to the host")
        case .hostStopped: return String(localized: "the host has stopped")
        case .daemon(let m): return m
        case .protocolError(let m): return "protocol: \(m)"
        }
    }
}

/// How the client reaches a host: one msgpack request payload in, one
/// reply payload out, and a watch whose frames (`Watching`, then
/// `Event`s) arrive as the host sends them. The app's transport is the
/// in-process host over the FFI; tests use an in-memory one. Callbacks
/// may come on any thread; the client hops to its own queue.
public protocol HostTransport: AnyObject {
    func request(_ frame: Data, completion: @escaping (Result<Data, Error>) -> Void)
    /// `onStopped` fires once when the host loop ends; nothing follows.
    func watch(onFrame: @escaping (Data) -> Void, onStopped: @escaping () -> Void)
    /// End the watch. The transport delivers nothing after this.
    func close()
}

/// What the host controller answers when the client asks for a host:
/// a transport to a running one, or what to show while it is not
/// (starting, busy, not signed in), after which the client asks again.
public enum HostProvision {
    case transport(HostTransport)
    case pending(String)
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

    /// Logged in, by the CLI's own rule (`Profile::active`): the
    /// profile holds device credentials, i.e. `secrets.toml` exists.
    public var isSignedIn: Bool {
        FileManager.default.fileExists(atPath: dir.appendingPathComponent("secrets.toml").path)
    }

    /// `server_url` from `config.toml`, or nil when there is no profile.
    /// The file is one flat key per line, so a line scan stands in for
    /// a TOML parser.
    public var serverURL: String? {
        guard let text = try? String(contentsOf: dir.appendingPathComponent("config.toml"), encoding: .utf8) else {
            return nil
        }
        return Self.tomlString(key: "server_url", in: text)
    }

    static func tomlString(key: String, in text: String) -> String? {
        for rawLine in text.split(whereSeparator: \.isNewline) {
            let line = rawLine.trimmingCharacters(in: .whitespaces)
            guard line.hasPrefix(key) else { continue }
            let rest = line.dropFirst(key.count).trimmingCharacters(in: .whitespaces)
            guard rest.hasPrefix("=") else { continue }
            let value = rest.dropFirst().trimmingCharacters(in: .whitespaces)
            guard value.count >= 2, value.hasPrefix("\""), value.hasSuffix("\"") else { return nil }
            return String(value.dropFirst().dropLast())
        }
        return nil
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

    /// What the host said about itself in its `Hello`.
    public struct DaemonInfo: Equatable {
        public let buildId: String
        public let pid: Int64
    }

    public var onEvent: ((WatchEvent) -> Void)?
    public var onState: ((State) -> Void)?
    /// Set on a successful handshake, cleared when the host goes.
    public private(set) var daemon: DaemonInfo?
    public private(set) var state: State = .connecting(String(localized: "starting")) {
        didSet { if state != oldValue { onState?(state) } }
    }

    private let queue: DispatchQueue
    private var transport: HostTransport?
    private var backoff: TimeInterval = 1
    private var stopped = false
    private var retryScheduled = false
    /// Consulted whenever there is no transport. The app's host
    /// controller starts the in-process host here and hands over a
    /// transport once it runs; nil (tests) means nothing is hosting.
    public var hostProvider: (() -> HostProvision)?

    public init(queue: DispatchQueue = .main) {
        self.queue = queue
    }

    public func start() {
        stopped = false
        connect()
    }

    public func stop() {
        stopped = true
        detach()
    }

    /// Ask the provider now instead of waiting out the backoff: the
    /// host just came up.
    public func poke() {
        guard !stopped, transport == nil else { return }
        backoff = 1
        connect()
    }

    /// One request with its own reply. Fails immediately when there is
    /// no host rather than queueing: the caller refreshes again once
    /// there is one anyway.
    public func send(_ request: Request, completion: @escaping (Result<Reply, Error>) -> Void) {
        guard let transport, case .connected = state else {
            completion(.failure(ClientError.notConnected))
            return
        }
        transport.request(MessagePack.encode(request.value)) { [weak self] result in
            self?.queue.async {
                guard let self, self.transport === transport else {
                    completion(.failure(ClientError.hostStopped))
                    return
                }
                completion(self.reply(from: result))
            }
        }
    }

    // MARK: Host

    private func connect() {
        if stopped { return }
        guard let hostProvider else {
            state = .connecting(String(localized: "nothing hosting the profile"))
            return
        }
        switch hostProvider() {
        case .pending(let why):
            state = .connecting(why)
            scheduleRetry()
        case .transport(let t):
            transport = t
            backoff = 1
            handshake(t)
        }
    }

    private func handshake(_ t: HostTransport) {
        // The CLI uses the build id to restart a stale daemon; the host
        // is this process, so we only identify ourselves.
        t.request(MessagePack.encode(Request.hello(buildId: "monoplan-mac").value)) { [weak self] result in
            self?.queue.async {
                guard let self, self.transport === t else { return }
                guard case .success(.hello(let buildId, let pid, _)) = self.reply(from: result) else {
                    self.drop(reason: "bad hello")
                    return
                }
                self.daemon = DaemonInfo(buildId: buildId, pid: pid)
                t.watch(
                    onFrame: { [weak self] data in
                        self?.queue.async {
                            guard let self, self.transport === t else { return }
                            self.dispatch(data)
                        }
                    },
                    onStopped: { [weak self] in
                        self?.queue.async {
                            guard let self, self.transport === t else { return }
                            self.drop(reason: String(localized: "host stopped"))
                        }
                    })
            }
        }
    }

    /// The host went (its loop ended, or a frame made no sense): forget
    /// it and ask the provider, which restarts it if it should run.
    private func drop(reason: String) {
        detach()
        if stopped { return }
        state = .connecting(reason)
        connect()
    }

    private func detach() {
        transport?.close()
        transport = nil
        daemon = nil
    }

    private func scheduleRetry() {
        if retryScheduled { return }
        retryScheduled = true
        let delay = backoff
        backoff = min(backoff * 2, 10)
        queue.asyncAfter(deadline: .now() + delay) { [weak self] in
            guard let self else { return }
            self.retryScheduled = false
            if self.transport == nil { self.connect() }
        }
    }

    // MARK: Frames

    private func reply(from result: Result<Data, Error>) -> Result<Reply, Error> {
        switch result {
        case .failure(let e):
            return .failure(e)
        case .success(let data):
            let reply: Reply
            do {
                reply = try Reply.parse(try MessagePack.decode(data))
            } catch {
                return .failure(ClientError.protocolError("undecodable reply: \(error)"))
            }
            if case .output(_, let sync) = reply, case .connected = state {
                state = .connected(sync)
            }
            if case .error(let m) = reply { return .failure(ClientError.daemon(m)) }
            return .success(reply)
        }
    }

    /// A watch frame: `Watching` first, then events, or the host
    /// announcing its exit.
    private func dispatch(_ payload: Data) {
        let reply: Reply
        do {
            reply = try Reply.parse(try MessagePack.decode(payload))
        } catch {
            drop(reason: "undecodable frame: \(error)")
            return
        }
        switch reply {
        case .watching(let sync):
            state = .connected(sync)
        case .event(let ev):
            if case .sync(let sync) = ev, case .connected = state {
                state = .connected(sync)
            }
            onEvent?(ev)
        case .shuttingDown:
            drop(reason: String(localized: "host stopped"))
        default:
            drop(reason: "unexpected frame on the watch")
        }
    }
}
