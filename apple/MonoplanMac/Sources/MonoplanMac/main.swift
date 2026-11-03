// The menubar app's entry point. Top-level code rather than `@main` so the
// activation policy is set before the run loop starts: `.accessory` gives a
// status item with no Dock icon or menu bar of its own, which is what a
// tray-only process should be. Opening the window (`WebWindow.swift`)
// switches to `.regular`, with a Dock icon and the main menu, and
// closing it switches back.
//
// The tray holds no doc. It watches its own in-process host (a watch
// over the FFI, never the socket) and re-fetches `agenda --days 0
// --json` when the host says something changed, once a minute (the
// relative times move), and when the menu opens. "Open Monoplan" and
// the item rows open the window, which mirrors the host's docs
// (spec/mac-window-plan.md).

import AppKit
import MonoplanMacKit

/// The web client in the browser, for what the window does not do:
/// creating an account (the sign-in sheet's link). Signup wraps a DEK
/// the app would then have to hold, so it stays in the browser
/// (spec/mac-window-plan.md "DEK-touching flows"). The dev stack serves
/// the client on 5176, so `MONOPLAN_WEB_URL=http://localhost:5176 bun
/// run mac` points the link there.
let webURL: URL = {
    if let raw = ProcessInfo.processInfo.environment["MONOPLAN_WEB_URL"],
       let url = URL(string: raw) {
        return url
    }
    return URL(string: "https://alpha.monoplan.app")!
}()

/// `MONOPLAN_MAC_DEBUG=1` logs every menu rebuild (connection state,
/// rows, fetch error) so the tray can be checked from a terminal.
let debug = ProcessInfo.processInfo.environment["MONOPLAN_MAC_DEBUG"] == "1"

final class TrayController: NSObject, NSMenuDelegate {
    private let statusItem: NSStatusItem
    private let menu = NSMenu()
    private let client: DaemonClient
    private let location: ProfileLocation
    private let hostController: HostController
    private var settingsWindow: SettingsWindowController?
    /// The window: one per process, made on first open, mirrors the
    /// controller's running host.
    private lazy var webWindow = WebWindowController(
        hostProvider: { [weak self] in self?.hostController.runningHost },
        onVisibility: { visible in NSApp.setActivationPolicy(visible ? .regular : .accessory) })
    /// Whether the last client state was connected, to tell a host
    /// coming (back) up from a status refresh.
    private var hadHost = false
    private var lines: [MenuLine] = []
    private var starred: [MenuLine] = []
    private var starredRows: [FocusRow] = []
    private var listNames: [String: String] = Lists.names(from: [])
    private var fetchError: String?
    private var refreshQueued = false
    private var minuteTimer: Timer?

    override init() {
        statusItem = NSStatusBar.system.statusItem(withLength: NSStatusItem.squareLength)
        location = ProfileLocation.resolve()
        client = DaemonClient()
        hostController = HostController(location: location, client: client)
        super.init()

        if let button = statusItem.button {
            button.image = Self.moonIcon()
            button.toolTip = "Monoplan"
        }
        menu.delegate = self
        statusItem.menu = menu
        rebuildMenu()

        client.onState = { [weak self] state in
            guard let self else { return }
            if case .connected(let sync) = state {
                self.refresh()
                self.webWindow.pushSync(sync)
                // A host after none: the window's page booted against
                // the old one (or showed its error); reload it.
                if !self.hadHost { self.webWindow.hostAvailable() }
                self.hadHost = true
            } else {
                self.hadHost = false
                self.rebuildMenu()
            }
            self.settingsWindow?.refresh()
        }
        client.onEvent = { [weak self] event in
            switch event {
            case .changed: self?.scheduleRefresh()
            case .sync(let sync):
                self?.rebuildMenu()
                self?.webWindow.pushSync(sync)
            }
            self?.settingsWindow?.refresh()
        }
        client.start()

        minuteTimer = Timer.scheduledTimer(withTimeInterval: 60, repeats: true) { [weak self] _ in
            self?.refresh()
        }
        minuteTimer?.tolerance = 5
    }

    // MARK: Data

    /// Coalesce a burst of `Changed` events into one fetch.
    private func scheduleRefresh() {
        if refreshQueued { return }
        refreshQueued = true
        DispatchQueue.main.asyncAfter(deadline: .now() + 0.25) { [weak self] in
            self?.refreshQueued = false
            self?.refresh()
        }
    }

