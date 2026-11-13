// The Debug page, pushed from Settings: what the engine reports that
// only a developer reads. The raw sync flags and the `Changed` events
// counted as they arrive, with Sync now (push now) and Reconnect (drop
// the socket and dial again); the engine's build, pid and profile path; and the session's
// ids (account, device, each open doc). Account actions and the plain
// sync status are in Settings (`SettingsView.swift`).

import MonoplanCore
import MonoplanMacKit
import SwiftUI

struct DebugView: View {
    @EnvironmentObject private var model: AppModel

    var body: some View {
        List {
            syncSection
            engineSection
            sessionSection
        }
        .navigationTitle("Debug")
        .navigationBarTitleDisplayMode(.inline)
        .refreshable {
            model.syncNow()
            model.loadSession()
        }
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
