// The Debug tab: everything the host reports, as a list: sync state,
// the host itself, and the session. Buttons for the four things this
// surface can do: sync now, reconnect, sign in, sign out. This was the
// whole app until the tabs (2026-11-05); account actions stay here
// until a settings surface takes them.

import MonoplanCore
import MonoplanMacKit
import SwiftUI

struct DebugView: View {
    @EnvironmentObject private var model: AppModel
    @State private var showSignIn = false

    var body: some View {
        NavigationStack {
            List {
                syncSection
                hostSection
                accountSection
                if let error = model.lastError {
                    Section("Last error") {
                        Text(error).foregroundStyle(.red)
                    }
                }
            }
            .navigationTitle("Debug")
            .refreshable {
                model.syncNow()
                model.loadSession()
            }
            .sheet(isPresented: $showSignIn) {
                SignInView()
                    .environmentObject(model)
            }
        }
    }

    private var syncSection: some View {
        Section("Sync") {
            if let sync = model.sync {
                LabeledContent("Status", value: sync.line())
                LabeledContent("Online", value: sync.online ? "yes" : "no")
                LabeledContent("Pending local work", value: sync.pending ? "yes" : "no")
                LabeledContent("Local only", value: sync.localOnly ? "yes" : "no")
                LabeledContent("Last synced", value: Self.format(millis: sync.lastSyncedAt))
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

    private var hostSection: some View {
        Section("Host") {
            if let info = model.hostInfo {
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

    private var accountSection: some View {
        Section("Account") {
            if let busy = model.busy {
                HStack {
                    ProgressView()
                    Text(busy).padding(.leading, 8)
                }
            }
            if let session = model.session, !session.localOnly {
                LabeledContent("Email", value: session.email)
                LabeledContent("Account", value: session.accountId)
                    .font(.footnote)
                LabeledContent("Device", value: session.deviceId)
                    .font(.footnote)
                LabeledContent("Server", value: session.serverUrl)
                LabeledContent("Open docs", value: "\(session.docs.count)")
                ForEach(session.docs, id: \.docId) { doc in
                    LabeledContent(doc.name ?? "Personal") {
                        Text(doc.docId).font(.caption2.monospaced())
                    }
                }
                Button("Sign Out", role: .destructive) { model.signOut() }
                    .disabled(model.busy != nil)
            } else {
                LabeledContent("Signed in", value: "no")
                if model.session?.localOnly == true {
                    Text("A local-only workspace is being hosted. Signing in replaces it with the account's.")
                        .font(.footnote).foregroundStyle(.secondary)
                }
                Button("Sign In…") { showSignIn = true }
                    .disabled(model.busy != nil)
            }
        }
    }

    private static func format(millis: Int64?) -> String {
        guard let millis else { return "never" }
        return Date(timeIntervalSince1970: Double(millis) / 1000)
            .formatted(date: .abbreviated, time: .standard)
    }
}
