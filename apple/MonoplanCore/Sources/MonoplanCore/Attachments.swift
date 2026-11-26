// What both apps' attachment rows share (spec/attachments.md): where an
// opened file's decrypted bytes go, and how a file is described.
//
// The bytes are online-only. Opening one has the engine fetch and
// decrypt it into a temp file (`Engine.fetchFile`), which Quick Look
// then shows; nothing decrypted is kept in the profile. Each open gets
// a directory of its own under one root in the system's temp directory,
// removed when the preview closes, and the root is emptied when the app
// starts.

import Foundation
import UniformTypeIdentifiers

public enum AttachmentFiles {
    /// The engine's cap on one file (`monoplan_cli::files::MAX_FILE_BYTES`).
    public static let maxBytes: UInt64 = 25 * 1024 * 1024

    private static var root: URL {
        FileManager.default.temporaryDirectory.appendingPathComponent("monoplan-files", isDirectory: true)
    }

    /// A fresh place for one opened file, named as the attachment is so
    /// the preview's title and "Open with" read right.
    public static func tempURL(name: String) throws -> URL {
        let dir = root.appendingPathComponent(UUID().uuidString, isDirectory: true)
        try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        return dir.appendingPathComponent(safeName(name), isDirectory: false)
    }

    /// Remove an opened file and the directory made for it.
    public static func discard(_ url: URL) {
        let dir = url.deletingLastPathComponent()
        guard dir.deletingLastPathComponent().standardizedFileURL == root.standardizedFileURL else { return }
        try? FileManager.default.removeItem(at: dir)
    }

    /// Remove every opened file a run left behind.
    public static func discardAll() {
        try? FileManager.default.removeItem(at: root)
    }

    /// An attachment's name as one path component. The name is whatever
    /// a member of the workspace wrote, so it is never trusted as a
    /// path: separators go, and a name that would be hidden or that
    /// names a directory entry is replaced.
    public static func safeName(_ name: String) -> String {
        var out = name
            .replacingOccurrences(of: "/", with: "_")
            .replacingOccurrences(of: ":", with: "_")
            .replacingOccurrences(of: "\\", with: "_")
            .trimmingCharacters(in: .whitespacesAndNewlines)
        while out.hasPrefix(".") { out.removeFirst() }
        return out.isEmpty ? "file" : out
    }

    /// A file's media type as the system knows it from its name; empty
    /// when it does not.
    public static func mime(of url: URL) -> String {
        UTType(filenameExtension: url.pathExtension)?.preferredMIMEType ?? ""
    }

    /// "1.2 MB".
    public static func sizeText(_ bytes: UInt64) -> String {
        ByteCountFormatter.string(fromByteCount: Int64(clamping: bytes), countStyle: .file)
    }

    /// The SF Symbol for a file of this name: a picture, a film, a
    /// sound, a PDF, or a plain document.
    public static func symbol(name: String) -> String {
        guard let type = UTType(filenameExtension: (name as NSString).pathExtension) else { return "doc" }
        if type.conforms(to: .image) { return "photo" }
        if type.conforms(to: .movie) { return "film" }
        if type.conforms(to: .audio) { return "waveform" }
        if type.conforms(to: .pdf) { return "doc.richtext" }
        if type.conforms(to: .archive) { return "doc.zipper" }
        return "doc"
    }
}

/// An attach in flight, or one that failed and is waiting to be
/// dismissed: the row shown under an item's files until the entry
/// lands in the doc.
public struct FileUpload: Identifiable, Equatable {
    public let id: UUID
    /// The item it is for, in its workspace (nil: Home).
    public let workspace: String?
    public let item: String
    public let name: String
    /// Why it did not happen; nil while it is under way.
    public var error: String?

    public init(workspace: String?, item: String, name: String) {
        id = UUID()
        self.workspace = workspace
        self.item = item
        self.name = name
    }
}

extension ItemFile: Identifiable {}
