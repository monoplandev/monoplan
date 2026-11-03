// The Settings window: the standard macOS preferences layout, a
// toolbar of icon-over-label panes (a sidebar is for System
// Settings-scale catalogues). Four panes: General (the device
// preferences the app owns for its window, spec/mac-window-plan.md
// "Settings"), Account (what a user looks at), Devices (the account's
// devices over the server API) and Advanced (the host process, the
// command line, the profile on disk). Panes are SwiftUI
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
import MonoplanMacKit

/// What the Devices pane calls the server with: the host's session and
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
        location: ProfileLocation, client: DaemonClient,
        signIn: @escaping (SignInCredentials, @escaping (Error?) -> Void) -> Void,
        signOut: @escaping (@escaping (Error?) -> Void) -> Void,
        credential: @escaping () -> APICredential?
    ) {
        model = SettingsModel(location: location, client: client, signIn: signIn, signOut: signOut, credential: credential)
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

    /// The Account pane's position among the tabs, for `showSignIn`.
    private static let accountPaneIndex = 1

    /// The tray forwards host state and sync events here; the window
    /// does not hook the client itself, so the tray's own callbacks
    /// stay intact.
    func refresh() {
        model.refresh()
    }

    func show() {
        model.refresh()
        NSApp.activate(ignoringOtherApps: true)
        showWindow(nil)
        window?.makeKeyAndOrderFront(nil)
    }

    /// Open on the Account pane with the sign-in sheet up (the tray's
    /// "Sign In…").
    func showSignIn() {
        (contentViewController as? NSTabViewController)?.selectedTabViewItemIndex = Self.accountPaneIndex
        show()
        model.showSignIn = true
    }
}

/// What both panes show. Everything here is read from the profile
/// files or the host's hello; nothing is editable yet, since sign-in /
/// sign-out rewrite the profile and stay with the CLI for now.
final class SettingsModel: ObservableObject {
    @Published var serverURL: String = "—"
    @Published var signedIn = false
    /// From the host's account row; nil until it answers.
    @Published var email: String?
    @Published var profilePath: String
    @Published var hostLine: String = String(localized: "Not running")
    /// The package version. From the host's hello once it answers
    /// (`version:mtime`, the mtime being the CLI's staleness check,
    /// not something to show); the bundle's own version until then.
    /// Either way the bundle's commit follows it when the build script
    /// recorded one.
    @Published var buildLine: String = SettingsModel.withCommit(SettingsModel.bundleVersion)
    @Published var syncLine: String = "—"
    /// Settings › General. Saved and applied app-wide on change.
    @Published var appearance: AppearancePreference = .load()
    /// Settings › General, delivered to the window's page
    /// (`WindowSettings`).
    @Published var typedDates: Bool = WindowSettings.load().typedDates
    /// Settings › General, but doc content: the Home workspace's
    /// `show_list_counts` through the host (`WorkspaceSettings`). Nil
    /// until the host answers, and again when there is no host; the
    /// toggle is disabled then.
    @Published var showListCounts: Bool?
    @Published var showListCountsError: String?
    @Published var openAtLogin = false
    @Published var loginError: String?
    @Published var signingOut = false
    @Published var signOutError: String?
    @Published var showSignIn = false
    @Published var signingIn = false
    @Published var signInError: String?
    @Published var cliLine: String = "—"
    @Published var cliError: String?
    /// Settings › Devices. Nil until the first load (or without a
    /// signed-in host); reloaded when the pane appears and after every
    /// rename or revoke, never on doc events (it is server state).
    @Published var devices: [DeviceRow]?
    @Published var devicesError: String?
    @Published var devicesBusy = false
    @Published var thisDeviceId: String?
    /// The row the rename sheet is up for.
    @Published var renaming: DeviceRow?
    /// nil when there is no bundled binary to link to (a `swift run`).
    let cli: CommandLineLink? = bundledBinary().map { CommandLineLink(target: $0) }

    private let location: ProfileLocation
    private let client: DaemonClient
    private let signInAction: (SignInCredentials, @escaping (Error?) -> Void) -> Void
    private let signOutAction: (@escaping (Error?) -> Void) -> Void
    private let credential: () -> APICredential?
    private let api: URLSession = {
        let c = URLSessionConfiguration.ephemeral
        c.httpCookieAcceptPolicy = .never
        c.httpShouldSetCookies = false
        return URLSession(configuration: c)
    }()

    init(
        location: ProfileLocation, client: DaemonClient,
        signIn: @escaping (SignInCredentials, @escaping (Error?) -> Void) -> Void,
        signOut: @escaping (@escaping (Error?) -> Void) -> Void,
        credential: @escaping () -> APICredential?
    ) {
        self.location = location
        self.client = client
        self.signInAction = signIn
        self.signOutAction = signOut
        self.credential = credential
        profilePath = location.dir.path
        refresh()
    }

