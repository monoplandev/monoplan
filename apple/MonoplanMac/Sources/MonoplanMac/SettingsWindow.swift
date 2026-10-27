// The Settings window: the standard macOS preferences layout, a
// toolbar of icon-over-label panes (a sidebar is for System
// Settings-scale catalogues). Two panes: Account (what a user looks
// at) and Advanced (the host process, the command line, the profile on
// disk). Panes are SwiftUI views hosted in an AppKit tab view
// controller, so adding one is a view, a label and an SF Symbol.
//
// The app stays an accessory (no Dock icon) even while this window is
// open: an accessory app can activate and give a window key focus, and
// a settings panel does not warrant a Dock presence. The policy flip to
// a regular app is reserved for a real window later.

import AppKit
import ServiceManagement
import SwiftUI
import MonoplanMacKit

final class SettingsWindowController: NSWindowController {
    private let model: SettingsModel

    init(location: ProfileLocation, client: DaemonClient, signOut: @escaping (@escaping (Error?) -> Void) -> Void) {
        model = SettingsModel(location: location, client: client, signOut: signOut)
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
        tabs.addTabViewItem(pane("Account", symbol: "person.crop.circle", AccountPane(model: model)))
        tabs.addTabViewItem(pane("Advanced", symbol: "gearshape.2", AdvancedPane(model: model)))

        let window = NSWindow(contentViewController: tabs)
        window.title = "Account"
        window.styleMask = [.titled, .closable]
        window.toolbarStyle = .preference
        window.isReleasedWhenClosed = false
        window.setContentSize(NSSize(width: 460, height: 320))
        window.center()
        super.init(window: window)
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }

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
    @Published var hostLine: String = "Not running"
    /// The package version. From the host's hello once it answers
    /// (`version:mtime`, the mtime being the CLI's staleness check,
    /// not something to show); the bundle's own version until then.
    @Published var buildLine: String = SettingsModel.bundleVersion
    @Published var syncLine: String = "—"
    @Published var openAtLogin = false
    @Published var loginError: String?
    @Published var signingOut = false
    @Published var signOutError: String?
    @Published var cliLine: String = "—"
    @Published var cliError: String?
    /// nil when there is no bundled binary to link to (a `swift run`).
    let cli: CommandLineLink? = bundledBinary().map { CommandLineLink(target: $0) }

    private let location: ProfileLocation
    private let client: DaemonClient
    private let signOutAction: (@escaping (Error?) -> Void) -> Void

    init(location: ProfileLocation, client: DaemonClient, signOut: @escaping (@escaping (Error?) -> Void) -> Void) {
        self.location = location
        self.client = client
        self.signOutAction = signOut
        profilePath = location.dir.path
        refresh()
    }

    /// Confirm, then stop the host and wipe the profile. Local state
    /// only: everything is on the server, and signing in again pulls
    /// it back.
    func signOut() {
        let alert = NSAlert()
        alert.messageText = "Sign out of Monoplan?"
        alert.informativeText = "This removes the local copy of your data from this Mac. Your data stays on the server and comes back when you sign in again."
        alert.alertStyle = .warning
        alert.addButton(withTitle: "Sign Out")
        alert.addButton(withTitle: "Cancel")
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
            case .installed: cliLine = "Installed · \(cli.link)"
            case .other(let what): cliLine = "\(cli.link) → \(what)"
            case .missing: cliLine = "Not installed"
            }
        } else {
            cliLine = "No bundled binary (run the built app)"
        }
        switch client.state {
        case .connecting(let reason):
            hostLine = "Not running · \(reason)"
            syncLine = "—"
            buildLine = Self.bundleVersion
        case .connected(let sync):
            hostLine = "Running"
            if let d = client.daemon {
                buildLine = String(d.buildId.split(separator: ":", maxSplits: 1).first ?? "")
            }
            syncLine = sync.online ? "Online" : (sync.offlineNotice() ?? "Offline")
            client.send(.account) { [weak self] result in
                if case .success(.account(_, let email, _)) = result {
                    self?.email = email
                }
            }
        }
    }

    static var bundleVersion: String {
        Bundle.main.infoDictionary?["CFBundleShortVersionString"] as? String ?? "—"
    }

    /// "Signed in (you@example.com)", or just the state while the
    /// host has not answered or there is no profile.
    var statusLine: String {
        guard signedIn else { return "Not signed in" }
        if let email { return "Signed in (\(email))" }
        return "Signed in"
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
            if model.signedIn {
                LabeledContent("") {
                    Button(model.signingOut ? "Signing out…" : "Sign Out…") { model.signOut() }
                        .disabled(model.signingOut)
                }
                if let error = model.signOutError {
                    Text(error).font(.footnote).foregroundStyle(.red)
                }
            }
            Text("Sign in is done from the command line for now (`monoplan login --server …`); it works while Monoplan is running.")
                .font(.footnote)
                .foregroundStyle(.secondary)
        }
        .formStyle(.grouped)
        .frame(width: 460)
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
