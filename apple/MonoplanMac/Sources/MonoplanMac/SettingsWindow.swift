// The Settings window: the standard macOS preferences layout, a
// toolbar of icon-over-label panes (a sidebar is for System
// Settings-scale catalogues). Four panes: General (the device
// preferences the app owns), Account (what a user looks at), Devices (the account's
// devices over the server API) and Advanced (the engine process, the
// profile on disk and its size). Panes are SwiftUI
// views hosted in an AppKit tab view controller, so adding one is a
// view, a label and an SF Symbol.
//
// The app stays an accessory (no Dock icon) even while this window is
// open: an accessory app can activate and give a window key focus, and
// a settings panel does not warrant a Dock presence. The policy flip to
// a regular app is reserved for a real window later.

import AppKit
import ServiceManagement
import SwiftUI
import MonoplanCore
import MonoplanMacKit

/// What the Devices pane calls the server with: the engine's session and
/// device token (never shown, never stored here).
struct APICredential {
    let serverURL: String
    let token: String
    /// This Mac's device id, to mark its row and keep it unrevokable.
    let deviceId: String
}

final class SettingsWindowController: NSWindowController {
    private let model: SettingsModel

    init(
        location: ProfileLocation, engine: EngineController,
        onSignIn: @escaping () -> Void,
        signOut: @escaping (@escaping (Error?) -> Void) -> Void,
        deleteAccount: @escaping (String, @escaping (Error?) -> Void) -> Void,
        credential: @escaping () -> APICredential?
    ) {
        model = SettingsModel(
            location: location, engine: engine, onSignIn: onSignIn, signOut: signOut,
            deleteAccount: deleteAccount, credential: credential)
        // The toolbar tab style: AppKit builds the toolbar from the tab
        // items (one icon + label each) and, with the `.preference`
        // toolbar style, draws it as the full-height icon surface every
        // Settings window on the platform has. The window title tracks
        // the selected pane's label, as it does elsewhere.
        let tabs = NSTabViewController()
        tabs.tabStyle = .toolbar
        // On a tab switch the controller retitles the window from the
        // selected *view controller's* title (not the tab label), and
        // an untitled one reads "Untitled"; so each pane names itself.
        func pane(_ label: String, symbol: String, _ view: some View) -> NSTabViewItem {
            let host = NSHostingController(rootView: view)
            host.title = label
            let item = NSTabViewItem(viewController: host)
            item.label = label
            item.image = NSImage(systemSymbolName: symbol, accessibilityDescription: label)
            return item
        }
        tabs.addTabViewItem(pane(String(localized: "General"), symbol: "gearshape", GeneralPane(model: model)))
        tabs.addTabViewItem(pane(String(localized: "Account"), symbol: "person.crop.circle", AccountPane(model: model)))
        tabs.addTabViewItem(pane(String(localized: "Devices"), symbol: "laptopcomputer.and.iphone", DevicesPane(model: model)))
        tabs.addTabViewItem(pane(String(localized: "Advanced"), symbol: "gearshape.2", AdvancedPane(model: model)))

        let window = NSWindow(contentViewController: tabs)
        window.title = String(localized: "General")
        window.styleMask = [.titled, .closable]
        window.toolbarStyle = .preference
        window.isReleasedWhenClosed = false
        window.setContentSize(NSSize(width: 460, height: 320))
        window.center()
        super.init(window: window)
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }

    /// The Account pane's position among the tabs, for `showAccount`.
    private static let accountPaneIndex = 1

    /// The tray forwards engine state and change events here; the
    /// window does not hook the engine controller itself, so the tray's
    /// own callbacks stay intact.
    func refresh() {
        model.refresh()
    }

    func show() {
        model.refresh()
        NSApp.activate(ignoringOtherApps: true)
        showWindow(nil)
        window?.makeKeyAndOrderFront(nil)
    }

    /// Open on the Account pane.
    func showAccount() {
        (contentViewController as? NSTabViewController)?.selectedTabViewItemIndex = Self.accountPaneIndex
        show()
    }
}

