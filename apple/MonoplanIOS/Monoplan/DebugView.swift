// The Debug page, pushed from Settings: what the engine reports that
// only a developer reads. The raw sync flags and the `Changed` events
// counted as they arrive, with Sync now (push now) and Reconnect (drop
// the socket and dial again); the engine's build, pid, profile path and
// the profile's size on disc, file by file; and the session's
// ids (account, device, each open doc). Account actions and the plain
// sync status are in Settings (`SettingsView.swift`).

import MonoplanCore
import MonoplanMacKit
import SwiftUI

struct DebugView: View {
    @EnvironmentObject private var model: AppModel
    @State private var localFiles: [(name: String, bytes: Int64)]?

    var body: some View {
        List {
            syncSection
            engineSection
            sessionSection
        }
        .navigationTitle("Debug")
        .navigationBarTitleDisplayMode(.inline)
        .task { await measureLocalData() }
        .refreshable {
            model.syncNow()
            model.loadSession()
            await measureLocalData()
        }
    }

    private func measureLocalData() async {
        let dir = model.location.dir
        localFiles = await Task.detached { Self.fileSizes(dir) }.value
    }

    /// Bytes allocated to every regular file under `dir`, largest
    /// first: the whole profile, not the sqlite file alone, since the
    /// WAL sidecar can hold a real share of it between checkpoints.
    private nonisolated static func fileSizes(_ dir: URL) -> [(name: String, bytes: Int64)] {
        let keys: Set<URLResourceKey> = [.isRegularFileKey, .totalFileAllocatedSizeKey, .fileSizeKey]
        guard let files = FileManager.default.enumerator(
            at: dir, includingPropertiesForKeys: Array(keys)
        ) else { return [] }
        var sizes: [(name: String, bytes: Int64)] = []
        let root = dir.standardizedFileURL.path + "/"
        for case let url as URL in files {
            guard let values = try? url.resourceValues(forKeys: keys),
                  values.isRegularFile == true else { continue }
            let path = url.standardizedFileURL.path
            let name = path.hasPrefix(root) ? String(path.dropFirst(root.count)) : path
            sizes.append((name, Int64(values.totalFileAllocatedSize ?? values.fileSize ?? 0)))
        }
        return sizes.sorted { $0.bytes > $1.bytes }
    }

    private static func format(bytes: Int64) -> String {
        ByteCountFormatter.string(fromByteCount: bytes, countStyle: .file)
    }

    private var syncSection: some View {
        Section("Sync") {
            if let sync = model.sync {
                LabeledContent("Online", value: sync.online ? "yes" : "no")
                LabeledContent("Pending local work", value: sync.pending ? "yes" : "no")
                LabeledContent("Local only", value: sync.localOnly ? "yes" : "no")
                if let reason = sync.offlineReason {
                    LabeledContent("Offline reason", value: reason)
                }
                LabeledContent("Doc changes seen", value: "\(model.changeCount)")
                if let at = model.lastChangeAt {
                    LabeledContent("Last change", value: at.formatted(date: .omitted, time: .standard))
                }
                Button("Sync now") { model.syncNow() }
                    .disabled(sync.localOnly)
                Button("Reconnect") { model.reconnect() }
                    .disabled(sync.localOnly)
            } else if case .connecting(let why) = model.clientState {
                LabeledContent("Status", value: why)
            }
        }
    }

    private var engineSection: some View {
        Section("Engine") {
            if let info = model.engineInfo {
                LabeledContent("Serving", value: "yes")
                LabeledContent("Build", value: info.buildId)
                LabeledContent("PID", value: "\(info.pid)")
            } else {
                LabeledContent("Serving", value: "no")
            }
            LabeledContent("Profile") {
                Text(model.location.dir.path)
                    .font(.caption2.monospaced())
                    .multilineTextAlignment(.trailing)
                    .textSelection(.enabled)
            }
            if let files = localFiles {
                LabeledContent(
                    "Local data",
                    value: Self.format(bytes: files.reduce(0) { $0 + $1.bytes }))
                ForEach(files.filter { $0.bytes > 0 }, id: \.name) { file in
                    LabeledContent {
                        Text(Self.format(bytes: file.bytes))
                    } label: {
                        Text(file.name).font(.caption.monospaced())
                    }
                    .foregroundStyle(.secondary)
                }
            }
        }
    }

    @ViewBuilder
    private var sessionSection: some View {
        if let session = model.session, !session.localOnly {
            Section("Session") {
                LabeledContent("Account", value: session.accountId)
                    .font(.footnote)
                LabeledContent("Device", value: session.deviceId)
                    .font(.footnote)
                LabeledContent("Open docs", value: "\(session.docs.count)")
                ForEach(session.docs, id: \.docId) { doc in
                    LabeledContent(doc.name ?? "Personal") {
                        Text(doc.docId).font(.caption2.monospaced())
                    }
                }
            }
        }
    }
}
