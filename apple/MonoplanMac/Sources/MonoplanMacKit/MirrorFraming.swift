// The window's side of the webview contract (spec/mac-window-plan.md
// "Swift side"; the JS half is js/web/src/sync/mirrorHost.ts): what the
// page posts to the `monoplan` message handler, what the app replies,
// and the calls the app makes back into `window.__monoplanHost`. Pure
// data in and out, so it is testable without WebKit or the host:
// the window controller does the IO around it.
//
// Bytes cross as base64; the peer id as a decimal string (a u64 does
// not fit a JSON number); the doc id is the `-w` selector `Host.subscribe`
// resolves.

import Foundation

/// A request the page posted. `message` is what `WKScriptMessage.body`
/// hands over for a JSON object: `[String: Any]`.
public enum HostRequest: Equatable {
    case session
    case subscribe(docId: String)
    case unsubscribe(docId: String)
    case `import`(docId: String, bytes: Data)
    /// A title-bar gesture on the page's own strip (the window has no
    /// title bar of its own; `.nav-titlebar` stands in for it).
    case window(WindowGesture)
    /// Whether Edit › Undo / Redo should be enabled: the doc's undo
    /// state, or a text field with focus (its own undo). Pushed by the
    /// page whenever either changes.
    case undoState(canUndo: Bool, canRedo: Bool)

    public enum WindowGesture: String, Equatable {
        /// A mouse-down: move the window with it.
        case drag
        /// A double-click: what the system's "double-click a window's
        /// title bar to" setting says.
        case doubleClick
    }

    public enum ParseError: Error, Equatable, LocalizedError {
        case notAnObject
        case unknownKind(String)
        case missing(String)
        case badBase64(String)
        case badValue(String)

        public var errorDescription: String? {
            switch self {
            case .notAnObject: return "request is not an object"
            case .unknownKind(let k): return "unknown request kind \(k)"
            case .missing(let f): return "request is missing \(f)"
            case .badBase64(let f): return "request field \(f) is not base64"
            case .badValue(let f): return "request field \(f) has an unknown value"
            }
        }
    }

    public init(message: Any) throws {
        guard let body = message as? [String: Any], let kind = body["kind"] as? String else {
            throw ParseError.notAnObject
        }
        func string(_ key: String) throws -> String {
            guard let v = body[key] as? String else { throw ParseError.missing(key) }
            return v
        }
        switch kind {
        case "session":
            self = .session
        case "subscribe":
            self = .subscribe(docId: try string("docId"))
        case "unsubscribe":
            self = .unsubscribe(docId: try string("docId"))
        case "import":
            let b64 = try string("bytes")
            guard let data = Data(base64Encoded: b64) else { throw ParseError.badBase64("bytes") }
            self = .import(docId: try string("docId"), bytes: data)
        case "window":
            guard let gesture = WindowGesture(rawValue: try string("action")) else { throw ParseError.badValue("action") }
            self = .window(gesture)
        case "undoState":
            func bool(_ key: String) throws -> Bool {
                guard let v = body[key] as? Bool else { throw ParseError.missing(key) }
                return v
            }
            self = .undoState(canUndo: try bool("canUndo"), canRedo: try bool("canRedo"))
        default:
            throw ParseError.unknownKind(kind)
        }
    }
}

/// The `session` reply: `HostSessionInfo` on the JS side. Built by the
/// app from `MonoplanCore.HostSession`; the device token is not in it
/// and never reaches the page.
public struct WindowSession: Equatable {
    public struct Doc: Equatable {
        public let docId: String
        public let name: String?
        public init(docId: String, name: String?) {
            self.docId = docId
            self.name = name
        }
    }

    public let accountId: String
    public let email: String
    public let deviceId: String
    public let primaryDocId: String
    public let serverURL: String
    public let docs: [Doc]

    public init(accountId: String, email: String, deviceId: String, primaryDocId: String,
                serverURL: String, docs: [Doc]) {
        self.accountId = accountId
        self.email = email
        self.deviceId = deviceId
        self.primaryDocId = primaryDocId
        self.serverURL = serverURL
        self.docs = docs
    }

    /// The reply body, as `WKScriptMessage` reply handlers take it.
    public var reply: [String: Any] {
        [
            "accountId": accountId,
            "email": email,
            "deviceId": deviceId,
            "primaryDocId": primaryDocId,
            "serverUrl": serverURL,
            "docs": docs.map { d -> [String: Any] in
                ["docId": d.docId, "name": d.name as Any? ?? NSNull()]
            },
        ]
    }
}

public enum HostReply {
    /// `{snapshot, peer}` for `subscribe`.
    public static func subscribe(snapshot: Data, peer: UInt64) -> [String: Any] {
        ["snapshot": snapshot.base64EncodedString(), "peer": String(peer)]
    }
}