/// What the panes show: read from the profile files and the engine.
final class SettingsModel: ObservableObject {
    @Published var serverURL: String = "—"
    @Published var signedIn = false
    /// From the engine's account row; nil until it answers.
    @Published var email: String?
    @Published var profilePath: String
    @Published var engineLine: String = String(localized: "Not running")
    /// The package version. The engine's own once it runs (`buildId`
    /// is `version:mtime`, the mtime being the CLI's staleness check,
    /// not something to show); the bundle's until then. Either way the
    /// bundle's commit follows it when the build script recorded one.
    @Published var buildLine: String = SettingsModel.withCommit(SettingsModel.bundleVersion)
    @Published var syncLine: String = "—"
    /// Settings › General. Saved and applied app-wide on change.
    @Published var appearance: AppearancePreference = .load()
    /// Settings › General, read by the tray at the next launch
    /// (`LaunchPreference`).
    @Published var openWindowAtLaunch: Bool = LaunchPreference.openWindow()
    @Published var openAtLogin = false
    @Published var loginError: String?
    @Published var signingOut = false
    @Published var signOutError: String?
    @Published var showDeleteAccount = false
    @Published var deletingAccount = false
    @Published var deleteAccountError: String?
    /// Settings › Devices. Nil until the first load (or without a
    /// signed-in engine); reloaded when the pane appears and after every
    /// rename or revoke, never on doc events (it is server state).
    @Published var devices: [DeviceRow]?
    @Published var devicesError: String?
    @Published var devicesBusy = false
    /// Whether the Devices pane is showing: an engine that comes up (a
    /// sign-in, a start) reloads the list only while it is.
    var devicesVisible = false
    @Published var thisDeviceId: String?
    /// The row the rename sheet is up for.
    @Published var renaming: DeviceRow?

    private let location: ProfileLocation
    private let engine: EngineController
    /// Opens the account window (`Onboarding.swift`), which offers
    /// both signing in and creating an account.
    let onSignIn: () -> Void
    private let signOutAction: (@escaping (Error?) -> Void) -> Void
    private let deleteAccountAction: (String, @escaping (Error?) -> Void) -> Void
    private let credential: () -> APICredential?
    private let api: URLSession = {
        let c = URLSessionConfiguration.ephemeral
        c.httpCookieAcceptPolicy = .never
        c.httpShouldSetCookies = false
        return URLSession(configuration: c)
    }()

    init(
        location: ProfileLocation, engine: EngineController,
        onSignIn: @escaping () -> Void,
        signOut: @escaping (@escaping (Error?) -> Void) -> Void,
        deleteAccount: @escaping (String, @escaping (Error?) -> Void) -> Void,
        credential: @escaping () -> APICredential?
    ) {
        self.location = location
        self.deleteAccountAction = deleteAccount
        self.engine = engine
        self.onSignIn = onSignIn
        self.signOutAction = signOut
        self.credential = credential
        profilePath = location.dir.path
        refresh()
    }

    /// Confirm, then stop the engine and wipe the profile. Local state
    /// only: everything is on the server, and signing in again pulls
    /// it back; a fresh local workspace takes its place meanwhile.
    func signOut() {
        guard Self.confirmSignOut() else { return }
        signingOut = true
        signOutError = nil
        signOutAction { [weak self] error in
            guard let self else { return }
            self.signingOut = false
            self.signOutError = error?.localizedDescription
            self.email = nil
            self.refresh()
        }
    }

    /// The question every Sign Out… asks first (here, the tray's and
    /// the app menu's): true when the user went ahead.
    static func confirmSignOut() -> Bool {
        let alert = NSAlert()
        alert.messageText = String(localized: "Sign out of Monoplan?")
        alert.informativeText = String(localized: "Signing out removes this device's local copy of your data.")
        alert.alertStyle = .warning
        alert.addButton(withTitle: String(localized: "Sign Out"))
        alert.addButton(withTitle: String(localized: "Cancel"))
        return alert.runModal() == .alertFirstButtonReturn
    }

