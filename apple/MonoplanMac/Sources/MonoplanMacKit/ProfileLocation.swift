// The profile directory the app's engine runs on, and what its files
// say without an engine: whether there is a profile, whether an account
// is behind it, which server it syncs with. The profile is the app's
// own, never the CLI's (spec/daemon-plan.md, amended 2026-11-15): the
// app and a CLI on the same Mac are two devices of the account.

import Foundation

/// Where the profile lives: the files are the CLI's `Profile` layout,
/// in a directory of the app's own.
public struct ProfileLocation {
    public let dir: URL

    /// `app.monoplan.mac` under Application Support, which is inside the
    /// app's container (both apps are sandboxed). `MONOPLAN_APP_DATA_DIR`
    /// picks another profile: a relative value is a directory beside
    /// the default one under Application Support (`dev` is
    /// `…/Application Support/dev`), since the sandbox refuses paths
    /// outside the container; an absolute value is taken verbatim, for
    /// tests and unsandboxed runs. Not the CLI's `MONOPLAN_DATA_DIR`, so
    /// a shell that exports it never points the app at the CLI's profile.
    public static func resolve(environment: [String: String] = ProcessInfo.processInfo.environment) -> ProfileLocation {
        let support = FileManager.default.urls(for: .applicationSupportDirectory, in: .userDomainMask)[0]
        if let raw = environment["MONOPLAN_APP_DATA_DIR"], !raw.isEmpty {
            let dir = raw.hasPrefix("/")
                ? URL(fileURLWithPath: raw)
                : support.appendingPathComponent(raw, isDirectory: true)
            return ProfileLocation(dir: dir.standardizedFileURL)
        }
        return ProfileLocation(dir: support.appendingPathComponent("app.monoplan.mac", isDirectory: true))
    }

    public init(dir: URL) { self.dir = dir }

    /// A profile exists, by the CLI's own rule (`Profile::exists`): one
    /// of its two marker files is there. `secrets.toml` is the keys
    /// themselves, in a build that cannot use the keychain;
    /// `keychain.toml` names the keychain item that holds them
    /// (`spec/cli.md` "Local state"). True for an account's device and
    /// for a local-only workspace alike (`cli/src/bootstrap.rs`); the
    /// engine runs either. False only before the first bootstrap and
    /// between a sign-out's wipe and the next one.
    public var hasProfile: Bool {
        ["secrets.toml", "keychain.toml"].contains {
            FileManager.default.fileExists(atPath: dir.appendingPathComponent($0).path)
        }
    }

    /// The profile has an account behind it (`Profile::has_account`):
    /// `secrets.toml` carries a `device_token`, or `keychain.toml` says
    /// `has_account = true` (the token itself is in the keychain, which
    /// this never asks). False for a local-only workspace, which is
    /// what the Sign In… offers hang on.
    public var hasAccount: Bool {
        if let text = try? String(contentsOf: dir.appendingPathComponent("secrets.toml"), encoding: .utf8) {
            return Self.tomlString(key: "device_token", in: text) != nil
        }
        guard let text = try? String(contentsOf: dir.appendingPathComponent("keychain.toml"), encoding: .utf8) else {
            return false
        }
        return Self.tomlBool(key: "has_account", in: text) == true
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
        guard let value = tomlValue(key: key, in: text),
              value.count >= 2, value.hasPrefix("\""), value.hasSuffix("\"") else { return nil }
        return String(value.dropFirst().dropLast())
    }

    static func tomlBool(key: String, in text: String) -> Bool? {
        switch tomlValue(key: key, in: text) {
        case "true": return true
        case "false": return false
        default: return nil
        }
    }

    /// The raw text after `key =` on the first line that sets it.
    private static func tomlValue(key: String, in text: String) -> String? {
        for rawLine in text.split(whereSeparator: \.isNewline) {
            let line = rawLine.trimmingCharacters(in: .whitespaces)
            guard line.hasPrefix(key) else { continue }
            let rest = line.dropFirst(key.count).trimmingCharacters(in: .whitespaces)
            guard rest.hasPrefix("=") else { continue }
            return rest.dropFirst().trimmingCharacters(in: .whitespaces)
        }
        return nil
    }
}
