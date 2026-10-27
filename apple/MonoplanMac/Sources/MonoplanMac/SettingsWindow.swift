// The Settings window: the standard macOS preferences layout, a
// toolbar of icon-over-label panes (a sidebar is for System
// Settings-scale catalogues). One pane today, Account. Panes are
// SwiftUI views hosted in an AppKit tab view controller, so adding one
// is a view, a label and an SF Symbol.
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
    private let model: AccountModel

    init(location: ProfileLocation, client: DaemonClient, isHosting: @escaping () -> Bool) {
        model = AccountModel(location: location, client: client, isHosting: isHosting)
        // The toolbar tab style: AppKit builds the toolbar from the tab
        // items (one icon + label each) and, with the `.preference`
        // toolbar style, draws it as the full-height icon surface every
        // Settings window on the platform has. The window title tracks
        // the selected pane's label, as it does elsewhere.
        let tabs = NSTabViewController()
        tabs.tabStyle = .toolbar
        let account = NSTabViewItem(viewController: NSHostingController(rootView: AccountPane(model: model)))
        account.label = "Account"
        account.image = NSImage(systemSymbolName: "person.crop.circle", accessibilityDescription: "Account")
        tabs.addTabViewItem(account)

        let window = NSWindow(contentViewController: tabs)
        window.title = "Monoplan Settings"
        window.styleMask = [.titled, .closable]
        window.toolbarStyle = .preference
        window.isReleasedWhenClosed = false
        window.setContentSize(NSSize(width: 460, height: 320))
        window.center()
        super.init(window: window)
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }

    /// The tray forwards daemon state and sync events here; the window
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

/// What the Account pane shows. Everything here is read from the
/// profile files or the daemon's hello; nothing is editable yet, since
/// sign-in / sign-out rewrite the profile and restart the daemon and
/// stay with the CLI for now.
final class AccountModel: ObservableObject {
    @Published var serverURL: String = "—"
    @Published var signedIn = false
    /// From the daemon's account row; nil until it answers.
    @Published var email: String?
    @Published var profilePath: String
    @Published var daemonLine: String = "Not connected"
    @Published var syncLine: String = "—"
    @Published var openAtLogin = false
    @Published var loginError: String?
    @Published var cliLine: String = "—"
    @Published var cliError: String?
    /// nil when there is no bundled binary to link to (a `swift run`).
    let cli: CommandLineLink? = bundledBinary().map { CommandLineLink(target: $0) }
    private let isHosting: () -> Bool

    private let location: ProfileLocation
    private let client: DaemonClient

    init(location: ProfileLocation, client: DaemonClient, isHosting: @escaping () -> Bool) {
        self.location = location
        self.client = client
        self.isHosting = isHosting
        profilePath = location.dir.path
        refresh()
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
            daemonLine = "Not connected · \(reason)"
            syncLine = "—"
        case .connected(let sync):
            if let d = client.daemon {
                let who = isHosting() ? "this app" : (d.embedded ? "another Monoplan app" : "headless daemon")
                daemonLine = "Running · \(who) · pid \(d.pid) · build \(d.buildId)"
            } else {
                daemonLine = "Running"
            }
            syncLine = sync.online ? "Online" : (sync.offlineNotice() ?? "Offline")
            client.send(.account) { [weak self] result in
                if case .success(.account(_, let email, _)) = result {
                    self?.email = email
                }
            }
        }
    }

    /// "Signed in (you@example.com)", or just the state while the
    /// daemon has not answered or there is no profile.
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
    @ObservedObject var model: AccountModel

    var body: some View {
        Form {
            LabeledContent("Server", value: model.serverURL)
            LabeledContent("Status", value: model.statusLine)
            LabeledContent("Daemon", value: model.daemonLine)
            LabeledContent("Sync", value: model.syncLine)
            Toggle("Open at login", isOn: Binding(
                get: { model.openAtLogin },
                set: { model.setOpenAtLogin($0) }
            ))
            if let error = model.loginError {
                Text(error).font(.footnote).foregroundStyle(.red)
            }
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
            Text("Sign in and sign out are done from the command line for now (`monoplan login` / `monoplan logout`); quit Monoplan first, it holds the profile while it runs.")
                .font(.footnote)
                .foregroundStyle(.secondary)
        }
        .formStyle(.grouped)
        .frame(width: 460)
    }
}