    /// The delete sheet's submit.
    /// The password goes straight to Rust. On success the profile is
    /// wiped, as after Sign Out, and the sheet closes itself.
    func deleteAccount(password: String, done: @escaping (Bool) -> Void) {
        deletingAccount = true
        deleteAccountError = nil
        deleteAccountAction(password) { [weak self] error in
            guard let self else { return }
            self.deletingAccount = false
            self.deleteAccountError = error?.localizedDescription
            if error == nil { self.email = nil }
            self.refresh()
            done(error == nil)
        }
    }

    func refresh() {
        serverURL = location.serverURL ?? "—"
        signedIn = location.hasAccount
        openAtLogin = SMAppService.mainApp.status == .enabled
        // Devices are the account's: nothing to show once signed out,
        // and the pane's own load again once an engine serves (the
        // credential needs one).
        if !signedIn {
            devices = nil
            devicesError = nil
        }
        switch engine.state {
        case .connecting(let reason):
            engineLine = String(localized: "Not running · \(reason)")
            syncLine = "—"
            buildLine = Self.withCommit(Self.bundleVersion)
        case .connected(let sync):
            if signedIn, devicesVisible, devices == nil, devicesError == nil, !devicesBusy { loadDevices() }
            engineLine = String(localized: "Running")
            buildLine = Self.withCommit(String(MonoplanCore.buildId().split(separator: ":", maxSplits: 1).first ?? ""))
            syncLine = sync.line()
            engine.calls?.run({ try $0.account() }) { [weak self] result in
                if case .success(let account) = result {
                    self?.email = account?.email
                }
            }
        }
    }

    static var bundleVersion: String {
        Bundle.main.infoDictionary?["CFBundleShortVersionString"] as? String ?? "—"
    }

    /// The short commit `build-app.sh` wrote into Info.plist (a trailing
    /// "+" means the tree was dirty); nil under `swift run`, which has
    /// no bundle.
    static var bundleCommit: String? {
        guard let s = Bundle.main.infoDictionary?["MonoplanGitCommit"] as? String, !s.isEmpty else { return nil }
        return s
    }

    /// "0.0.1-alpha.15 (96e2b3f3a1c0)", or just the version when the
    /// commit is unknown.
    static func withCommit(_ version: String) -> String {
        guard let commit = bundleCommit else { return version }
        return "\(version) (\(commit))"
    }

    /// "Signed in (you@example.com)", or just the state while the
    /// engine has not answered; "Local workspace (no account)" for an
    /// accountless install, which is what a fresh app runs as.
    var statusLine: String {
        guard signedIn else { return String(localized: "Local workspace (no account)") }
        if let email { return String(localized: "Signed in (\(email))") }
        return String(localized: "Signed in")
    }

    func setAppearance(_ pref: AppearancePreference) {
        appearance = pref
        pref.activate()
    }

    func setOpenWindowAtLaunch(_ on: Bool) {
        openWindowAtLaunch = on
        LaunchPreference.setOpenWindow(on)
    }

    // MARK: Devices

    func loadDevices() {
        request({ cred in Devices.list(server: cred.serverURL, token: cred.token) }) { [weak self] result in
            guard let self else { return }
            switch result {
            case .success(let data):
                do {
                    self.devices = try Devices.parse(data)
                    self.devicesError = nil
                } catch {
                    self.devicesError = String(localized: "Could not load devices: \(error.localizedDescription)")
                }
            case .failure(DevicesError.notSignedIn):
                // No signed-in engine to ask with: not an error, just
                // nothing to list until there is one.
                self.devices = nil
                self.devicesError = nil
            case .failure(let error):
                self.devices = nil
                self.devicesError = String(localized: "Could not load devices: \(error.localizedDescription)")
            }
        }
    }

