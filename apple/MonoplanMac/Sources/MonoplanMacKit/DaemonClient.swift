// Client of the app's own engine (cli/src/daemon.rs `LocalClient`, reached
// through `MonoplanCore.Engine`): the same MessagePack `Request` / `Reply`
// payloads the CLI sends over the Unix socket (cli/src/ipc.rs), as
// function calls on an `EngineTransport`. Hello, then a watch so the
// engine's `Event` frames arrive as they happen; requests are answered
// one by one with their own completion.
//
// The socket is the CLI's path. The app never opens it
// (spec/mac-window-plan.md): the transport is the in-process engine, and
// when there is none the app's engine controller is asked for one
// (`engineProvider`), starts it, and says what the menu should show
// meanwhile. All callbacks run on `queue`.

import Foundation

public struct SyncStatus: Equatable {
    public let online: Bool
    public let lastSyncedAt: Int64?
    /// Why the engine is offline, when it knows (a failed connect, a
    /// dropped socket); nil while online or before the first attempt.
    public let offlineReason: String?
    /// Local work the server has not confirmed: a replica still
    /// pulling, a push in flight, or ops not yet pushed. Absent on the
    /// wire when false.
    public let pending: Bool
    /// No account: the engine never connects, so there is nothing to be
    /// offline from (`ipc::SyncStatus::local_only`). Absent on the
    /// wire when false.
    public let localOnly: Bool

    public init(online: Bool, lastSyncedAt: Int64?, offlineReason: String? = nil, pending: Bool = false,
                localOnly: Bool = false) {
        self.online = online
        self.lastSyncedAt = lastSyncedAt
        self.offlineReason = offlineReason
        self.pending = pending
        self.localOnly = localOnly
    }

    /// The browser footer's fold of the two facts (js/web/src/syncFacts.tsx).
    public enum Level: Equatable {
        case offline
        case syncing
        case synced
    }

    /// A local-only workspace is quiet, like a synced one: no dot, no
    /// notice.
    public var level: Level {
        if localOnly { return .synced }
        if !online { return .offline }
        return pending ? .syncing : .synced
    }

    /// Mirrors `SyncStatus::offline_notice` in cli/src/ipc.rs.
    public func offlineNotice(now: Date = Date()) -> String? {
        if online || localOnly { return nil }
        var line: String
        if let at = lastSyncedAt {
            line = String(localized: "Offline · last synced \(Self.ago(since: at, now: now)) ago")
        } else {
            line = String(localized: "Offline · never synced")
        }
        if let offlineReason { line += " · \(offlineReason)" }
        return line
    }

    /// One line for any state: "Syncing…", "Synced · 2m ago", or the
    /// offline notice. What the window's account menu and the Settings
    /// window show.
    public func line(now: Date = Date()) -> String {
        if localOnly { return String(localized: "Local workspace · not synced") }
        switch level {
        case .syncing:
            return String(localized: "Syncing…")
        case .synced:
            if let at = lastSyncedAt {
                return String(localized: "Synced · \(Self.ago(since: at, now: now)) ago")
            }
            return String(localized: "Synced")
        case .offline:
            return offlineNotice(now: now) ?? String(localized: "Offline")
        }
    }

    /// `12s`, `3m`, `2h`, `5d`: the CLI's `ago` (cli/src/ipc.rs).
    static func ago(since millis: Int64, now: Date) -> String {
        let secs = max(0, Int(now.timeIntervalSince1970) - Int(millis / 1000))
        if secs < 60 { return String(localized: "\(secs)s") }
        if secs < 3600 { return String(localized: "\(secs / 60)m") }
        if secs < 86_400 { return String(localized: "\(secs / 3600)h") }
        return String(localized: "\(secs / 86_400)d")
    }

    static func from(_ v: MPValue?) -> SyncStatus? {
        guard let online = v?["online"]?.boolValue else { return nil }
        return SyncStatus(
            online: online, lastSyncedAt: v?["last_synced_at"]?.intValue,
            offlineReason: v?["offline_reason"]?.stringValue,
            pending: v?["pending"]?.boolValue ?? false,
            localOnly: v?["local_only"]?.boolValue ?? false)
    }
}

/// What a `Request.setWhen` does to the item's duration alongside.
public enum DurationEdit: Equatable {
    /// Leave it to the engine (a first timed `when` defaults it to 60
    /// minutes; clearing `when` clears it).
    case keep
    case clear
    case minutes(Int)
}

