// The Settings page, pushed from Home's gear: what a person acts on.
// General (the device's list-counts preference, as the web's Settings
// has it), reminders (this iPhone's two settings, spec/reminders.md),
// place alerts (its one, spec/place-alerts.md),
// the account (who is signed in and where, sign in / sign out,
// delete account), sync
// (the status line and when it last synced), About (links to the
// website and the privacy policy), and the last error when
// there is one. Everything diagnostic sits a level down in Debug
// (`DebugView.swift`), pushed from the last row. Sign-in is the Mac
// sheet's form without account creation or recovery.

import MonoplanCore
import MonoplanMacKit
import SwiftUI

struct SettingsView: View {
    @EnvironmentObject private var model: AppModel
    @State private var showSignIn = false
    @State private var confirmSignOut = false
    @State private var showDeleteAccount = false
    @AppStorage(listCountsKey) private var showListCounts = false
    @AppStorage(ReminderSettings.eventKey) private var eventReminder = ReminderSettings.defaultEvent
    @AppStorage(ReminderSettings.taskKey) private var taskReminder = ReminderSettings.defaultTask
    /// Notifications are turned off for the app in the system's
    /// Settings: no reminder can be delivered whatever is picked here.
    @State private var notificationsDenied = false
    @AppStorage(PlaceAlertSettings.key) private var placeAlerts = false
    @Environment(\.scenePhase) private var scenePhase
    @Environment(\.openURL) private var openURL

    var body: some View {
        List {
            Section("General") {
                Toggle("Show list counts in Home", isOn: $showListCounts)
            }
            remindersSection
            PlaceAlertsSection(
                isOn: $placeAlerts, location: model.placeLocation, notificationsDenied: notificationsDenied)
            accountSection
            syncSection
            Section("About") {
                Link("Website", destination: URL(string: "https://monoplan.app/")!)
                Link("Privacy Policy", destination: URL(string: "https://monoplan.app/privacy/")!)
            }
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
            // Debug builds only: an archive (Release) has no Debug row.
            #if DEBUG
            Section {
                NavigationLink("Debug") { DebugView() }
            }
            #endif
        }
        .navigationTitle("Settings")
        .onChange(of: eventReminder) { model.remindersSettingsChanged() }
        .onChange(of: taskReminder) { model.remindersSettingsChanged() }
        .onChange(of: placeAlerts) { model.placeAlertsSettingChanged() }
        // Read again on the way back from the system's Settings.
        .onChange(of: scenePhase, initial: true) {
            guard scenePhase == .active else { return }
            ReminderScheduler.shared.readDenied { notificationsDenied = $0 }
        }
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

    /// A notification for an item with a time, by kind. A setting of
    /// this iPhone alone: each device has its own.
    private var remindersSection: some View {
        Section {
            Picker("Events", selection: $eventReminder) {
                Text("10 minutes before").tag(EventReminder.tenMinutes)
                Text("Off").tag(EventReminder.off)
            }
            Picker("Scheduled tasks", selection: $taskReminder) {
                Text("At the time").tag(TaskReminder.atTime)
                Text("10 minutes before").tag(TaskReminder.tenMinutes)
                Text("Off").tag(TaskReminder.off)
            }
            if notificationsDenied {
                Button("Turn On Notifications…") {
                    if let url = URL(string: UIApplication.openNotificationSettingsURLString) { openURL(url) }
                }
            }
        } header: {
            Text("Reminders")
        } footer: {
            if notificationsDenied {
                Text("Notifications are turned off for Monoplan, so no reminders are shown.")
            } else {
                Text("For items with a time, on this iPhone only.")
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
                    .signOutConfirmation(isPresented: $confirmSignOut)
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

/// A notification on arriving at a place with open tasks. A setting of
/// this iPhone alone. While it is on and something in the system's
/// Settings stops the alerts, the footer says what, and a button goes
/// there.
private struct PlaceAlertsSection: View {
    @Binding var isOn: Bool
    @ObservedObject var location: PlaceAlertLocation
    let notificationsDenied: Bool
    @Environment(\.openURL) private var openURL

    /// What stops the alerts, if something does.
    private var blocked: String? {
        guard isOn else { return nil }
        switch location.access {
        case .denied: return String(localized: "Location is turned off for Monoplan, so no alerts are shown.")
        case .approximate:
            return String(localized: "Monoplan has only your approximate location, so no alerts are shown. Turn on Precise Location for it in Settings.")
        case .unknown, .allowed:
            return notificationsDenied
                ? String(localized: "Notifications are turned off for Monoplan, so no alerts are shown.") : nil
        }
    }

    var body: some View {
        Section {
            Toggle("Alert on arrival", isOn: $isOn)
            if blocked != nil {
                Button("Open Settings…") {
                    if let url = URL(string: UIApplication.openSettingsURLString) { openURL(url) }
                }
            }
        } header: {
            Text("Places")
        } footer: {
            if let blocked {
                Text(blocked)
            } else {
                Text("When you arrive somewhere you have open tasks, on this iPhone only. Your location never leaves it.")
            }
        }
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

/// The one confirmation before Sign Out, for both places that offer it
/// (Settings' account section and Home's account menu).
private struct SignOutConfirmation: ViewModifier {
    @EnvironmentObject private var model: AppModel
    @Binding var isPresented: Bool

    func body(content: Content) -> some View {
        content.confirmationDialog("Sign out of Monoplan?", isPresented: $isPresented, titleVisibility: .visible) {
            Button("Sign Out", role: .destructive) { model.signOut() }
            Button("Cancel", role: .cancel) {}
        } message: {
            Text("This removes the account's data from this iPhone. It stays on the server and your other devices.")
        }
    }
}

extension View {
    func signOutConfirmation(isPresented: Binding<Bool>) -> some View {
        modifier(SignOutConfirmation(isPresented: isPresented))
    }
}