    func renameDevice(_ device: DeviceRow, to name: String) {
        let name = name.trimmingCharacters(in: .whitespaces)
        guard !name.isEmpty, name != device.name else { return }
        request({ cred in Devices.rename(server: cred.serverURL, token: cred.token, id: device.id, name: name) }) { [weak self] result in
            guard let self else { return }
            if case .failure(let error) = result { self.devicesError = error.localizedDescription }
            self.loadDevices()
        }
    }

    /// Confirm, then revoke. Never this Mac: that is Sign Out.
    func revokeDevice(_ device: DeviceRow) {
        guard device.id != thisDeviceId else { return }
        let alert = NSAlert()
        alert.messageText = String(localized: "Revoke “\(device.name)”?")
        alert.informativeText = String(localized: "It will be signed out and will have to sign in again to reach your account.")
        alert.alertStyle = .warning
        alert.addButton(withTitle: String(localized: "Revoke"))
        alert.addButton(withTitle: String(localized: "Cancel"))
        guard alert.runModal() == .alertFirstButtonReturn else { return }
        request({ cred in Devices.revoke(server: cred.serverURL, token: cred.token, id: device.id) }) { [weak self] result in
            guard let self else { return }
            if case .failure(let error) = result { self.devicesError = error.localizedDescription }
            self.loadDevices()
        }
    }

    private enum DevicesError: LocalizedError {
        case notSignedIn
        case badRequest
        case server(String)

        var errorDescription: String? {
            switch self {
            case .notSignedIn: return String(localized: "Not signed in")
            case .badRequest: return String(localized: "The server address is not a URL")
            case .server(let message): return message
            }
        }
    }

    /// One API call: the credential off the main thread (the engine's
    /// `account()` blocks briefly), the request, the reply on main.
    private func request(_ build: @escaping (APICredential) -> URLRequest?,
                         completion: @escaping (Result<Data, Error>) -> Void) {
        devicesBusy = true
        let finish = { [weak self] (result: Result<Data, Error>) in
            DispatchQueue.main.async {
                self?.devicesBusy = false
                completion(result)
            }
        }
        DispatchQueue.global(qos: .userInitiated).async { [weak self, credential, api] in
            guard let cred = credential() else { return finish(.failure(DevicesError.notSignedIn)) }
            DispatchQueue.main.async { self?.thisDeviceId = cred.deviceId }
            guard let req = build(cred) else { return finish(.failure(DevicesError.badRequest)) }
            api.dataTask(with: req) { data, response, error in
                if let error { return finish(.failure(error)) }
                let data = data ?? Data()
                let status = (response as? HTTPURLResponse)?.statusCode ?? 0
                guard (200..<300).contains(status) else {
                    return finish(.failure(DevicesError.server(Devices.errorMessage(status: status, body: data))))
                }
                finish(.success(data))
            }.resume()
        }
    }

    func revealProfile() {
        NSWorkspace.shared.activateFileViewerSelecting([location.dir])
    }

    /// System Settings › Login Items, through the app's own identity:
    /// no launchd plist. Only works from an .app bundle.
    func setOpenAtLogin(_ on: Bool) {
        do {
            if on { try SMAppService.mainApp.register() } else { try SMAppService.mainApp.unregister() }
            loginError = nil
        } catch {
            loginError = error.localizedDescription
        }
        refresh()
    }
}

/// The device preferences the app owns. Appearance drives
/// `NSApp.appearance`; the launch toggle is the tray's
/// (`LaunchPreference`); the reminders are this Mac's two settings
/// (spec/reminders.md), applied by `ReminderScheduler`.
struct GeneralPane: View {
    @ObservedObject var model: SettingsModel
    /// Read by the window's sidebar (`MainWindowView`).
    @AppStorage(listCountsKey) private var showListCounts = false
    @AppStorage(DoneSound.key) private var doneSound = false
    @AppStorage(ReminderSettings.eventKey) private var eventReminder = ReminderSettings.defaultEvent
    @AppStorage(ReminderSettings.taskKey) private var taskReminder = ReminderSettings.defaultTask
    /// Notifications are turned off for the app in System Settings: no
    /// reminder can be shown whatever is picked here.
    @State private var notificationsDenied = false