/// What a write does to the item's pending date suggestion
/// (`when_suggested`, `WhenSuggest`).
public enum SuggestionEdit: Equatable {
    case keep
    /// Drop it: the user's "no", or its phrase left the title.
    case clear
    /// Raise one: a `when`-shaped value read out of the title.
    case set(String)
}

public enum Request {
    case hello(buildId: String)
    case watch
    /// Push now and report whether the server is reachable
    /// (`Request::Sync`; the reply is `synced`).
    case sync
    /// The network changed under the engine (path change, wake): drop
    /// the socket and connect again now (`Request::Reconnect`).
    case reconnect
    /// `monoplan agenda --days N --json`; `all` is `--all`, every
    /// workspace merged with rows tagged by `doc_id` / `doc_name`.
    case agenda(days: Int, all: Bool = false)
    /// `monoplan starred --json`: the Starred lens (spec/focus.md).
    /// `all` is `--all`: Home's entries with their cross-workspace refs
    /// resolved, rows tagged with `doc_id` / `doc_name` / `list_name`.
    case focus(all: Bool = false)
    /// `monoplan item ls --list <list> [--done] --json`: a list's items
    /// (`Items.parse`); Open only unless `done`. `workspace` is the
    /// `-w` selector (a doc id here); nil is the Personal workspace.
    case items(list: String, done: Bool = false, workspace: String? = nil)
    /// `monoplan item show <id> --json`: one item with its notes and
    /// list name (`ItemDetail.parse`). `workspace` as for `items`.
    case item(id: String, workspace: String? = nil)
    /// `monoplan item add <text> --list <list>`: capture one item at
    /// the end of a list, in Backlog. The reply's stdout is the new
    /// item's id on a line of its own. `workspace` as for `items`. The
    /// frame is pinned on the Rust side by
    /// `add_frame_from_the_apps_decodes_and_applies`.
    case addItem(text: String, list: String, workspace: String? = nil)
    /// `monoplan item set <id> --lifecycle done|backlog`: tick an item,
    /// or reopen a closed one into Backlog (what the core's un-done
    /// writes, `spec/data-model.md` "Set lifecycle"). `workspace` as for
    /// `items`. The frame is pinned on the Rust side by
    /// `set_lifecycle_frame_from_the_apps_decodes_and_applies`.
    case setDone(id: String, done: Bool, workspace: String? = nil)
    /// `monoplan item set <id> --lifecycle <state>`: move the item to
    /// any of the six workflow states. `state` is the name the JSON
    /// reads carry (`backlog`, `todo`, `in_progress`, `review`, `done`,
    /// `cancelled`); the same frame as `setDone`.
    case setLifecycle(id: String, state: String, workspace: String? = nil)
    /// `monoplan item set <id> --text <text> [--when-suggested <when|->]`:
    /// retitle the item, and in the same frame raise or drop the date
    /// suggestion the new title brings (`WhenSuggest.edit`).
    /// `workspace` as for `items`.
    case setText(id: String, text: String, suggestion: SuggestionEdit = .keep, workspace: String? = nil)
    /// `monoplan item set <id> --when-suggested <when|->`: raise a date
    /// suggestion on the item, or dismiss the pending one (nil). The
    /// yes is `setWhen` with the suggested value: any date set clears
    /// the suggestion. The frame is pinned on the Rust side by
    /// `when_suggested_frame_from_the_apps_decodes_and_applies`.
    case setWhenSuggested(id: String, when: String?, workspace: String? = nil)
    /// `monoplan item set <id> --list <list>`: move the item to another
    /// list of the same workspace (appended). `workspace` as for `items`.
    case moveItem(id: String, list: String, workspace: String? = nil)
    /// `monoplan item set <id> --star 1` / `--unstar`: put the item at
    /// the top of Starred (as the web's star does), or take it out. Starred is the Home doc's
    /// whatever `workspace` the item is in; the engine writes the ref
    /// there (`focus::Lens`). The frame is pinned on the Rust side by
    /// `star_frame_from_the_apps_writes_home_from_any_workspace`.
    case setStarred(id: String, starred: Bool, workspace: String? = nil)
    /// `monoplan item set <id> --when <when|-> [--duration <minutes|->]`:
    /// set the planned date (`YYYY-MM-DD` or `YYYY-MM-DDTHH:MM`,
    /// floating) or clear it (nil), and in the same call optionally the
    /// duration (applied after `when`). `workspace` as for `items`.
    case setWhen(id: String, when: String?, duration: DurationEdit = .keep, workspace: String? = nil)
    /// `monoplan item set <id> --duration <minutes|->`: set the
    /// duration in whole minutes, or clear it (nil).
    case setDuration(id: String, minutes: Int?, workspace: String? = nil)
    /// `monoplan item set <id> --place <label|-> [--coords] [--address]
    /// [--new-place]`: set the place, or clear it (nil). A place with
    /// coordinates or an address is written as given. A bare label
    /// takes a known place of that label whole (the engine's rule)
    /// unless `exact`, which writes just the label.
    case setPlace(id: String, place: PlaceInfo?, exact: Bool = false, workspace: String? = nil)
    /// `monoplan item bin <id>` / `monoplan item restore <id>`: set or
    /// clear the bin mask; the workflow state under it is kept.
    /// `workspace` as for `items`.
    case setBinned(id: String, binned: Bool, workspace: String? = nil)
    /// `monoplan item duplicate <id>`: copy the item beside itself, in
    /// its own list (`Doc::duplicate_item`); the reply is the copy's
    /// id on a line. `workspace` as for `items`. The frame is pinned on
    /// the Rust side by `duplicate_frame_from_the_apps_decodes_and_applies`.
    case duplicateItem(id: String, workspace: String? = nil)
    /// `monoplan bin rm <id>`: hard-delete a binned item. The engine
    /// refuses one that is not in the bin. `workspace` as for `items`.
    case deleteBinned(id: String, workspace: String? = nil)
    /// `monoplan places [query] -n <limit> --json`: the account's known
    /// places, most recently used first (`KnownPlace.parse`). Spans
    /// every open doc, so it takes no workspace.
    case places(query: String, limit: Int)
    /// `monoplan find <query> -n <limit> --json`: the search index of
    /// every open doc, merged and ranked (`Find.parse`). Spans every
    /// open doc, so it takes no workspace.
    case find(query: String, limit: Int)
    /// `monoplan done -n <limit> --json`: the Personal workspace's Done
    /// items across every list, newest first by the moment they entered
    /// Done (`Items.parse`; `Done.days` groups them). A full window
    /// (`limit` rows back) means there may be more.
    case done(limit: Int)
    /// `monoplan done --count`: how many Done items the Personal
    /// workspace holds in all, as a bare number (`Done.parseCount`).
    case doneCount
    /// `monoplan list ls --json --all`: id → name, archived included so
    /// every list id an item can carry resolves. `workspace` as for
    /// `items`.
    case lists(workspace: String? = nil)
    /// Who is logged in, from the daemon's account row.
    case account
    /// Create a workspace the account owns, by name, and open it on the
    /// engine (`Request::CreateWorkspace`; the reply is `workspaceCreated`).
    case createWorkspace(name: String)
    /// Delete a workspace by doc id, on the server and on the engine
    /// (`Request::DeleteWorkspace`; the reply is `workspaceDeleted`).
    case deleteWorkspace(docId: String)