    private func refresh() {
        client.send(.agenda(days: 0)) { [weak self] result in
            guard let self else { return }
            switch result {
            case .success(.output(let stdout, _)):
                do {
                    self.lines = TodayMenu.lines(from: try Agenda.parse(stdout), now: Date())
                    self.fetchError = nil
                } catch {
                    self.fetchError = String(localized: "could not read agenda: \(error.localizedDescription)")
                }
            case .success(let other):
                self.fetchError = String(localized: "unexpected reply: \(String(describing: other))")
            case .failure(let error):
                if case ClientError.notConnected = error { break }
                self.fetchError = error.localizedDescription
            }
            self.rebuildMenu()
        }
        client.send(.lists) { [weak self] result in
            guard let self else { return }
            if case .success(.output(let stdout, _)) = result,
               let rows = try? Lists.parse(stdout) {
                self.listNames = Lists.names(from: rows)
                self.starred = StarredMenu.lines(from: self.starredRows, listNames: self.listNames)
                self.rebuildMenu()
            }
        }
        client.send(.focus) { [weak self] result in
            guard let self else { return }
            if case .success(.output(let stdout, _)) = result,
               let rows = try? Focus.parse(stdout) {
                self.starredRows = rows
            } else {
                // Today is the headline; a failed Starred fetch just
                // leaves the section empty rather than replacing it.
                self.starredRows = []
            }
            self.starred = StarredMenu.lines(from: self.starredRows, listNames: self.listNames)
            self.rebuildMenu()
        }
    }

    // MARK: Menu

    func menuWillOpen(_ menu: NSMenu) {
        // Relative times ("in 12m") should be right when looked at.
        refresh()
    }