    var body: some View {
        Form {
            general
            reminders
        }
        .formStyle(.grouped)
        .frame(width: 460)
        .onChange(of: eventReminder) { remindersChanged() }
        .onChange(of: taskReminder) { remindersChanged() }
        .onAppear { readDenied() }
        // Read again on the way back from System Settings.
        .onReceive(NotificationCenter.default.publisher(for: NSApplication.didBecomeActiveNotification)) { _ in
            readDenied()
        }
    }

    private func readDenied() {
        ReminderScheduler.shared.readDenied { notificationsDenied = $0 }
    }

    /// Schedule by the new setting. The first one turned on is when
    /// the system asks whether to allow notifications; a refusal is
    /// read back the next time the app comes forward.
    private func remindersChanged() {
        ReminderScheduler.shared.reschedule(settingsChanged: true)
        readDenied()
    }

    private var reminders: some View {
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
                Button("Open Notification Settings…") {
                    if let url = URL(string: "x-apple.systempreferences:com.apple.Notifications-Settings.extension") {
                        NSWorkspace.shared.open(url)
                    }
                }
            }
        } header: {
            Text("Reminders")
        } footer: {
            if notificationsDenied {
                Text("Notifications are turned off for Monoplan, so no reminders are shown.")
            } else {
                Text("For items with a time, on this Mac only.")
            }
        }
    }

    private var general: some View {
        Section {
            Picker("Appearance", selection: Binding(
                get: { model.appearance },
                set: { model.setAppearance($0) }
            )) {
                Text("System").tag(AppearancePreference.system)
                Text("Light").tag(AppearancePreference.light)
                Text("Dark").tag(AppearancePreference.dark)
            }
            .pickerStyle(.segmented)
            Toggle("Show list counts in sidebar", isOn: $showListCounts)
            Toggle("Play a sound on completing a task", isOn: $doneSound)
            Toggle("Open window at launch", isOn: Binding(
                get: { model.openWindowAtLaunch },
                set: { model.setOpenWindowAtLaunch($0) }
            ))
        }
    }
}

struct AccountPane: View {
    @ObservedObject var model: SettingsModel

    var body: some View {
        Form {
            LabeledContent("Server", value: model.serverURL)
            LabeledContent("Status", value: model.statusLine)
            LabeledContent("Sync", value: model.syncLine)
            Toggle("Open at login", isOn: Binding(
                get: { model.openAtLogin },
                set: { model.setOpenAtLogin($0) }
            ))
            if let error = model.loginError {
                Text(error).font(.footnote).foregroundStyle(.red)
            }
            LabeledContent("") {
                if model.signedIn {
                    Button { model.signOut() } label: {
                        if model.signingOut { Text("Signing out…") } else { Text("Sign Out…") }
                    }
                    .disabled(model.signingOut)
                } else {
                    Button("Sign In…") { model.onSignIn() }
                }
            }
            if let error = model.signOutError {
                Text(error).font(.footnote).foregroundStyle(.red)
            }
            if model.signedIn {
                Section {
                    LabeledContent("") {
                        Button("Delete Account…", role: .destructive) { model.showDeleteAccount = true }
                            .disabled(model.signingOut || model.email == nil)
                    }
                }
            }
        }
        .formStyle(.grouped)
        .frame(width: 460)
        .sheet(isPresented: $model.showDeleteAccount) {
            if let email = model.email {
                DeleteAccountSheet(model: model, email: email)
            }
        }
    }
}

/// Settings › Account › Delete Account… (`spec/auth.md` §Account
/// deletion), the CLI's `auth delete-account` as a sheet: what goes,
/// then the password, which the server re-verifies.
/// No emailed confirmation: the password is the re-auth boundary, as
/// on the web and the CLI.
struct DeleteAccountSheet: View {
    @ObservedObject var model: SettingsModel
    let email: String
    @Environment(\.dismiss) private var dismiss
    @State private var password = ""