    var value: MPValue {
        switch self {
        case .hello(let buildId):
            return .map(["Hello": .map(["build_id": .string(buildId)])])
        case .watch:
            return .string("Watch")
        case .sync:
            return .string("Sync")
        case .reconnect:
            return .string("Reconnect")
        // `Doc { workspace?, cmd }`: no `workspace` means the Personal
        // workspace (`spec/cli.md` §Workspaces), which is all the app shows.
        case .agenda(let days, let all):
            var args: [String: MPValue] = ["days": .int(Int64(days)), "today": .nil, "json": .bool(true)]
            // Absent means false on the wire (`serde(default)`), so the
            // single-doc shape is unchanged.
            if all { args["all"] = .bool(true) }
            return .map(["Doc": .map(["cmd": .map(["Agenda": .map(args)])])])
        case .focus(let all):
            var args: [String: MPValue] = ["cmd": .nil, "json": .bool(true)]
            if all { args["all"] = .bool(true) }
            return .map(["Doc": .map(["cmd": .map(["Focus": .map(args)])])])
        case .items(let list, let done, let workspace):
            return Self.doc(workspace, .map(["Ls": .map([
                "list": .string(list), "done": .bool(done), "json": .bool(true),
            ])]))
        case .item(let id, let workspace):
            return Self.doc(workspace, .map(["Show": .map([
                "item_id": .string(id), "json": .bool(true),
            ])]))
        case .addItem(let text, let list, let workspace):
            return Self.doc(workspace, .map(["Add": .map([
                "list": .string(list), "texts": .array([.string(text)]),
            ])]))
        case .setDone(let id, let done, let workspace):
            return Self.set(id, workspace, ["lifecycle": .string(done ? "Done" : "Backlog")])
        case .setLifecycle(let id, let state, let workspace):
            return Self.set(id, workspace, ["lifecycle": .string(Self.lifecycleVariant(state))])
        case .setText(let id, let text, let suggestion, let workspace):
            // `when_suggested` is absent unless it is being written
            // (`serde(default)`), so a plain retitle's frame is unchanged.
            var fields: [String: MPValue] = ["text": .string(text)]
            switch suggestion {
            case .keep: break
            case .clear: fields["when_suggested"] = .string("-")
            case .set(let when): fields["when_suggested"] = .string(when)
            }
            return Self.set(id, workspace, fields)
        case .setWhenSuggested(let id, let when, let workspace):
            return Self.set(id, workspace, ["when_suggested": .string(when ?? "-")])
        case .moveItem(let id, let list, let workspace):
            return Self.set(id, workspace, ["list": .string(list)])
        case .setStarred(let id, let starred, let workspace):
            return Self.set(id, workspace, starred ? ["star": .map(["At": .int(1)])] : ["unstar": .bool(true)])
        case .setWhen(let id, let when, let duration, let workspace):
            // `-` is the CLI's "clear" for both flags.
            var fields: [String: MPValue] = ["when": .string(when ?? "-")]
            switch duration {
            case .keep: break
            case .clear: fields["duration"] = .string("-")
            case .minutes(let n): fields["duration"] = .string(String(n))
            }
            return Self.set(id, workspace, fields)
        case .setPlace(let id, let place, let exact, let workspace):
            guard let place else { return Self.set(id, workspace, ["place": .string("-")]) }
            var fields: [String: MPValue] = ["place": .string(place.label), "new_place": .bool(exact)]
            if let lat = place.lat, let lon = place.lon { fields["coords"] = .string("\(lat),\(lon)") }
            if let address = place.address, !address.isEmpty { fields["address"] = .string(address) }
            return Self.set(id, workspace, fields)
        case .setBinned(let id, let binned, let workspace):
            return Self.doc(workspace, .map([binned ? "ItemBin" : "Restore": .map(["item_id": .string(id)])]))
        case .duplicateItem(let id, let workspace):
            return Self.doc(workspace, .map(["Duplicate": .map(["item_id": .string(id)])]))
        case .deleteBinned(let id, let workspace):
            return Self.doc(workspace, .map(["Bin": .map([
                "sub": .map(["Rm": .map(["item_id": .string(id)])]), "json": .bool(false),
            ])]))
        case .places(let query, let limit):
            return Self.doc(nil, .map(["Places": .map([
                "query": query.isEmpty ? .nil : .string(query),
                "limit": .int(Int64(limit)), "json": .bool(true),
            ])]))
        case .find(let query, let limit):
            return Self.doc(nil, .map(["Find": .map([
                "query": .string(query), "limit": .int(Int64(limit)), "json": .bool(true),
            ])]))
        case .setDuration(let id, let minutes, let workspace):
            return Self.set(id, workspace, ["duration": .string(minutes.map(String.init) ?? "-")])
        case .done(let limit):
            return .map(["Doc": .map(["cmd": .map(["Done": .map([
                "limit": .int(Int64(limit)), "since": .nil, "json": .bool(true),
            ])])])])
        case .doneCount:
            return .map(["Doc": .map(["cmd": .map(["Done": .map([
                "limit": .nil, "since": .nil, "json": .bool(false), "count": .bool(true),
            ])])])])
        case .lists(let workspace):
            return Self.doc(workspace, .map(["Lists": .map(["cmd": .map(["Ls": .map([
                "json": .bool(true), "archived": .bool(false), "all": .bool(true),
            ])])])]))
        case .account:
            return .string("Account")
        case .createWorkspace(let name):
            return .map(["CreateWorkspace": .map(["name": .string(name)])])
        case .deleteWorkspace(let docId):
            return .map(["DeleteWorkspace": .map(["workspace": .string(docId)])])
        }
    }
}

