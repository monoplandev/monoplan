// Export JSON… and Import JSON… on a workspace's "…" menu
// (`MainWindow.swift`), as the web's workspace menu has them
// (`js/web/src/nav.tsx`): the engine's JSON export of one workspace
// (`Engine.exportJson`, `spec/data-model.md` "JSON export / import"),
// saved where the user picks as `<name>-<date>.json`, and an export
// read back in (`Engine.importJson`): additive, rows already there by
// id skipped, one undo step. Readable and lossy: no CRDT history.
//
// The panels grant the sandbox access to the one file picked
// (`com.apple.security.files.user-selected.read-write`).

import AppKit
import MonoplanCore
import UniformTypeIdentifiers

enum WorkspaceJSON {
    /// Export JSON…: ask where, then read the export and write it.
    static func save(workspace: String?, name: String, model: MainWindowModel, on window: NSWindow?) {
        let panel = NSSavePanel()
        panel.allowedContentTypes = [.json]
        panel.canCreateDirectories = true
        panel.nameFieldStringValue = "\(slug(name))-\(today()).json"
        present(panel, on: window) {
            guard let url = panel.url else { return }
            model.exportJson(workspace: workspace) { result in
                do {
                    try Data(result.get().utf8).write(to: url, options: .atomic)
                } catch {
                    WorkspaceConfirm.failed(
                        String(localized: "Could not export"), message(error), on: window)
                }
            }
        }
    }

    /// Import JSON…: pick a file, import it, and say what it added.
    static func load(workspace: String?, model: MainWindowModel, on window: NSWindow?) {
        let panel = NSOpenPanel()
        panel.allowedContentTypes = [.json]
        panel.allowsMultipleSelection = false
        panel.canChooseDirectories = false
        present(panel, on: window) {
            guard let url = panel.url else { return }
            let json: String
            do {
                json = try String(contentsOf: url, encoding: .utf8)
            } catch {
                return WorkspaceConfirm.failed(
                    String(localized: "Could not import the file"), error.localizedDescription, on: window)
            }
            model.importJson(workspace: workspace, json: json) { result in
                switch result {
                case .success(let counts): imported(counts, on: window)
                case .failure(let error):
                    WorkspaceConfirm.failed(
                        String(localized: "Could not import the file"), message(error), on: window)
                }
            }
        }
    }

    private static func imported(_ counts: ImportCounts, on window: NSWindow?) {
        let alert = NSAlert()
        alert.messageText = String(localized: "Import finished")
        var lines = [
            String(localized: "Items added: \(Int(counts.itemsAdded))"),
            String(localized: "Lists added: \(Int(counts.listsAdded))"),
        ]
        let existing = Int(counts.itemsExisting) + Int(counts.listsExisting)
        if existing > 0 {
            lines.append(String(localized: "Already present, skipped: \(existing)"))
        }
        alert.informativeText = lines.joined(separator: "\n")
        if let window { alert.beginSheetModal(for: window) } else { alert.runModal() }
    }

    /// Run a panel as a sheet on the window, or modal without one;
    /// `chosen` runs when the user confirms.
    private static func present(_ panel: NSSavePanel, on window: NSWindow?, chosen: @escaping () -> Void) {
        if let window {
            panel.beginSheetModal(for: window) { if $0 == .OK { chosen() } }
        } else if panel.runModal() == .OK {
            chosen()
        }
    }

    private static func message(_ error: Error) -> String {
        error is EngineError ? engineErrorMessage(error) : error.localizedDescription
    }

    /// The file name's stem from the workspace's name, as the web's
    /// `exportSlug`: lower case ASCII letters and digits, dashes
    /// between, at most 40; "monoplan" when nothing is left.
    static func slug(_ name: String) -> String {
        let folded = name.folding(options: [.diacriticInsensitive, .caseInsensitive], locale: nil).lowercased()
        var out = ""
        var dash = false
        for scalar in folded.unicodeScalars {
            if scalar.isASCII, CharacterSet.alphanumerics.contains(scalar) {
                if dash, !out.isEmpty { out.append("-") }
                out.unicodeScalars.append(scalar)
                dash = false
            } else {
                dash = true
            }
        }
        let stem = String(out.prefix(40)).trimmingCharacters(in: CharacterSet(charactersIn: "-"))
        return stem.isEmpty ? "monoplan" : stem
    }

    /// Today as `yyyy-MM-dd`, in UTC as the web's `toISOString`.
    private static func today() -> String {
        let f = DateFormatter()
        f.locale = Locale(identifier: "en_US_POSIX")
        f.timeZone = TimeZone(identifier: "UTC")
        f.dateFormat = "yyyy-MM-dd"
        return f.string(from: Date())
    }
}
