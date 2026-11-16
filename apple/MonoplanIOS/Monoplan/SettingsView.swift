// The Settings page, pushed from Home's gear: what a person acts on.
// General (the device's list-counts preference, as the web's Settings
// has it), the account (who is signed in and where, sign in / sign out,
// delete account), sync
// (the status line and when it last synced), and the last error when
// there is one. Everything diagnostic sits a level down in Debug
// (`DebugView.swift`), pushed from the last row. Sign-in is the Mac
// sheet's form without account creation or recovery.

import MonoplanCore
import SwiftUI

struct SettingsView: View {
    @EnvironmentObject private var model: AppModel
    @State private var showSignIn = false
    @State private var confirmSignOut = false
    @State private var showDeleteAccount = false
    @AppStorage(listCountsKey) private var showListCounts = false

    var body: some View {
        List {
            Section("General") {
                Toggle("Show list counts in Home", isOn: $showListCounts)
            }
            accountSection
            syncSection
            if let session = model.session, !session.localOnly {
                Section {
                    Button("Delete Account…", role: .destructive) { showDeleteAccount = true }
                        .disabled(model.busy != nil)
                }
            }
            if let error = model.lastError {
                Section("Last error") {
                    Text(error).foregroundStyle(.red)
                }
            }
            Section {
                NavigationLink("Debug") { DebugView() }
            }
        }
        .navigationTitle("Settings")
        .sheet(isPresented: $showSignIn) {
            SignInView()
                .environmentObject(model)
        }
        .sheet(isPresented: $showDeleteAccount) {
            if let session = model.session {
                DeleteAccountView(email: session.email)
                    .environmentObject(model)
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
                LabeledContent("Server", value: session.serverUrl)
                Button("Sign Out", role: .destructive) { confirmSignOut = true }
                    .disabled(model.busy != nil)
                    .confirmationDialog("Sign out of Monoplan?", isPresented: $confirmSignOut, titleVisibility: .visible) {
                        Button("Sign Out", role: .destructive) { model.signOut() }
                        Button("Cancel", role: .cancel) {}
                    } message: {
                        Text("This removes the account's data from this iPhone. It stays on the server and your other devices.")
                    }
            } else {
                if model.session?.localOnly == true {
                    Text("This device is running a local-only workspace. Signing in replaces it with the account's.")
                        .font(.footnote).foregroundStyle(.secondary)
                }
                Button("Sign In…") { showSignIn = true }
                    .disabled(model.busy != nil)
            }
        }
    }

    private var syncSection: some View {
        Section("Sync") {
            if let sync = model.sync {
                LabeledContent("Status", value: sync.line())
                if !sync.localOnly {
                    LabeledContent("Last synced", value: Self.format(millis: sync.lastSyncedAt))
                }
            } else if case .connecting(let why) = model.engineState {
                LabeledContent("Status", value: why)
            }
        }
    }

    private static func format(millis: Int64?) -> String {
        guard let millis else { return String(localized: "never") }
        return Date(timeIntervalSince1970: Double(millis) / 1000)
            .formatted(date: .abbreviated, time: .standard)
    }
}

/// Settings › Delete Account… (`spec/auth.md` §Account deletion), the
/// macOS sheet on iOS: what goes, then the password, which the server
/// re-verifies. No typed-back email and no emailed confirmation: the
/// password is the re-auth boundary, as on the other clients.
struct DeleteAccountView: View {
    @EnvironmentObject private var model: AppModel
    let email: String
    @Environment(\.dismiss) private var dismiss
    @State private var password = ""
    @FocusState private var passwordFocused: Bool

    private var canSubmit: Bool {
        !model.deletingAccount && !password.isEmpty
    }

    var body: some View {
        NavigationStack {
            Form {
                Section {
                    Text("This deletes the account \(email) and cannot be undone.")
                } footer: {
                    VStack(alignment: .leading, spacing: 4) {
                        Text("Your Personal workspace and every workspace you own are erased from the server for everyone, including anyone they are shared with.")
                        Text("You leave every workspace shared with you.")
                        Text("All your devices are signed out, and the local copy on this iPhone is wiped.")
                    }
                }
                Section {
                    SecureField("Password", text: $password)
                        .textContentType(.password)
                        .focused($passwordFocused)
                        .submitLabel(.done)
                        .onSubmit { if canSubmit { submit() } }
                } footer: {
                    if let error = model.deleteAccountError {
                        Text(error).foregroundStyle(.red)
                    }
                }
                Section {
                    Button(role: .destructive) { submit() } label: {
                        HStack {
                            Text("Delete Account")
                            if model.deletingAccount {
                                Spacer()
                                ProgressView()
                            }
                        }
                    }
                    .disabled(!canSubmit)
                }
            }
            .disabled(model.deletingAccount)
            .navigationTitle("Delete Account")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button("Cancel") { dismiss() }
                        .disabled(model.deletingAccount)
                }
            }
        }
        .interactiveDismissDisabled(model.deletingAccount)
        .onAppear { passwordFocused = true }
        .onDisappear { model.deleteAccountError = nil }
    }

    private func submit() {
        passwordFocused = false
        model.deleteAccount(password: password) { ok in
            if ok { dismiss() }
        }
    }
}