    private func rebuildMenu() {
        if debug {
            NSLog("MonoplanMac: state=%@ lines=%@ starred=%@ error=%@",
                  String(describing: client.state),
                  lines.map { "\($0.title)\($0.note.map { " [\($0)]" } ?? "")" }.joined(separator: " | "),
                  starred.map { "\($0.title)\($0.note.map { " [\($0)]" } ?? "")" }.joined(separator: " | "),
                  fetchError ?? "-")
        }
        menu.removeAllItems()

        let header = NSMenuItem(title: String(localized: "Today · \(Self.headerFormatter.string(from: Date()))"), action: nil, keyEquivalent: "")
        header.isEnabled = false
        menu.addItem(header)

        switch client.state {
        case .connecting(let reason):
            addDisabled(String(localized: "Not running · \(reason)"))
        case .connected:
            let noteTab = Self.noteTabLocation(for: lines + starred)
            if let fetchError {
                addDisabled(fetchError)
            } else if lines.isEmpty {
                addDisabled(String(localized: "Nothing today"))
            } else {
                for line in lines { menu.addItem(rowItem(line, noteTab: noteTab)) }
            }
            menu.addItem(.separator())
            addDisabled(String(localized: "Starred"))
            if starred.isEmpty {
                addDisabled(String(localized: "Nothing starred"))
            } else {
                for line in starred { menu.addItem(rowItem(line, noteTab: noteTab)) }
            }
        }

        if case .connected(let sync) = client.state, let notice = sync.offlineNotice() {
            menu.addItem(.separator())
            addDisabled(notice)
        }

        menu.addItem(.separator())
        // No key equivalents and no images: a status menu is not where
        // shortcuts live, and a key equivalent anywhere (even the
        // customary ⌘, on Settings) makes AppKit reserve a trailing
        // column on every row, which shows as dead space after the
        // right-aligned notes. An image would likewise open an icon
        // column that the other rows do not use.
        if !location.isSignedIn {
            addAction(String(localized: "Sign In…"), #selector(signIn))
        }
        addAction(String(localized: "Settings…"), #selector(openSettings))
        addAction(String(localized: "Open Monoplan"), #selector(openWindow))
        menu.addItem(.separator())
        addAction(String(localized: "Quit Monoplan"), #selector(quit))
    }

    /// A plain command row. From macOS 27 AppKit supplies its own
    /// symbol for recognisable items (a gear beside "Settings…") even
    /// when none is set, so the item opts out explicitly.
    private func addAction(_ title: String, _ action: Selector) {
        let item = NSMenuItem(title: title, action: action, keyEquivalent: "")
        item.target = self
        if #available(macOS 27, *) {
            item.preferredImageVisibility = .hidden
        }
        menu.addItem(item)
    }

    private func addDisabled(_ title: String) {
        let item = NSMenuItem(title: title, action: nil, keyEquivalent: "")
        item.isEnabled = false
        menu.addItem(item)
    }

    private static let titleFont = NSFont.menuFont(ofSize: 0)
    private static let noteFont = NSFont.menuFont(ofSize: NSFont.smallSystemFontSize)
    /// Gap between the longest title and the start of the note column.
    private static let noteGap: CGFloat = 20

    private static func displayTitle(_ line: MenuLine) -> String {
        line.done ? "✓ \(line.title)" : line.title
    }

    /// One right-aligned tab stop shared by every row: the widest title
    /// plus a gap plus the widest note. Notes then end on a common right
    /// edge across Today and Starred, and the menu is exactly as wide as
    /// its widest title-plus-note, no wider.
    private static func noteTabLocation(for lines: [MenuLine]) -> CGFloat {
        let titles = lines.map { NSAttributedString(string: displayTitle($0), attributes: [.font: titleFont]).size().width }
        let notes = lines.compactMap { $0.note }.map { NSAttributedString(string: $0, attributes: [.font: noteFont]).size().width }
        return ceil((titles.max() ?? 0) + noteGap + (notes.max() ?? 0))
    }

    private func rowItem(_ line: MenuLine, noteTab: CGFloat) -> NSMenuItem {
        let item = NSMenuItem(title: "", action: #selector(openItem(_:)), keyEquivalent: "")
        item.target = self
        item.representedObject = line.id
        var titleAttributes: [NSAttributedString.Key: Any] = [.font: Self.titleFont]
        if line.done { titleAttributes[.foregroundColor] = NSColor.secondaryLabelColor }
        let title = NSMutableAttributedString(string: Self.displayTitle(line), attributes: titleAttributes)
        if let note = line.note {
            title.append(NSAttributedString(
                string: "\t\(note)",
                attributes: [
                    .foregroundColor: line.overdue ? NSColor.systemRed : NSColor.secondaryLabelColor,
                    .font: Self.noteFont,
                ]
            ))
            let paragraph = NSMutableParagraphStyle()
            paragraph.tabStops = [NSTextTab(textAlignment: .right, location: noteTab, options: [:])]
            paragraph.lineBreakMode = .byClipping
            title.addAttribute(.paragraphStyle, value: paragraph, range: NSRange(location: 0, length: title.length))
        }
        item.attributedTitle = title
        return item
    }

    /// The Monoplan moon as a menu bar glyph: a disc with a crescent
    /// shade on the bottom-right. Drawn, not shipped as a PNG, so it is
    /// resolution-independent and the shade is two numbers to tune.
    ///
    /// It is a *template* image: the menu bar ignores colour and reads
    /// only alpha, painting the shape in its own ink (black on a light
    /// bar, white on a dark one, dimmed when inactive). So the disc is
    /// full alpha and the crescent is partial alpha; the app icon's blue
    /// tile and cream fill cannot survive here, and on a light bar the
    /// crescent reads as a highlight rather than a shadow, which is the
    /// price of one-ink glyphs.
    static func moonIcon() -> NSImage {
        let side: CGFloat = 18
        let radius: CGFloat = 7.5
        /// How far the shading circle is pushed up-left, as a fraction
        /// of the radius: the crescent's thickness at the bottom-right.
        let shadeOffset: CGFloat = 0.16
        /// The crescent's ink, as a fraction of the disc's.
        let shadeAlpha: CGFloat = 0.45

        let image = NSImage(size: NSSize(width: side, height: side), flipped: false) { rect in
            let centre = NSPoint(x: rect.midX, y: rect.midY)
            let disc = NSBezierPath(ovalIn: NSRect(
                x: centre.x - radius, y: centre.y - radius, width: 2 * radius, height: 2 * radius))
            NSColor.black.setFill()
            disc.fill()

            // Inside the disc, replace (not blend) the region outside the
            // up-left-shifted circle with lighter ink: that is the crescent.
            disc.addClip()
            let shift = radius * shadeOffset
            let lit = NSBezierPath(ovalIn: NSRect(
                x: centre.x - radius - shift, y: centre.y - radius + shift,
                width: 2 * radius, height: 2 * radius))
            let crescent = NSBezierPath(rect: rect)
            crescent.append(lit)
            crescent.windingRule = .evenOdd
            crescent.addClip()
            NSColor(white: 0, alpha: shadeAlpha).setFill()
            NSGraphicsContext.current?.compositingOperation = .copy
            rect.fill()
            return true
        }
        image.isTemplate = true
        image.accessibilityDescription = "Monoplan"
        return image
    }

    private static let headerFormatter: DateFormatter = {
        let f = DateFormatter()
        f.setLocalizedDateFormatFromTemplate("EEE d MMM")
        return f
    }()

    // MARK: Actions

    /// The window over the host's docs; also the Dock icon's reopen.
    @objc func openWindow() {
        webWindow.show()
    }

    /// The View menu's Reload.
    @objc func reloadWindow(_ sender: Any?) {
        webWindow.reload(sender)
    }

    @objc func undoInWindow(_ sender: Any?) {
        webWindow.undo(sender)
    }

    @objc func redoInWindow(_ sender: Any?) {
        webWindow.redo(sender)
    }

    /// Edit › Undo / Redo follow the page's `undoState`; every other
    /// item of ours is always available.
    @objc func validateMenuItem(_ item: NSMenuItem) -> Bool {
        switch item.action {
        case #selector(undoInWindow(_:)): return webWindow.canUndo
        case #selector(redoInWindow(_:)): return webWindow.canRedo
        default: return true
        }
    }

    /// One window controller for the process; reopening brings the
    /// same window forward rather than stacking a second.
    @objc func openSettings() {
        settings().show()
    }

    @objc private func signIn() {
        settings().showSignIn()
    }

    private func settings() -> SettingsWindowController {
        if let settingsWindow { return settingsWindow }
        let w = SettingsWindowController(
            location: location, client: client,
            signIn: { [weak self] creds, done in self?.hostController.signIn(creds, completion: done) },
            signOut: { [weak self] done in self?.hostController.signOut(completion: done) },
            // The Devices pane's server access: the host's session and
            // token. `session()` blocks briefly; the pane asks off the
            // main thread.
            credential: { [weak self] in
                guard let host = self?.hostController.runningHost, let token = host.deviceToken(),
                      let session = try? host.session()
                else { return nil }
                return APICredential(serverURL: session.serverUrl, token: token, deviceId: session.deviceId)
            })
        settingsWindow = w
        return w
    }

    /// Item links are `#item_<id>` (spec/urls.md "Item URLs name only
    /// the item"), opened in the window.
    @objc private func openItem(_ sender: NSMenuItem) {
        guard let id = sender.representedObject as? String else { return }
        webWindow.show(fragment: "item_\(id)")
    }

    /// Quitting ends the host too (`applicationShouldTerminate`), which
    /// is how to stop syncing on a Mac; the CLI starts a headless
    /// daemon on its next command if it needs one.
    @objc private func quit() {
        NSApp.terminate(nil)
    }

    /// Before the process goes: the window hands its last edits to the
    /// host, then the host persists, folds and releases the profile.
    func shutdown(completion: @escaping () -> Void) {
        webWindow.handOff { [self] in
            webWindow.tearDown()
            client.stop()
            hostController.stop()
            completion()
        }
    }
}

final class AppDelegate: NSObject, NSApplicationDelegate {
    private var tray: TrayController?

    func applicationDidFinishLaunching(_ notification: Notification) {
        let tray = TrayController()
        self.tray = tray
        NSApp.mainMenu = MainMenu.build(
            settings: #selector(TrayController.openSettings),
            reload: #selector(TrayController.reloadWindow(_:)),
            undo: #selector(TrayController.undoInWindow(_:)),
            redo: #selector(TrayController.redoInWindow(_:)),
            target: tray)
    }

    /// The Dock icon (while the window is open) or a second launch.
    func applicationShouldHandleReopen(_ sender: NSApplication, hasVisibleWindows flag: Bool) -> Bool {
        tray?.openWindow()
        return false
    }

    func applicationShouldTerminate(_ sender: NSApplication) -> NSApplication.TerminateReply {
        guard let tray else { return .terminateNow }
        tray.shutdown { NSApp.reply(toApplicationShouldTerminate: true) }
        return .terminateLater
    }
}

let app = NSApplication.shared
// Before any window or menu exists, so nothing flashes the system
// appearance first (Settings › General › Appearance).
AppearancePreference.applySaved()
let delegate = AppDelegate()
app.delegate = delegate
app.setActivationPolicy(.accessory)
app.run()
