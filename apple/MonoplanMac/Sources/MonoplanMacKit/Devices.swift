// The account's devices (spec/auth.md "Devices"): `GET /api/devices`,
// `PATCH /api/devices/:id {name}`, `DELETE /api/devices/:id`, with the
// engine's device token as `Authorization: Bearer`. The Settings window's
// Devices pane (spec/mac-window-plan.md "Settings") is the caller; the
// requests and the parsing are here so they are testable without a
// server. The web client's Settings › Devices is the same three calls.

import Foundation

public struct DeviceRow: Decodable, Equatable, Identifiable {
    public let id: String
    public let name: String
    /// `web`, or a native app's host platform (`macos`, `ios`, …).
    public let kind: String
    /// Unix millis of the last authed call.
    public let lastSeenAt: Int64
    public let lastSeenIp: String?

    enum CodingKeys: String, CodingKey {
        case id, name, kind
        case lastSeenAt = "last_seen_at"
        case lastSeenIp = "last_seen_ip"
    }

    public init(id: String, name: String, kind: String, lastSeenAt: Int64, lastSeenIp: String?) {
        self.id = id
        self.name = name
        self.kind = kind
        self.lastSeenAt = lastSeenAt
        self.lastSeenIp = lastSeenIp
    }
}

public enum Devices {
    /// The list reply: `{"devices": [...]}`.
    public static func parse(_ data: Data) throws -> [DeviceRow] {
        struct List: Decodable { let devices: [DeviceRow] }
        return try JSONDecoder().decode(List.self, from: data).devices
    }

    public static func list(server: String, token: String) -> URLRequest? {
        request("GET", server: server, token: token, path: "/api/devices")
    }

    public static func rename(server: String, token: String, id: String, name: String) -> URLRequest? {
        request("PATCH", server: server, token: token, path: "/api/devices/\(escape(id))", json: ["name": name])
    }

    public static func revoke(server: String, token: String, id: String) -> URLRequest? {
        request("DELETE", server: server, token: token, path: "/api/devices/\(escape(id))")
    }

    /// An SF Symbol for the device's `kind`.
    public static func symbol(forKind kind: String) -> String {
        switch kind {
        case "web": return "globe"
        case "macos": return "laptopcomputer"
        case "ios": return "iphone"
        case "android": return "smartphone"
        case "linux", "windows": return "desktopcomputer"
        default: return "questionmark.circle"
        }
    }

    /// What to show for a non-2xx reply: the server's `message`
    /// (spec/auth.md: every error is `{code, message}`), else the status.
    public static func errorMessage(status: Int, body: Data) -> String {
        if let obj = try? JSONSerialization.jsonObject(with: body) as? [String: Any],
           let message = obj["message"] as? String, !message.isEmpty {
            return message
        }
        return "HTTP \(status)"
    }

    private static func escape(_ id: String) -> String {
        id.addingPercentEncoding(withAllowedCharacters: .urlPathAllowed) ?? id
    }

    private static func request(_ method: String, server: String, token: String, path: String,
                                json: [String: Any]? = nil) -> URLRequest? {
        // An absolute http(s) URL or nothing: newer Foundation accepts
        // almost any string as a relative URL.
        guard let url = URL(string: server + path), url.scheme != nil, url.host != nil else { return nil }
        var req = URLRequest(url: url)
        req.httpMethod = method
        req.setValue("Bearer \(token)", forHTTPHeaderField: "Authorization")
        req.setValue("application/json", forHTTPHeaderField: "Accept")
        if let json {
            req.setValue("application/json", forHTTPHeaderField: "Content-Type")
            req.httpBody = try? JSONSerialization.data(withJSONObject: json, options: [.sortedKeys])
        }
        req.timeoutInterval = 30
        return req
    }
}
