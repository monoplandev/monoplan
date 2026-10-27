// The bundled CLI on the user's PATH. The app hosts the doc itself
// (spec/daemon-plan.md, amended 2026-10-27); the `monoplan` copy in
// the bundle is for the terminal, and a symlink to it keeps the CLI at
// the app's version so its hello never refuses.

import Foundation

/// The `monoplan` copy `build-app.sh` puts beside the app executable.
public func bundledBinary(executable: URL? = Bundle.main.executableURL) -> String? {
    guard let executable else { return nil }
    let path = executable.deletingLastPathComponent().appendingPathComponent("monoplan").path
    return FileManager.default.isExecutableFile(atPath: path) ? path : nil
}

/// `/usr/local/bin/monoplan` → the bundled binary, so the terminal runs
/// the app's version.
public struct CommandLineLink {
    public enum State: Equatable {
        /// The link points at `target`.
        case installed
        /// Something else is at the link path: another symlink target,
        /// or a regular file (a cargo-installed CLI, say).
        case other(String)
        case missing
    }

    public let link: String
    public let target: String

    public init(link: String = "/usr/local/bin/monoplan", target: String) {
        self.link = link
        self.target = target
    }

    public var state: State {
        let fm = FileManager.default
        if let dest = try? fm.destinationOfSymbolicLink(atPath: link) {
            return dest == target ? .installed : .other(dest)
        }
        return fm.fileExists(atPath: link) ? .other(link) : .missing
    }

    /// Replace whatever symlink is at `link` with one to `target`. A
    /// regular file there is left alone: it is someone's install, not
    /// ours to clobber. Falls back to an admin prompt when the
    /// directory is not writable (a stock `/usr/local/bin` is root's).
    public func install() throws {
        let fm = FileManager.default
        if case .other(let what) = state, what == link {
            throw InstallError.fileInTheWay(link)
        }
        do {
            try? fm.removeItem(atPath: link)
            try fm.createSymbolicLink(atPath: link, withDestinationPath: target)
        } catch {
            try Self.linkAsAdmin(link: link, target: target)
        }
        guard state == .installed else { throw InstallError.notInstalled }
    }

    private static func linkAsAdmin(link: String, target: String) throws {
        let shell = "mkdir -p '\(Self.quoted((link as NSString).deletingLastPathComponent))' && ln -sfn '\(Self.quoted(target))' '\(Self.quoted(link))'"
        let proc = Process()
        proc.executableURL = URL(fileURLWithPath: "/usr/bin/osascript")
        proc.arguments = ["-e", "do shell script \"\(shell.replacingOccurrences(of: "\\", with: "\\\\").replacingOccurrences(of: "\"", with: "\\\""))\" with administrator privileges"]
        let err = Pipe()
        proc.standardError = err
        proc.standardOutput = FileHandle.nullDevice
        try proc.run()
        proc.waitUntilExit()
        if proc.terminationStatus != 0 {
            let text = String(data: err.fileHandleForReading.readDataToEndOfFile(), encoding: .utf8) ?? ""
            throw InstallError.admin(text.trimmingCharacters(in: .whitespacesAndNewlines))
        }
    }

    /// Single-quote-safe: a `'` in a path becomes `'\''`.
    private static func quoted(_ s: String) -> String {
        s.replacingOccurrences(of: "'", with: "'\\''")
    }

    public enum InstallError: LocalizedError {
        case fileInTheWay(String)
        case admin(String)
        case notInstalled

        public var errorDescription: String? {
            switch self {
            case .fileInTheWay(let p): return String(localized: "\(p) exists and is not a symlink; remove it first")
            case .admin(let e): return e.isEmpty ? String(localized: "cancelled") : e
            case .notInstalled: return String(localized: "link was not created")
            }
        }
    }
}
