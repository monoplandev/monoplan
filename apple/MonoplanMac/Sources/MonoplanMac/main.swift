// The menubar app's entry point. Top-level code rather than `@main` so the
// activation policy is set before the run loop starts: `.accessory` gives a
// status item with no Dock icon or menu bar of its own, which is what a
// tray-only process should be. Opening the window (`MainWindow.swift`)
// switches to `.regular`, with a Dock icon and the main menu, and
// closing it switches back.
//
// The tray holds no doc. It follows its own in-process engine through
// the typed calls (spec/swift-bindings-plan.md; never the socket) and
// reads the agenda and Starred again when the engine says something
// changed, once a minute (the relative times move), and when the menu
// opens. "Open Monoplan" and the item rows open the window.

import AppKit
import MonoplanCore
import MonoplanMacKit
import Network

/// `MONOPLAN_MAC_DEBUG=1` logs every menu rebuild (connection state,
/// rows, fetch error) so the tray can be checked from a terminal.
let debug = ProcessInfo.processInfo.environment["MONOPLAN_MAC_DEBUG"] == "1"

final class TrayController: NSObject, NSMenuDelegate {
    private let statusItem: NSStatusItem
    private let menu = NSMenu()
    private let location: ProfileLocation
    private let engineController: EngineController
    private var settingsWindow: SettingsWindowController?
    /// The window: one per process, made on first open.
    private lazy var mainWindow = MainWindowController(
        engine: engineController,
        signedIn: { [location] in location.hasAccount },
        onSignIn: { [weak self] in self?.signIn() },
        onVisibility: { visible in NSApp.setActivationPolicy(visible ? .regular : .accessory) })
    /// Whether the last engine state was running, to tell an engine
    /// coming (back) up from a sync report.
    private var hadEngine = false
    private var lines: [MenuLine] = []
    private var starred: [MenuLine] = []
    private var fetchError: String?
    private var refreshQueued = false
    private var minuteTimer: Timer?
    /// The network as the system sees it. The engine's own liveness ping
    /// takes up to a minute to notice a dead socket after a Wi-Fi
    /// change or a wake; a path change or a wake tells it to reconnect
    /// now instead (`Request::Reconnect`).
    private var pathMonitor: NWPathMonitor?
    private var lastPath: NWPath?
    private var reconnectQueued = false