/// A call into `window.__monoplanHost`, rendered as the JavaScript the
/// app evaluates. Every argument is JSON-encoded, so a doc id or a
/// base64 string can never break out of its literal.
public enum HostInbound: Equatable {
    case update(docId: String, bytes: Data)
    case stopped(docId: String)
    case sync(online: Bool, lastSyncedAt: Int64?)
    case handOff
    /// The app's device settings changed (`WindowSettings`).
    case settings(WindowSettings)
    /// Edit › Undo / Redo: the page undoes (a focused text field's own
    /// undo first, else the doc's), since the window's menu is the one
    /// undo surface there.
    case undo
    case redo
    /// The window's search bubble: open the page's Find palette.
    case find
    /// The window entered or left full screen: the page drops the
    /// title-bar padding there (no traffic lights, no drag).
    case fullScreen(Bool)

    public static let global = "window.__monoplanHost"

    public var script: String {
        switch self {
        case .update(let docId, let bytes):
            return "\(Self.global).update(\(json(docId)), \(json(bytes.base64EncodedString())))"
        case .stopped(let docId):
            return "\(Self.global).stopped(\(json(docId)))"
        case .sync(let online, let at):
            let last = at.map(String.init) ?? "null"
            return "\(Self.global).sync({online: \(online ? "true" : "false"), lastSyncedAt: \(last)})"
        case .handOff:
            return "\(Self.global).handOff()"
        case .settings(let settings):
            return "\(Self.global).settings(\(settings.json))"
        case .undo:
            return "\(Self.global).undo()"
        case .redo:
            return "\(Self.global).redo()"
        case .find:
            return "\(Self.global).find()"
        case .fullScreen(let active):
            return "\(Self.global).fullScreen(\(active ? "true" : "false"))"
        }
    }

    private func json(_ s: String) -> String {
        // JSONSerialization refuses a bare string as a top-level value
        // on older Foundation, so encode a one-element array and strip
        // the brackets.
        let data = try! JSONSerialization.data(withJSONObject: [s], options: [])
        let text = String(decoding: data, as: UTF8.self)
        return String(text.dropFirst().dropLast())
    }
}

/// What the app's URL scheme serves (spec/mac-window-plan.md "Swift
/// side"): the Vite build out of the bundle under `monoplan://app/`,
/// and `/api/*` forwarded to the host's server with the device token.
public enum AppScheme {
    public static let scheme = "monoplan"
    public static let host = "app"
    /// The page the window loads.
    public static let entry = "mirror.html"

    public enum Route: Equatable {
        /// A file of the web build, as a path relative to its root.
        case file(String)
        /// An API call to forward: the path and query to put on the
        /// server URL.
        case api(pathAndQuery: String)
        case notFound
    }

    /// The window's start URL, with an optional fragment (`#item_<id>`
    /// and the other `spec/urls.md` fragments).
    public static func entryURL(fragment: String? = nil) -> URL {
        var c = URLComponents()
        c.scheme = scheme
        c.host = host
        c.path = "/" + entry
        c.fragment = fragment
        return c.url!
    }

    public static func route(_ url: URL) -> Route {
        guard url.scheme?.lowercased() == scheme, url.host?.lowercased() == host else { return .notFound }
        let path = url.path
        if path == "/api" || path.hasPrefix("/api/") {
            var pq = path
            if let q = url.query, !q.isEmpty { pq += "?" + q }
            return .api(pathAndQuery: pq)
        }
        // A bundle path: no traversal (`url.path` is percent-decoded, so
        // an encoded dot-dot is caught too); the root is the entry page.
        // The query is a cache-buster at most.
        let segments = path.split(separator: "/").map(String.init)
        if segments.contains("..") { return .notFound }
        let rel = segments.joined(separator: "/")
        return .file(rel.isEmpty ? entry : rel)
    }

    /// MIME types for what the build contains. WebKit streams wasm
    /// only under `application/wasm`, and treats an unknown type as a
    /// download, so the map is explicit rather than UTType-guessed.
    public static func mimeType(for path: String) -> String {
        let ext = (path as NSString).pathExtension.lowercased()
        switch ext {
        case "html": return "text/html; charset=utf-8"
        case "js", "mjs": return "text/javascript; charset=utf-8"
        case "css": return "text/css; charset=utf-8"
        case "wasm": return "application/wasm"
        case "json", "map": return "application/json; charset=utf-8"
        case "webmanifest": return "application/manifest+json"
        case "svg": return "image/svg+xml"
        case "png": return "image/png"
        case "ico": return "image/x-icon"
        case "jpg", "jpeg": return "image/jpeg"
        case "webp": return "image/webp"
        case "woff": return "font/woff"
        case "woff2": return "font/woff2"
        case "txt": return "text/plain; charset=utf-8"
        default: return "application/octet-stream"
        }
    }

    /// The request headers the proxy forwards verbatim. Anything else
    /// (cookies, origin, user agent) is the webview's business.
    public static let forwardedRequestHeaders = ["Content-Type", "Accept", "X-Request-Id"]
    /// The response headers it passes back.
    public static let forwardedResponseHeaders = ["Content-Type", "X-Request-Id"]
}
