// The profile directory the app's engine runs on, and what its files
// say without an engine: whether there is a profile, whether an account
// is behind it, which server it syncs with. The app never opens the
// CLI's socket; it reaches its engine through `MonoplanCore.Engine`.

import Foundation

/// Where the profile lives, by the same rules as `Profile` in the CLI.
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
}