    override init() {
        statusItem = NSStatusBar.system.statusItem(withLength: NSStatusItem.squareLength)
        location = ProfileLocation.resolve()
        engineController = EngineController(location: location)
        super.init()

        if let button = statusItem.button {
            button.image = Self.moonIcon()
            button.toolTip = "Monoplan"
        }
        menu.delegate = self
        statusItem.menu = menu
        rebuildMenu()

        engineController.onState = { [weak self] state in
            guard let self else { return }
            if case .connected = state {
                // An engine after none: its rows. A sync report alone
                // only moves the offline notice.
                if self.hadEngine { self.rebuildMenu() } else { self.refresh() }
                self.hadEngine = true
            } else {
                self.hadEngine = false
                self.rebuildMenu()
            }
            self.mainWindow.engineStateChanged(state)
            self.settingsWindow?.refresh()
        }
        engineController.onChanged = { [weak self] in
            self?.scheduleRefresh()
            self?.mainWindow.docChanged()
            self?.settingsWindow?.refresh()
        }
        engineController.start()

        // Settings › General "Open window at launch". The window shows
        // the engine's state itself, so it need not wait for the start.
        if LaunchPreference.openWindow() { mainWindow.show() }

        minuteTimer = Timer.scheduledTimer(withTimeInterval: 60, repeats: true) { [weak self] _ in
            self?.refresh()
        }
        minuteTimer?.tolerance = 5

        let monitor = NWPathMonitor()
        monitor.pathUpdateHandler = { [weak self] path in
            DispatchQueue.main.async { self?.pathChanged(path) }
        }
        monitor.start(queue: DispatchQueue(label: "app.monoplan.mac.path"))
        pathMonitor = monitor
        NSWorkspace.shared.notificationCenter.addObserver(
            self, selector: #selector(didWake), name: NSWorkspace.didWakeNotification, object: nil)
    }

    // MARK: Network

    /// The first callback describes the path as it is; only a change
    /// after that is news. A change is any difference (status, the
    /// interfaces in use), since a healthy-looking socket over a gone
    /// interface is exactly the case the ping is slow to catch.
    private func pathChanged(_ path: NWPath) {
        defer { lastPath = path }
        guard let previous = lastPath, previous != path else { return }
        scheduleReconnect()
    }

    @objc private func didWake() {
        scheduleReconnect()
    }

    /// Coalesce the burst a wake produces (several path updates and the
    /// wake itself) into one reconnect. Nothing to do without an engine.
    private func scheduleReconnect() {
        if reconnectQueued { return }
        reconnectQueued = true
        DispatchQueue.main.asyncAfter(deadline: .now() + 1) { [weak self] in
            guard let self else { return }
            self.reconnectQueued = false
            self.engineController.calls?.run({ try $0.reconnect() }) { _ in }
        }
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
        guard let calls = engineController.calls else { return }
        calls.run({ engine in
            // Today is the headline; a failed Starred read just leaves
            // the section empty rather than replacing it.
            (try engine.agenda(days: 0), (try? engine.starred()) ?? [])
        }) { [weak self] result in
            guard let self else { return }
            switch result {
            case .success((let groups, let starred)):
                self.lines = TodayMenu.lines(from: groups, now: Date())
                self.starred = StarredMenu.lines(from: starred)
                self.fetchError = nil
            case .failure(EngineError.Stopped):
                // An engine that has just stopped: the state line says
                // so and the next engine's refresh fills the rows.
                break
            case .failure(let error):
                self.fetchError = engineErrorMessage(error)
            }
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
                  String(describing: engineController.state),
                  lines.map { "\($0.title)\($0.note.map { " [\($0)]" } ?? "")" }.joined(separator: " | "),
                  starred.map { "\($0.title)\($0.note.map { " [\($0)]" } ?? "")" }.joined(separator: " | "),
                  fetchError ?? "-")
        }
        menu.removeAllItems()

        let header = NSMenuItem(title: String(localized: "Today · \(Self.headerFormatter.string(from: Date()))"), action: nil, keyEquivalent: "")
        header.isEnabled = false
        menu.addItem(header)

        switch engineController.state {
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

        if case .connected(let sync) = engineController.state, let notice = sync.offlineNotice() {
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
        // A local-only workspace (no account): the way to one. The
        // sheet offers both signing in and creating an account.
        if !location.hasAccount {
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

    /// A click opens the window on the item's list (`openItem`).
    private func rowItem(_ line: MenuLine, noteTab: CGFloat) -> NSMenuItem {
        let item = NSMenuItem(title: "", action: #selector(openItem(_:)), keyEquivalent: "")
        item.target = self
        item.representedObject = RowTarget(itemId: line.id, listId: line.listId, docId: line.docId)
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

    /// The window; also the Dock icon's reopen.
    @objc func openWindow() {
        mainWindow.show()
    }

    /// Monoplan › New Workspace…: the name is asked for on the window
    /// and the engine creates and opens the doc (`Engine.createWorkspace`).
    /// Needs a running engine with an account (one workspace per
    /// accountless install, `spec/auth.md`), which `validateMenuItem`
    /// checks.
    @objc func newWorkspace(_ sender: Any?) {
        mainWindow.show()
        mainWindow.promptNewWorkspace()
    }

    /// New Workspace… needs an engine that is serving a signed-in
    /// profile; every other item of ours is always available.
    @objc func validateMenuItem(_ item: NSMenuItem) -> Bool {
        switch item.action {
        case #selector(newWorkspace(_:)):
            guard location.hasAccount, case .connected = engineController.state else { return false }
            return true
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
            location: location, engine: engineController,
            signIn: { [weak self] creds, done in self?.engineController.signIn(creds, completion: done) },
            signUp: { [weak self] creds, done in self?.engineController.signUp(creds, completion: done) },
            signOut: { [weak self] done in self?.engineController.signOut(completion: done) },
            // The Devices pane's server access: who the engine is signed
            // in as and its device token. `account()` blocks briefly;
            // the pane asks off the main thread.
            credential: { [weak self] in
                guard let engine = self?.engineController.runningEngine, let token = engine.deviceToken(),
                      let account = try? engine.account()
                else { return nil }
                return APICredential(serverURL: account.serverUrl, token: token, deviceId: account.deviceId)
            })
        settingsWindow = w
        return w
    }

    /// A row of the menu: the window on the item's list, in its
    /// workspace, with the item's panel open.
    @objc private func openItem(_ sender: NSMenuItem) {
        guard let target = sender.representedObject as? RowTarget else { return }
        mainWindow.show(list: target.listId, workspace: target.docId, item: target.itemId)
    }

    /// Quitting ends the engine too (`applicationShouldTerminate`), which
    /// is how to stop syncing on a Mac; the CLI starts a headless
    /// daemon on its next command if it needs one.
    @objc private func quit() {
        NSApp.terminate(nil)
    }

    /// Before the process goes: the engine persists, folds and releases
    /// the profile. Blocks.
    func shutdown() {
        engineController.stop()
    }
}

/// What a menu row opens: `NSMenuItem.representedObject` wants an object.
private final class RowTarget: NSObject {
    let itemId: String
    let listId: String
    let docId: String?

    init(itemId: String, listId: String, docId: String?) {
        self.itemId = itemId
        self.listId = listId
        self.docId = docId
    }
}

final class AppDelegate: NSObject, NSApplicationDelegate {
    private var tray: TrayController?

    func applicationDidFinishLaunching(_ notification: Notification) {
        let tray = TrayController()
        self.tray = tray
        NSApp.mainMenu = MainMenu.build(
            newWorkspace: #selector(TrayController.newWorkspace(_:)),
            settings: #selector(TrayController.openSettings),
            target: tray)
    }

    /// The Dock icon (while the window is open) or a second launch.
    func applicationShouldHandleReopen(_ sender: NSApplication, hasVisibleWindows flag: Bool) -> Bool {
        tray?.openWindow()
        return false
    }

    func applicationShouldTerminate(_ sender: NSApplication) -> NSApplication.TerminateReply {
        tray?.shutdown()
        return .terminateNow
    }
}

let app = NSApplication.shared
// Before any window or menu exists, so nothing flashes the system
// appearance first (Settings › General › Appearance).
AppearancePreference.applySaved()
GraySelection.install()
let delegate = AppDelegate()
app.delegate = delegate
app.setActivationPolicy(.accessory)
app.run()