    private var canSubmit: Bool {
        !model.deletingAccount && !password.isEmpty
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            Text("Delete your account?").font(.headline)
            Text("This deletes the account \(email) and cannot be undone.")
                .fixedSize(horizontal: false, vertical: true)
            VStack(alignment: .leading, spacing: 4) {
                Text("Your Personal workspace and every workspace you own are erased from the server for everyone, including anyone they are shared with.")
                Text("You leave every workspace shared with you.")
                Text("All your devices are signed out, and the local copy on this Mac is wiped.")
            }
            .font(.footnote).foregroundStyle(.secondary)
            .fixedSize(horizontal: false, vertical: true)
            Form {
                SecureField("Password", text: $password)
                    .textContentType(.password)
                    .onSubmit { if canSubmit { submit() } }
            }
            .formStyle(.columns)
            .disabled(model.deletingAccount)
            if let error = model.deleteAccountError {
                Text(error).font(.footnote).foregroundStyle(.red)
                    .fixedSize(horizontal: false, vertical: true)
            }
            HStack {
                Spacer()
                if model.deletingAccount { ProgressView().controlSize(.small) }
                Button("Cancel") { dismiss() }
                    .keyboardShortcut(.cancelAction)
                    .disabled(model.deletingAccount)
                Button("Delete Account", role: .destructive) { submit() }
                    .disabled(!canSubmit)
            }
        }
        .padding(20)
        .frame(width: 400)
        .onDisappear { model.deleteAccountError = nil }
    }

    private func submit() {
        model.deleteAccount(password: password) { ok in
            if ok { dismiss() }
        }
    }
}

/// The account's devices (spec/auth.md "Devices"), as the browser's
/// Settings › Devices shows them: name, kind, last seen; rename any,
/// revoke any but this Mac. Loaded when the pane appears.
struct DevicesPane: View {
    @ObservedObject var model: SettingsModel

    var body: some View {
        Form {
            if let devices = model.devices {
                ForEach(devices) { device in
                    DeviceRowView(model: model, device: device, isThisMac: device.id == model.thisDeviceId)
                }
            } else if model.devicesError == nil {
                if model.devicesBusy {
                    Text("Loading…").foregroundStyle(.secondary)
                } else if !model.signedIn {
                    Text("Not signed in").foregroundStyle(.secondary)
                } else {
                    // Signed in but no engine to ask with yet (starting);
                    // `refresh` loads once it serves.
                    Text("Waiting for the engine…").foregroundStyle(.secondary)
                }
            }
            if let error = model.devicesError {
                Text(error).font(.footnote).foregroundStyle(.red)
                    .fixedSize(horizontal: false, vertical: true)
            }
        }
        .formStyle(.grouped)
        .frame(width: 460)
        .onAppear {
            model.devicesVisible = true
            model.loadDevices()
        }
        .onDisappear { model.devicesVisible = false }
        .sheet(item: $model.renaming) { device in
            RenameDeviceSheet(model: model, device: device)
        }
    }
}

struct DeviceRowView: View {
    @ObservedObject var model: SettingsModel
    let device: DeviceRow
    let isThisMac: Bool

    var body: some View {
        HStack(spacing: 10) {
            Image(systemName: Devices.symbol(forKind: device.kind))
                .frame(width: 20)
                .foregroundStyle(.secondary)
            VStack(alignment: .leading, spacing: 2) {
                HStack(spacing: 6) {
                    Text(device.name).lineLimit(1).truncationMode(.middle)
                    if isThisMac {
                        Text("This Mac")
                            .font(.caption2)
                            .padding(.horizontal, 5)
                            .padding(.vertical, 1)
                            .background(Color.accentColor.opacity(0.15), in: Capsule())
                    }
                }
                Text("Last seen \(Self.relative(device.lastSeenAt))")
                    .font(.footnote)
                    .foregroundStyle(.secondary)
            }
            Spacer()
            Menu {
                Button("Rename…") { model.renaming = device }
                if !isThisMac {
                    Button("Revoke…", role: .destructive) { model.revokeDevice(device) }
                }
            } label: {
                Image(systemName: "ellipsis.circle")
            }
            .menuStyle(.borderlessButton)
            .menuIndicator(.hidden)
            .fixedSize()
            .disabled(model.devicesBusy)
        }
    }