extension Request {
    /// A workflow state's JSON name as the frame spells it: the Rust
    /// `LifecycleState` variant. An unknown name reads as Backlog, as
    /// it does everywhere else in the apps.
    static func lifecycleVariant(_ state: String) -> String {
        switch state {
        case "todo": return "Todo"
        case "in_progress": return "InProgress"
        case "review": return "Review"
        case "done": return "Done"
        case "cancelled": return "Cancelled"
        default: return "Backlog"
        }
    }

    /// `Doc { Set(SetArgs) }` with `fields` set and every other field
    /// at its unset value (nil, or false for the flags).
    static func set(_ id: String, _ workspace: String?, _ fields: [String: MPValue]) -> MPValue {
        var args: [String: MPValue] = [
            "item_id": .string(id),
            "text": .nil, "list": .nil, "lifecycle": .nil, "when": .nil, "duration": .nil,
            "deadline": .nil, "place": .nil, "coords": .nil, "address": .nil,
            "new_place": .bool(false), "star": .nil, "unstar": .bool(false),
        ]
        args.merge(fields) { _, new in new }
        return doc(workspace, .map(["Set": .map(args)]))
    }

    /// `Doc { workspace?, cmd }`. `workspace` absent on the wire means
    /// the Personal workspace (`serde(default)`), so the frames that
    /// never name one are unchanged.
    static func doc(_ workspace: String?, _ cmd: MPValue) -> MPValue {
        var frame: [String: MPValue] = ["cmd": cmd]
        if let workspace { frame["workspace"] = .string(workspace) }
        return .map(["Doc": .map(frame)])
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
    case synced(sync: SyncStatus)
    case account(accountId: String, email: String, deviceId: String)
    /// `createWorkspace` done: the new doc's id and trimmed name.
    case workspaceCreated(docId: String, name: String, sync: SyncStatus)
    case workspaceDeleted(docId: String, sync: SyncStatus)
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
        case "Synced":
            if let sync = SyncStatus.from(body["sync"]) { return .synced(sync: sync) }
        case "Account":
            if let id = body["account_id"]?.stringValue, let email = body["email"]?.stringValue,
               let device = body["device_id"]?.stringValue {
                return .account(accountId: id, email: email, deviceId: device)
            }
        case "WorkspaceCreated":
            if let id = body["doc_id"]?.stringValue, let name = body["name"]?.stringValue,
               let sync = SyncStatus.from(body["sync"]) {
                return .workspaceCreated(docId: id, name: name, sync: sync)
            }
        case "WorkspaceDeleted":
            if let id = body["doc_id"]?.stringValue, let sync = SyncStatus.from(body["sync"]) {
                return .workspaceDeleted(docId: id, sync: sync)
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
    case engineStopped
    case daemon(String)
    case protocolError(String)

    public var errorDescription: String? {
        switch self {
        case .notConnected: return String(localized: "not connected to the engine")
        case .engineStopped: return String(localized: "the engine has stopped")
        case .daemon(let m): return m
        case .protocolError(let m): return "protocol: \(m)"
        }
    }
}

/// How the client reaches an engine: one msgpack request payload in, one
/// reply payload out, and a watch whose frames (`Watching`, then
/// `Event`s) arrive as the engine sends them. The app's transport is the
/// in-process engine over the FFI; tests use an in-memory one. Callbacks
/// may come on any thread; the client hops to its own queue.
public protocol EngineTransport: AnyObject {
    func request(_ frame: Data, completion: @escaping (Result<Data, Error>) -> Void)
    /// `onStopped` fires once when the engine loop ends; nothing follows.
    func watch(onFrame: @escaping (Data) -> Void, onStopped: @escaping () -> Void)
    /// End the watch. The transport delivers nothing after this.
    func close()
}

/// What the engine controller answers when the client asks for an engine:
/// a transport to a running one, or what to show while it is not
/// (starting, busy, not signed in), after which the client asks again.
public enum EngineProvision {
    case transport(EngineTransport)
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

    /// A profile exists, by the CLI's own rule (`Profile::active`):
    /// `secrets.toml` is there. True for an account's device and for a
    /// local-only workspace alike (`cli/src/bootstrap.rs`); the engine
    /// runs either. False only before the first bootstrap and between
    /// a sign-out's wipe and the next one.
    public var hasProfile: Bool {
        FileManager.default.fileExists(atPath: dir.appendingPathComponent("secrets.toml").path)
    }

    /// The profile has an account behind it (`Profile::has_account`):
    /// `secrets.toml` carries a `device_token`. False for a local-only
    /// workspace, which is what the Sign In… offers hang on.
    public var hasAccount: Bool {
        guard let text = try? String(contentsOf: dir.appendingPathComponent("secrets.toml"), encoding: .utf8) else {
            return false
        }
        return Self.tomlString(key: "device_token", in: text) != nil
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

    /// What the engine said about itself in its `Hello`.
    public struct DaemonInfo: Equatable {
        public let buildId: String
        public let pid: Int64
    }

    public var onEvent: ((WatchEvent) -> Void)?
    public var onState: ((State) -> Void)?
    /// Set on a successful handshake, cleared when the engine goes.
    public private(set) var daemon: DaemonInfo?
    public private(set) var state: State = .connecting(String(localized: "starting")) {
        didSet { if state != oldValue { onState?(state) } }
    }

    private let queue: DispatchQueue
    /// What this client's hello calls itself. The CLI's build id
    /// restarts a stale daemon; the engine here is this process, so the
    /// name only identifies the client (`monoplan-mac`, `monoplan-ios`).
    private let clientId: String
    private var transport: EngineTransport?
    private var backoff: TimeInterval = 1
    private var stopped = false
    private var retryScheduled = false
    /// Consulted whenever there is no transport. The app's engine
    /// controller starts the in-process engine here and hands over a
    /// transport once it runs; nil (tests) means no engine.
    public var engineProvider: (() -> EngineProvision)?

    public init(queue: DispatchQueue = .main, clientId: String = "monoplan-mac") {
        self.queue = queue
        self.clientId = clientId
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
    /// engine just came up.
    public func poke() {
        guard !stopped, transport == nil else { return }
        backoff = 1
        connect()
    }

    /// The engine this client is on has ended, by the caller's own doing
    /// (a sign-in, a sign-up or a sign-out stopped it): forget it now
    /// and ask the provider, without waiting for the watch to say so.
    public func engineGone() {
        if stopped { return }
        backoff = 1
        if transport == nil { connect() } else { drop(reason: String(localized: "engine stopped")) }
    }

    /// One request with its own reply. Fails immediately when there is
    /// no engine rather than queueing: the caller refreshes again once
    /// there is one anyway.
    public func send(_ request: Request, completion: @escaping (Result<Reply, Error>) -> Void) {
        guard let transport, case .connected = state else {
            completion(.failure(ClientError.notConnected))
            return
        }
        transport.request(MessagePack.encode(request.value)) { [weak self] result in
            self?.queue.async {
                guard let self, self.transport === transport else {
                    completion(.failure(ClientError.engineStopped))
                    return
                }
                completion(self.reply(from: result))
            }
        }
    }

    // MARK: Engine

    private func connect() {
        if stopped { return }
        guard let engineProvider else {
            state = .connecting(String(localized: "no engine is running"))
            return
        }
        switch engineProvider() {
        case .pending(let why):
            state = .connecting(why)
            scheduleRetry()
        case .transport(let t):
            transport = t
            backoff = 1
            handshake(t)
        }
    }

    private func handshake(_ t: EngineTransport) {
        // The CLI uses the build id to restart a stale daemon; the engine
        // is this process, so we only identify ourselves.
        t.request(MessagePack.encode(Request.hello(buildId: clientId).value)) { [weak self] result in
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
                            self.drop(reason: String(localized: "engine stopped"))
                        }
                    })
            }
        }
    }

    /// The engine went (its loop ended, or a frame made no sense): forget
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
            if case .synced(let sync) = reply, case .connected = state {
                state = .connected(sync)
            }
            if case .workspaceCreated(_, _, let sync) = reply, case .connected = state {
                state = .connected(sync)
            }
            if case .workspaceDeleted(_, let sync) = reply, case .connected = state {
                state = .connected(sync)
            }
            if case .error(let m) = reply { return .failure(ClientError.daemon(m)) }
            return .success(reply)
        }
    }

    /// A watch frame: `Watching` first, then events, or the engine
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
            drop(reason: String(localized: "engine stopped"))
        default:
            drop(reason: "unexpected frame on the watch")
        }
    }
}