    /// The sheet's submit. The password goes straight to Rust and is
    /// never stored here; the sheet closes itself on success.
    func signIn(_ creds: SignInCredentials, done: @escaping (Bool) -> Void) {
        signingIn = true
        signInError = nil
        signInAction(creds) { [weak self] error in
            guard let self else { return }
            self.signingIn = false
            self.signInError = error?.localizedDescription
            self.refresh()
            done(error == nil)
        }
    }

    /// Confirm, then stop the host and wipe the profile. Local state
    /// only: everything is on the server, and signing in again pulls
    /// it back.
    func signOut() {
        let alert = NSAlert()
        alert.messageText = String(localized: "Sign out of Monoplan?")
        alert.informativeText = String(localized: "This removes the local copy of your data from this Mac. Your data stays on the server and comes back when you sign in again.")
        alert.alertStyle = .warning
        alert.addButton(withTitle: String(localized: "Sign Out"))
        alert.addButton(withTitle: String(localized: "Cancel"))
        guard alert.runModal() == .alertFirstButtonReturn else { return }
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

    func refresh() {
        serverURL = location.serverURL ?? "—"
        signedIn = location.isSignedIn
        openAtLogin = SMAppService.mainApp.status == .enabled
        if let cli {
            switch cli.state {
            case .installed: cliLine = String(localized: "Installed · \(cli.link)")
            case .other(let what): cliLine = "\(cli.link) → \(what)"
            case .missing: cliLine = String(localized: "Not installed")
            }
        } else {
            cliLine = String(localized: "No bundled binary (run the built app)")
        }
        switch client.state {
        case .connecting(let reason):
            hostLine = String(localized: "Not running · \(reason)")
            syncLine = "—"
            buildLine = Self.withCommit(Self.bundleVersion)
            showListCounts = nil
        case .connected(let sync):
            hostLine = String(localized: "Running")
            if let d = client.daemon {
                buildLine = Self.withCommit(String(d.buildId.split(separator: ":", maxSplits: 1).first ?? ""))
            }
            syncLine = sync.online ? String(localized: "Online") : (sync.offlineNotice() ?? String(localized: "Offline"))
            client.send(.account) { [weak self] result in
                if case .success(.account(_, let email, _)) = result {
                    self?.email = email
                }
            }
            // The tray refreshes this window on every doc change too,
            // so a toggle from the page or another device shows here.
            fetchWorkspaceSettings()
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
    /// host has not answered or there is no profile.
    var statusLine: String {
        guard signedIn else { return String(localized: "Not signed in") }
        if let email { return String(localized: "Signed in (\(email))") }
        return String(localized: "Signed in")
    }

    func setAppearance(_ pref: AppearancePreference) {
        appearance = pref
        pref.activate()
    }

    func setTypedDates(_ on: Bool) {
        typedDates = on
        var s = WindowSettings.load()
        s.typedDates = on
        s.save()
    }

    /// Optimistic; the reply (the settings after the change) settles
    /// it, an error puts it back.
    func setShowListCounts(_ on: Bool) {
        let before = showListCounts
        showListCounts = on
        showListCountsError = nil
        client.send(.setShowListCounts(on)) { [weak self] result in
            guard let self else { return }
            switch result {
            case .success(.output(let stdout, _)):
                self.showListCounts = (try? WorkspaceSettings.parse(stdout))?.showListCounts ?? before
            case .success:
                self.showListCounts = before
            case .failure(let e):
                self.showListCounts = before
                self.showListCountsError = e.localizedDescription
            }
        }
    }

    private func fetchWorkspaceSettings() {
        client.send(.settings) { [weak self] result in
            guard let self else { return }
            if case .success(.output(let stdout, _)) = result, let s = try? WorkspaceSettings.parse(stdout) {
                self.showListCounts = s.showListCounts
            } else {
                self.showListCounts = nil
            }
        }
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

    /// One API call: the credential off the main thread (the host's
    /// `session()` blocks briefly), the request, the reply on main.
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

    /// Symlink the bundled CLI onto PATH, at the app's version so its
    /// hello is accepted.
    func installCLI() {
        guard let cli else { return }
        do {
            try cli.install()
            cliError = nil
        } catch {
            cliError = error.localizedDescription
        }
        refresh()
    }
}

/// The device preferences the app owns for its window
/// (spec/mac-window-plan.md "Settings"). Appearance drives
/// `NSApp.appearance`, which the webview follows through
/// `prefers-color-scheme`; the page itself holds no theme in the app.
/// The rest reach the page as `WindowSettings`, except list counts,
/// which are the workspace's own and go through the host.
struct GeneralPane: View {
    @ObservedObject var model: SettingsModel

    var body: some View {
        Form {
            Picker("Appearance", selection: Binding(
                get: { model.appearance },
                set: { model.setAppearance($0) }
            )) {
                Text("System").tag(AppearancePreference.system)
                Text("Light").tag(AppearancePreference.light)
                Text("Dark").tag(AppearancePreference.dark)
            }
            .pickerStyle(.segmented)
            Toggle("Recognise dates as you type", isOn: Binding(
                get: { model.typedDates },
                set: { model.setTypedDates($0) }
            ))
            Toggle("Show list counts", isOn: Binding(
                get: { model.showListCounts ?? true },
                set: { model.setShowListCounts($0) }
            ))
            .disabled(model.showListCounts == nil)
            if let error = model.showListCountsError {
                Text(error).font(.footnote).foregroundStyle(.red)
            }
        }
        .formStyle(.grouped)
        .frame(width: 460)
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
                    Button("Sign In…") { model.showSignIn = true }
                }
            }
            if let error = model.signOutError {
                Text(error).font(.footnote).foregroundStyle(.red)
            }
        }
        .formStyle(.grouped)
        .frame(width: 460)
        .sheet(isPresented: $model.showSignIn) {
            SignInSheet(model: model)
        }
    }
}

/// The add-account sheet: email and password, with the server and
/// device name under a disclosure since almost nobody changes them.
/// The server is part of the account, not a preference, which is why
/// it is set here and only read back on the Account pane.
struct SignInSheet: View {
    @ObservedObject var model: SettingsModel
    @Environment(\.dismiss) private var dismiss
    @State private var email = ""
    @State private var password = ""
    @State private var server = SignInSheet.defaultServer
    @State private var deviceName = Host.current().localizedName ?? "Mac"
    @State private var advanced = false