    static func relative(_ millis: Int64) -> String {
        let f = RelativeDateTimeFormatter()
        f.unitsStyle = .short
        let date = Date(timeIntervalSince1970: TimeInterval(millis) / 1000)
        // "now" for anything within the last minute (this Mac is always
        // fresh: the list call itself bumps it).
        if Date().timeIntervalSince(date) < 60 { return String(localized: "just now") }
        return f.localizedString(for: date, relativeTo: Date())
    }
}

struct RenameDeviceSheet: View {
    @ObservedObject var model: SettingsModel
    let device: DeviceRow
    @Environment(\.dismiss) private var dismiss
    @State private var name: String

    init(model: SettingsModel, device: DeviceRow) {
        self.model = model
        self.device = device
        _name = State(initialValue: device.name)
    }

    private var canSave: Bool {
        !name.trimmingCharacters(in: .whitespaces).isEmpty
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            Text("Rename device").font(.headline)
            TextField("Name", text: $name)
                .onSubmit { if canSave { save() } }
            HStack {
                Spacer()
                Button("Cancel") { dismiss() }
                    .keyboardShortcut(.cancelAction)
                Button("Save") { save() }
                    .keyboardShortcut(.defaultAction)
                    .disabled(!canSave)
            }
        }
        .padding(20)
        .frame(width: 360)
    }

    private func save() {
        model.renameDevice(device, to: name)
        dismiss()
    }
}

/// The process-level view: the in-app engine, its build, and
/// where the profile lives. Nothing here needs touching day to day.
struct AdvancedPane: View {
    @ObservedObject var model: SettingsModel
    @State private var localFiles: [(name: String, bytes: Int64)]?

    /// The profile's size on disc. The row is always present (a dash
    /// until measured) so the pane keeps its height; the file-by-file
    /// split is the row's tooltip.
    private var localDataLine: String {
        guard let files = localFiles else { return "—" }
        return Self.format(bytes: files.reduce(0) { $0 + $1.bytes })
    }

    private var localDataBreakdown: String {
        (localFiles ?? []).filter { $0.bytes > 0 }
            .map { "\($0.name)  \(Self.format(bytes: $0.bytes))" }
            .joined(separator: "\n")
    }

    private func measureLocalData() async {
        let dir = URL(fileURLWithPath: model.profilePath, isDirectory: true)
        localFiles = await Task.detached { Self.fileSizes(dir) }.value
    }

    /// Bytes allocated to every regular file under `dir`, largest
    /// first: the whole profile, not the sqlite file alone, since the
    /// WAL sidecar can hold a real share of it between checkpoints.
    /// Same measure as the iOS Debug page.
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

    var body: some View {
        Form {
            LabeledContent("Engine", value: model.engineLine)
            LabeledContent("Build", value: model.buildLine)
            LabeledContent("Profile") {
                HStack {
                    Text(model.profilePath)
                        .truncationMode(.middle)
                        .lineLimit(1)
                        .foregroundStyle(.secondary)
                    Button("Reveal") { model.revealProfile() }
                }
            }
            LabeledContent("Local data", value: localDataLine)
                .help(localDataBreakdown)
        }
        .formStyle(.grouped)
        .frame(width: 460)
        // Re-measured each time the pane comes up, and when the
        // profile moves (sign in / out).
        .task(id: model.profilePath) { await measureLocalData() }
    }
}