    static let defaultServer = "https://alpha.monoplan.app"

    private var canSubmit: Bool {
        !model.signingIn && !email.isEmpty && !password.isEmpty && !server.isEmpty && !deviceName.isEmpty
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            Text("Sign in to Monoplan").font(.headline)
            Form {
                TextField("Email", text: $email)
                    .textContentType(.username)
                SecureField("Password", text: $password)
                    .textContentType(.password)
                    .onSubmit { if canSubmit { submit() } }
                DisclosureGroup("Advanced", isExpanded: $advanced) {
                    TextField("Server", text: $server)
                        .textContentType(.URL)
                    TextField("Device name", text: $deviceName)
                }
            }
            .formStyle(.columns)
            .disabled(model.signingIn)
            if let error = model.signInError {
                Text(error).font(.footnote).foregroundStyle(.red)
                    .fixedSize(horizontal: false, vertical: true)
            }
            HStack {
                Link("Create an account…", destination: webURL)
                    .font(.footnote)
                Spacer()
                if model.signingIn { ProgressView().controlSize(.small) }
                Button("Cancel") { dismiss() }
                    .keyboardShortcut(.cancelAction)
                    .disabled(model.signingIn)
                Button("Sign In") { submit() }
                    .keyboardShortcut(.defaultAction)
                    .disabled(!canSubmit)
            }
        }
        .padding(20)
        .frame(width: 400)
        .onDisappear { model.signInError = nil }
    }

    private func submit() {
        let creds = SignInCredentials(
            server: Self.normalise(server), email: email.trimmingCharacters(in: .whitespaces),
            password: password, deviceName: deviceName.trimmingCharacters(in: .whitespaces))
        model.signIn(creds) { ok in
            if ok { dismiss() }
        }
    }

    /// A bare host becomes https; a trailing slash would double up
    /// against the `/api/…` paths the client appends.
    static func normalise(_ raw: String) -> String {
        var s = raw.trimmingCharacters(in: .whitespacesAndNewlines)
        if !s.contains("://") { s = "https://" + s }
        while s.hasSuffix("/") { s.removeLast() }
        return s
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
                } else {
                    Text("Not signed in").foregroundStyle(.secondary)
                }
            }
            if let error = model.devicesError {
                Text(error).font(.footnote).foregroundStyle(.red)
                    .fixedSize(horizontal: false, vertical: true)
            }
        }
        .formStyle(.grouped)
        .frame(width: 460)
        .onAppear { model.loadDevices() }
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

/// The process-level view: the in-app host, the bundled CLI, and
/// where the profile lives. Nothing here needs touching day to day.
struct AdvancedPane: View {
    @ObservedObject var model: SettingsModel

    var body: some View {
        Form {
            LabeledContent("Host", value: model.hostLine)
            LabeledContent("Build", value: model.buildLine)
            LabeledContent("Command line") {
                HStack {
                    Text(model.cliLine)
                        .truncationMode(.middle)
                        .lineLimit(1)
                        .foregroundStyle(.secondary)
                    if let cli = model.cli, cli.state != .installed {
                        Button("Install") { model.installCLI() }
                    }
                }
            }
            if let error = model.cliError {
                Text(error).font(.footnote).foregroundStyle(.red)
            }
            LabeledContent("Profile") {
                HStack {
                    Text(model.profilePath)
                        .truncationMode(.middle)
                        .lineLimit(1)
                        .foregroundStyle(.secondary)
                    Button("Reveal") { model.revealProfile() }
                }
            }
        }
        .formStyle(.grouped)
        .frame(width: 460)
    }
}
