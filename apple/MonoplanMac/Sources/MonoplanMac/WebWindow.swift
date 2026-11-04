// The app's window (spec/mac-window-plan.md, phase 4): a WKWebView
// running the web client (`js/web/mirror.html`, built into the bundle)
// over a mirror of the in-process host's docs. Three pieces:
//
// - `WebWindowController`: the NSWindow and the webview, its lifecycle
//   (open from the tray, hand everything off to the host before it
//   closes or the app quits), activation policy via `onVisibility`,
//   and the native controls over the page: two round Liquid Glass
//   bubbles floating in the top-right corner, search (the page's Find
//   palette, by `HostInbound.find`) and account (`AccountActions`: a
//   menu of the signed-in email, the Settings window's Account pane
//   and sign out / sign in). The page's own account menu is not
//   rendered in the window (`Session.mirror`), so this is where a
//   user looks.
// - `MirrorHostBridge`: the `monoplan` message handler the page posts
//   to (`session`, `subscribe`, `unsubscribe`, `import`, `window`,
//   `undoState`) and the calls
//   back into `window.__monoplanHost` (`update`, `stopped`, `sync`,
//   `handOff`, `settings`, `undo`, `redo`, `find`). One `Host.subscribe` per open doc; a `MirrorSink` per
//   subscription hops to the main queue and evaluates the update into
//   the page, never calling `Host.import` from inside `update` (the
//   loop is busy delivering). Requests that block in Rust (`subscribe`
//   exports a snapshot, `import` waits for durability) run on a serial
//   work queue, in order, off the main thread.
// - `AppSchemeHandler`: `monoplan://app/*` serves the Vite build with
//   the right MIME types (wasm streams, `localStorage` has a stable
//   origin, no `file://` rules), and forwards `/api/*` to the host's
//   server with `Authorization: Bearer <device token>`, so `api.ts`'s
//   same-origin fetches work unchanged and the token never enters the
//   page. The webview holds no DEK, no storage of its own, no socket.
//
// `MONOPLAN_MIRROR_URL=http://localhost:5176/mirror.html` loads the
// Vite dev server instead (HMR for the UI); the doc still mirrors the
// host through the handler, but `/api` then goes through Vite's proxy
// without the token, so the HTTP surface (devices, members) is
// unauthenticated there.

import AppKit
import MonoplanCore
import MonoplanMacKit
import UniformTypeIdentifiers
import WebKit

/// What the window's account button can do; all of it is the tray's
/// (the Settings window and the host controller), handed in as
/// closures so the window stays ignorant of both.
struct AccountActions {
    /// Whether the profile is signed in (`ProfileLocation.isSignedIn`):
    /// decides between Sign Out… and Sign In… and whether the cached
    /// email is still current.
    let isSignedIn: () -> Bool
    /// Settings on the Account pane.
    let openAccount: () -> Void
    /// Settings on the Account pane with the sign-in sheet up.
    let signIn: () -> Void
    /// Confirm and sign out (the Account pane's own flow).
    let signOut: () -> Void
}

final class WebWindowController: NSWindowController, NSWindowDelegate, WKNavigationDelegate, WKUIDelegate,
                                 WKDownloadDelegate, NSMenuDelegate {
    /// Where the host comes from at request time: the running one, or
    /// nil while there is none (the page then shows its boot error and
    /// is reloaded when a host is back, see `hostAvailable`).
    private let hostProvider: () -> MonoplanCore.Host?
    /// The tray switches the activation policy on this: a regular app
    /// with a Dock icon and menu bar while the window is up, a bare
    /// status item otherwise.
    private let onVisibility: (Bool) -> Void
    private let account: AccountActions
    private let bridge: MirrorHostBridge
    /// The email the page's session came back with, for the account
    /// menu's header; nil until a page has booted against a host.
    private var accountEmail: String?
    /// The account bubble's menu, filled in on open (`menuNeedsUpdate`).
    private let accountMenu = NSMenu()
    /// The host's latest sync status: the dot on the account bubble
    /// and the line under the email in its menu.
    private var syncStatus: SyncStatus?
    /// The New Workspace sheet while it is up (`promptNewWorkspace`).
    private var newWorkspacePrompt: NewWorkspacePrompt?
    /// The dot on the account bubble's shoulder: amber while syncing,
    /// red while offline, hidden when synced so the quiet state is quiet.
    private var syncDot: NSView?
    private let schemeHandler: AppSchemeHandler
    private var webView: WKWebView?
    /// The Liquid Glass under the page's sidebar (macOS 26 and later,
    /// nil before), retinted as the window gains and loses key.
    private var sidebarGlass: NSView?
    /// The glass's width: the page's sidebar column at the current
    /// zoom (CSS pixels scale with `pageZoom`), plus the bleed.
    private var sidebarGlassWidth: NSLayoutConstraint?
    /// Re-styles the glass when the app's appearance changes (Settings
    /// › General › Appearance, or the system).
    private var appearanceObservation: NSKeyValueObservation?
    /// Set once a hand-off has run for the pending close; the second
    /// `windowShouldClose` lets the window go.
    private var handedOff = false
    /// The dev-server override, if any.
    private let devURL: URL?
    /// The device settings the page was last given (`WindowSettings`):
    /// as a document-start user script, so a load or reload starts
    /// with them, and pushed live when `UserDefaults` changes.
    private var settings = WindowSettings.load()
    private var settingsObservers: [NSObjectProtocol] = []
    /// What the page last said about Edit › Undo / Redo (`undoState`);
    /// the main menu validates against it. Nothing until a page says.
    private(set) var canUndo = false
    private(set) var canRedo = false

    init(hostProvider: @escaping () -> MonoplanCore.Host?, onVisibility: @escaping (Bool) -> Void,
         account: AccountActions) {
        self.hostProvider = hostProvider
        self.onVisibility = onVisibility
        self.account = account
        self.bridge = MirrorHostBridge(hostProvider: hostProvider)
        self.schemeHandler = AppSchemeHandler(
            webRoot: Bundle.main.resourceURL?.appendingPathComponent("web", isDirectory: true),
            credential: { [bridge] in bridge.apiCredential })
        self.devURL = ProcessInfo.processInfo.environment["MONOPLAN_MIRROR_URL"].flatMap(URL.init(string:))
        super.init(window: nil)
        bridge.onUndoState = { [weak self] canUndo, canRedo in
            self?.canUndo = canUndo
            self?.canRedo = canRedo
        }
        bridge.onSession = { [weak self] email in self?.accountEmail = email }
        accountMenu.delegate = self
        // Any defaults write (the window's own frame autosave included)
        // posts the first; the locale one is the 24-Hour Time switch
        // (a language change relaunches the app). Only a change to the
        // settings themselves goes on.
        for name in [UserDefaults.didChangeNotification, NSLocale.currentLocaleDidChangeNotification] {
            settingsObservers.append(NotificationCenter.default.addObserver(
                forName: name, object: nil, queue: .main
            ) { [weak self] _ in self?.settingsChanged() })
        }
    }

    deinit {
        for o in settingsObservers { NotificationCenter.default.removeObserver(o) }
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }

    var isOpen: Bool { window != nil }

    private func settingsChanged() {
        let now = WindowSettings.load()
        guard now != settings else { return }
        settings = now
        guard let webView else { return }
        installSettingsScript(into: webView.configuration.userContentController)
        if webView.url != nil { webView.evaluateJavaScript(HostInbound.settings(now).script) }
    }

    /// The document-start script that sets `window.__monoplanSettings`,
    /// replacing the previous one (the only user script there is).
    private func installSettingsScript(into controller: WKUserContentController) {
        controller.removeAllUserScripts()
        controller.addUserScript(WKUserScript(
            source: settings.userScript, injectionTime: .atDocumentStart, forMainFrameOnly: true))
    }

    /// Open (or bring forward) the window, on `fragment` (`item_<id>`
    /// and the other `spec/urls.md` fragments) when given.
    func show(fragment: String? = nil) {
        if window == nil { makeWindow() }
        if let webView, webView.url != nil, let fragment {
            // Already loaded: a fragment navigation, as a link would do.
            webView.evaluateJavaScript("location.hash = \(jsString(fragment))")
        } else if let webView, webView.url == nil {
            webView.load(URLRequest(url: startURL(fragment: fragment)))
        }
        onVisibility(true)
        NSApp.activate(ignoringOtherApps: true)
        showWindow(nil)
        window?.makeKeyAndOrderFront(nil)
    }

    /// The host's sync status: the account bubble's dot and menu line
    /// here, and the page's copy, kept and replayed on load, pushed
    /// live afterwards.
    func pushSync(_ status: SyncStatus) {
        syncStatus = status
        applySyncDot()
        bridge.pushSync(status, into: webView)
    }

    private func applySyncDot() {
        guard let dot = syncDot else { return }
        switch syncStatus?.level {
        case .syncing?:
            dot.layer?.backgroundColor = NSColor.systemOrange.cgColor
            dot.isHidden = false
        case .offline?:
            dot.layer?.backgroundColor = NSColor.systemRed.cgColor
            dot.isHidden = false
        case .synced?, nil:
            dot.isHidden = true
        }
    }

    /// A host is serving again after none was: the page booted against
    /// the old one (or failed to), so reload it against this one.
    func hostAvailable() {
        guard let webView, webView.url != nil else { return }
        webView.reload()
    }

    /// Reload the page (the View menu).
    @objc func reload(_ sender: Any?) {
        webView?.reload()
    }

    /// The New Workspace sheet (Monoplan › New Workspace…): a name,
    /// Create / Cancel. `create` is handed the trimmed name and a
    /// completion; on success the page opens the new doc live
    /// (`HostInbound.docAdded`) and goes to its Inbox, on failure the
    /// sheet is followed by the error. One prompt at a time.
    func promptNewWorkspace(create: @escaping (String, @escaping (Result<String, Error>) -> Void) -> Void) {
        guard let window, newWorkspacePrompt == nil else { return }
        let prompt = NewWorkspacePrompt()
        newWorkspacePrompt = prompt
        prompt.run(on: window) { [weak self] name in
            guard let self else { return }
            guard let name else {
                self.newWorkspacePrompt = nil
                return
            }
            create(name) { result in
                DispatchQueue.main.async {
                    self.newWorkspacePrompt = nil
                    switch result {
                    case .success(let docId):
                        self.webView?.evaluateJavaScript(HostInbound.docAdded(docId: docId).script)
                    case .failure(let error):
                        guard let window = self.window else { return }
                        let alert = NSAlert()
                        alert.alertStyle = .warning
                        alert.messageText = String(localized: "Could not create the workspace")
                        alert.informativeText = error.localizedDescription
                        alert.beginSheetModal(for: window)
                    }
                }
            }
        }
    }

    /// Edit › Undo / Redo: the page's, by menu click or by the key
    /// equivalent the page let through (it takes ⌘Z itself outside text
    /// fields, so this is mostly clicks and typing).
    @objc func undo(_ sender: Any?) {
        webView?.evaluateJavaScript(HostInbound.undo.script)
    }

    @objc func redo(_ sender: Any?) {
        webView?.evaluateJavaScript(HostInbound.redo.script)
    }

    // MARK: Zoom

    /// View › Zoom In / Zoom Out / Actual Size (`PageZoom`): the saved
    /// level, applied to the webview as it is made and on each step.
    private var zoom = PageZoom.load()
    var canZoomIn: Bool { PageZoom.stepIn(from: zoom) != zoom }
    var canZoomOut: Bool { PageZoom.stepOut(from: zoom) != zoom }
    var canResetZoom: Bool { zoom != 1 }

    @objc func zoomIn(_ sender: Any?) { setZoom(PageZoom.stepIn(from: zoom)) }
    @objc func zoomOut(_ sender: Any?) { setZoom(PageZoom.stepOut(from: zoom)) }
    @objc func actualSize(_ sender: Any?) { setZoom(1) }

    /// The page reflows a frame or so after `pageZoom` changes, while
    /// the constraint takes effect at once. Glass that is too wide for
    /// a moment hides under the opaque main column; glass that is too
    /// narrow shows the bare material past the sidebar's edge. So the
    /// glass grows before the page and shrinks after it: the shrink
    /// waits for the page to paint at the new size (two animation
    /// frames in), and is dropped if the zoom moved again meanwhile.
    private func setZoom(_ level: Double) {
        let previous = zoom
        zoom = level
        PageZoom.save(level)
        guard let webView else { return }
        webView.pageZoom = level
        if level >= previous {
            sidebarGlassWidth?.constant = Self.glassWidth(at: level)
            return
        }
        webView.callAsyncJavaScript(
            "await new Promise(r => requestAnimationFrame(() => requestAnimationFrame(r)))",
            in: nil, in: .page
        ) { [weak self] _ in
            guard let self, self.zoom == level else { return }
            self.sidebarGlassWidth?.constant = Self.glassWidth(at: level)
        }
    }

    private static func glassWidth(at zoom: Double) -> CGFloat {
        sidebarWidth * CGFloat(zoom) + glassBleed
    }

    /// Hand everything the page committed to the host, then `completion`
    /// (on the main queue). What quit waits for; a no-op without a page.
    func handOff(completion: @escaping () -> Void) {
        guard let webView, webView.url != nil else {
            completion()
            return
        }
        bridge.handOff(in: webView, completion: completion)
    }

    /// Drop the page and its subscriptions now (quit).
    func tearDown() {
        bridge.resetPage()
        webView?.navigationDelegate = nil
        webView?.uiDelegate = nil
        webView?.stopLoading()
        webView = nil
        window?.delegate = nil
        window?.orderOut(nil)
        window = nil
    }

    // MARK: Window

    private func makeWindow() {
        let config = WKWebViewConfiguration()
        config.setURLSchemeHandler(schemeHandler, forURLScheme: AppScheme.scheme)
        config.userContentController.addScriptMessageHandler(bridge, contentWorld: .page, name: MirrorHostBridge.handlerName)
        installSettingsScript(into: config.userContentController)
        config.websiteDataStore = .default()
        if debug {
            // Inspect Element in the context menu.
            config.preferences.setValue(true, forKey: "developerExtrasEnabled")
        }
        let webView = WKWebView(frame: .zero, configuration: config)
        webView.navigationDelegate = self
        webView.uiDelegate = self
        webView.allowsBackForwardNavigationGestures = false
        // No canvas of its own: the page paints nothing at its root and
        // its sidebar is transparent in the app (`[data-shell="mac"]
        // .nav`), so the window's material shows through them. The
        // key is WebKit's long-standing KVC switch; there is no public
        // one on macOS (`underPageBackgroundColor` only recolours the
        // canvas).
        webView.setValue(false, forKey: "drawsBackground")
        if #available(macOS 13.3, *) {
            webView.isInspectable = debug
        }
        webView.pageZoom = zoom
        self.webView = webView

        // No title bar of its own: the page runs edge to edge and the
        // traffic lights sit over the top of its sidebar, which pads
        // itself below them (`.nav-titlebar`) and forwards drags and
        // double-clicks as `window` requests, since the webview takes
        // every mouse event itself.
        let window = NSWindow(
            contentRect: NSRect(x: 0, y: 0, width: 1100, height: 720),
            styleMask: [.titled, .closable, .miniaturizable, .resizable, .fullSizeContentView],
            backing: .buffered, defer: false)
        window.title = "Monoplan"
        window.titleVisibility = .hidden
        window.titlebarAppearsTransparent = true
        // An empty toolbar in the compact unified style is the public
        // way to a taller title bar: the traffic lights move from 9px
        // to 13px below the top edge (and 3px in), and hit testing still
        // reaches the webview under the whole bar, so nothing is dead.
        let toolbar = NSToolbar(identifier: "MonoplanWindowToolbar")
        toolbar.allowsUserCustomization = false
        window.toolbar = toolbar
        window.toolbarStyle = .unifiedCompact
        window.titlebarSeparatorStyle = .none
        window.minSize = NSSize(width: 520, height: 360)
        window.tabbingMode = .disallowed
        window.isReleasedWhenClosed = false
        let (content, glass, glassWidth) = Self.content(over: webView, bubbles: makeBubbles())
        window.contentView = content
        sidebarGlass = glass
        sidebarGlassWidth = glassWidth
        glassWidth?.constant = Self.glassWidth(at: zoom)
        window.delegate = self
        applyGlassTint()
        appearanceObservation = NSApp.observe(\.effectiveAppearance) { [weak self] _, _ in
            self?.applyGlassTint()
        }
        // Remembered across launches; centred the first time.
        if !window.setFrameUsingName("MonoplanWindow") { window.center() }
        window.setFrameAutosaveName("MonoplanWindow")
        self.window = window
    }

    private func startURL(fragment: String?) -> URL {
        guard let devURL else { return AppScheme.entryURL(fragment: fragment) }
        guard let fragment, var c = URLComponents(url: devURL, resolvingAgainstBaseURL: false) else { return devURL }
        c.fragment = fragment
        return c.url ?? devURL
    }

    /// Closing hands off first: the page's last flush (a notes burst on
    /// its idle timer, the outbox) reaches the host before the
    /// subscriptions go. The first pass refuses and starts the hand-off;
    /// its completion closes for real.
    func windowShouldClose(_ sender: NSWindow) -> Bool {
        if handedOff { return true }
        handOff { [weak self] in
            guard let self, let window = self.window else { return }
            self.handedOff = true
            window.close()
        }
        return false
    }

    // MARK: Full screen

    /// The empty toolbar (the taller title bar) would otherwise stay
    /// pinned across the top of the screen as a blank strip: hide it
    /// with the menu bar, so both slide down together on a hover at the
    /// top edge.
    func window(_ window: NSWindow, willUseFullScreenPresentationOptions proposedOptions: NSApplication.PresentationOptions)
        -> NSApplication.PresentationOptions {
        proposedOptions.union(.autoHideToolbar)
    }

    /// In full screen the revealed title bar has no traffic lights or
    /// page under it to speak for the window, so show the title there;
    /// the page gives up its title-bar padding meanwhile.
    func windowWillEnterFullScreen(_ notification: Notification) {
        window?.titleVisibility = .visible
        bridge.pushFullScreen(true, into: webView)
    }

    func windowWillExitFullScreen(_ notification: Notification) {
        window?.titleVisibility = .hidden
        bridge.pushFullScreen(false, into: webView)
    }

    func windowWillClose(_ notification: Notification) {
        handedOff = false
        tearDown()
        onVisibility(false)
    }

    // MARK: The bubbles

    /// A bubble's diameter, the gap between the two, and where the pair
    /// sits: level with the traffic lights (their centre is 19px down
    /// in the 38px bar) and a little in from the right edge, over
    /// whatever the page draws there.
    private static let bubbleSize: CGFloat = 28
    private static let bubbleGap: CGFloat = 6
    private static let bubbleTop: CGFloat = 5
    private static let bubbleTrailing: CGFloat = 10
    /// How close two glass bubbles have to be before the container
    /// melts them together (`NSGlassEffectContainerView.spacing`): more
    /// than the gap, so the pair reads as one piece of glass pinched in
    /// the middle rather than two discs.
    private static let bubbleMerge: CGFloat = 10
    /// The sync dot's diameter, sitting on the account bubble's
    /// top-right shoulder, a hair outside the disc.
    private static let dotSize: CGFloat = 8

    /// The webview edge to edge with the bubbles floating over its
    /// top-right corner. They are added last so they draw (and
    /// hit-test) above the page.
    /// The page's sidebar column, which the glass below it must match:
    /// `.app`'s first grid column in `styles.css`.
    private static let sidebarWidth: CGFloat = 260
    /// How far the glass runs past the window's top, bottom and
    /// leading edges, so its rim highlight (the bright hairline glass
    /// draws around its shape) falls outside the window on those
    /// sides; the trailing rim sits under the nav's border.
    private static let glassBleed: CGFloat = 24
    /// The glass's tint by appearance and key state: none in light,
    /// where the frosting alone matches Finder, and in dark the page's
    /// dark ground (`--bg`, `#1a1a1d`) at partial opacity, since
    /// untinted glass over a light desktop stays too light for a dark
    /// sidebar. Darker again while the window is not key: the material
    /// under the glass goes flat and lighter in its inactive rendering,
    /// and the heavier tint cancels that. The alphas are the knobs.
    private static let glassTint = tint(darkAlpha: 0.7)
    private static let inactiveGlassTint = tint(darkAlpha: 0.8)

    private static func tint(darkAlpha: CGFloat) -> NSColor {
        NSColor(name: nil) { appearance in
            appearance.bestMatch(from: [.aqua, .darkAqua]) == .darkAqua
                ? NSColor(srgbRed: 0x1a / 255, green: 0x1a / 255, blue: 0x1d / 255, alpha: darkAlpha)
                : .clear
        }
    }

    /// Dark mode also takes the `.clear` glass style: the regular one
    /// frosts bright (a mid grey over a light wallpaper even at 0.45
    /// tint), while clear lets the tint set the darkness and the
    /// desktop come through as colour. Light keeps the regular frost.
    private func applyGlassTint() {
        guard #available(macOS 26, *), let glass = sidebarGlass as? NSGlassEffectView else { return }
        let dark = glass.effectiveAppearance.bestMatch(from: [.aqua, .darkAqua]) == .darkAqua
        glass.style = dark ? .clear : .regular
        glass.tintColor = window?.isKeyWindow == true ? Self.glassTint : Self.inactiveGlassTint
    }

    func windowDidBecomeKey(_ notification: Notification) {
        applyGlassTint()
    }

    func windowDidResignKey(_ notification: Notification) {
        applyGlassTint()
    }

    /// The window's ground, under the transparent webview: a vibrant
    /// material blurring what is behind the window, and on macOS 26
    /// and later a sheet of Liquid Glass over it the width of the
    /// page's sidebar, which is what Finder's sidebar is since then
    /// (an `NSVisualEffectView` alone, in any material, renders the
    /// older blur and lets far more of the desktop through; tried
    /// `.sidebar`, `.menu` and `.underWindowBackground` on
    /// 2026-11-04). The page's nav paints nothing, so the glass
    /// shows through it; its main column covers the rest. Returns the
    /// content view, the glass, for retinting, and its width
    /// constraint, which follows the page zoom.
    private static func content(over webView: WKWebView, bubbles: NSView)
        -> (NSView, glass: NSView?, glassWidth: NSLayoutConstraint?) {
        let container = NSVisualEffectView()
        container.material = .underWindowBackground
        container.blendingMode = .behindWindow
        container.state = .followsWindowActiveState
        var glass: NSView?
        var widthConstraint: NSLayoutConstraint?
        if #available(macOS 26, *) {
            let sheet = NSGlassEffectView()
            sheet.cornerRadius = 0
            glass = sheet
        }
        for v in [glass, webView, bubbles].compactMap({ $0 }) {
            v.translatesAutoresizingMaskIntoConstraints = false
            container.addSubview(v)
        }
        if let glass {
            let width = glass.widthAnchor.constraint(equalToConstant: glassWidth(at: 1))
            widthConstraint = width
            NSLayoutConstraint.activate([
                glass.leadingAnchor.constraint(equalTo: container.leadingAnchor, constant: -glassBleed),
                glass.topAnchor.constraint(equalTo: container.topAnchor, constant: -glassBleed),
                glass.bottomAnchor.constraint(equalTo: container.bottomAnchor, constant: glassBleed),
                width,
            ])
        }
        NSLayoutConstraint.activate([
            webView.leadingAnchor.constraint(equalTo: container.leadingAnchor),
            webView.trailingAnchor.constraint(equalTo: container.trailingAnchor),
            webView.topAnchor.constraint(equalTo: container.topAnchor),
            webView.bottomAnchor.constraint(equalTo: container.bottomAnchor),
            bubbles.heightAnchor.constraint(equalToConstant: bubbleSize),
            bubbles.topAnchor.constraint(equalTo: container.topAnchor, constant: bubbleTop),
            bubbles.trailingAnchor.constraint(equalTo: container.trailingAnchor, constant: -bubbleTrailing),
        ])
        return (container, glass, widthConstraint)
    }

    /// Search then account, left to right. On macOS 26 and later each
    /// is a round `NSGlassEffectView` (Liquid Glass, refracting the page
    /// under it; the press response is macOS 27's `effectIsInteractive`)
    /// and the pair sits in an `NSGlassEffectContainerView`, which is
    /// what melts neighbouring glass together. Before 26 they are plain
    /// circular bezels in a row.
    private func makeBubbles() -> NSView {
        let search = bubbleButton(
            symbol: "magnifyingglass", label: String(localized: "Find"), action: #selector(find(_:)))
        let account = bubbleButton(
            symbol: "person.crop.circle", label: String(localized: "Account"), action: #selector(showAccountMenu(_:)))
        let row = NSView()
        let items: [NSView]
        if #available(macOS 26, *) {
            items = [search, account].map { button in
                button.isBordered = false
                let glass = NSGlassEffectView()
                glass.cornerRadius = Self.bubbleSize / 2
                glass.contentView = button
                if #available(macOS 27, *) { glass.effectIsInteractive = true }
                return glass
            }
        } else {
            for b in [search, account] { b.bezelStyle = .circular }
            items = [search, account]
        }
        var constraints: [NSLayoutConstraint] = []
        var leading = row.leadingAnchor
        for (i, v) in items.enumerated() {
            v.translatesAutoresizingMaskIntoConstraints = false
            row.addSubview(v)
            constraints += [
                v.widthAnchor.constraint(equalToConstant: Self.bubbleSize),
                v.topAnchor.constraint(equalTo: row.topAnchor),
                v.bottomAnchor.constraint(equalTo: row.bottomAnchor),
                v.leadingAnchor.constraint(equalTo: leading, constant: i == 0 ? 0 : Self.bubbleGap),
            ]
            leading = v.trailingAnchor
        }
        constraints.append(row.trailingAnchor.constraint(equalTo: leading))
        NSLayoutConstraint.activate(constraints)
        // The sync dot, over the account bubble (added last, so it
        // paints above the glass). A plain view: the container only
        // melts glass, so it leaves this alone.
        let dot = NSView()
        dot.wantsLayer = true
        dot.layer?.cornerRadius = Self.dotSize / 2
        dot.isHidden = true
        dot.translatesAutoresizingMaskIntoConstraints = false
        dot.setAccessibilityElement(false)
        row.addSubview(dot)
        if let account = items.last {
            NSLayoutConstraint.activate([
                dot.widthAnchor.constraint(equalToConstant: Self.dotSize),
                dot.heightAnchor.constraint(equalToConstant: Self.dotSize),
                dot.trailingAnchor.constraint(equalTo: account.trailingAnchor, constant: 1),
                dot.topAnchor.constraint(equalTo: account.topAnchor, constant: -1),
            ])
        }
        syncDot = dot
        applySyncDot()
        if #available(macOS 26, *) {
            let group = NSGlassEffectContainerView()
            group.spacing = Self.bubbleMerge
            group.contentView = row
            return group
        }
        return row
    }

    private func bubbleButton(symbol: String, label: String, action: Selector) -> NSButton {
        let image = NSImage(systemSymbolName: symbol, accessibilityDescription: label)?
            .withSymbolConfiguration(.init(pointSize: 15, weight: .regular))
        let button = NSButton(image: image ?? NSImage(), target: self, action: action)
        button.imagePosition = .imageOnly
        button.imageScaling = .scaleNone
        button.toolTip = label
        button.setAccessibilityLabel(label)
        button.contentTintColor = .labelColor
        return button
    }

    /// The search bubble: the page's Find palette.
    @objc private func find(_ sender: Any?) {
        webView?.evaluateJavaScript(HostInbound.find.script)
    }

    /// The menu hangs off the bubble's bottom-left, as a pull-down's
    /// would.
    @objc private func showAccountMenu(_ sender: NSButton) {
        accountMenu.popUp(positioning: nil, at: NSPoint(x: 0, y: -4), in: sender)
    }

    /// Built on every open: who is signed in, the host's sync state
    /// (built here so the relative time is fresh), the Account pane,
    /// and sign out or sign in. The email is the one the page booted
    /// with, dropped once the profile is signed out.
    func menuNeedsUpdate(_ menu: NSMenu) {
        guard menu === accountMenu else { return }
        menu.removeAllItems()
        let signedIn = account.isSignedIn()
        let header = NSMenuItem(
            title: signedIn ? (accountEmail ?? String(localized: "Signed in")) : String(localized: "Not signed in"),
            action: nil, keyEquivalent: "")
        header.isEnabled = false
        menu.addItem(header)
        if signedIn, let syncStatus {
            let sync = NSMenuItem(title: syncStatus.line(), action: nil, keyEquivalent: "")
            sync.isEnabled = false
            menu.addItem(sync)
        }
        menu.addItem(.separator())
        func add(_ title: String, _ action: Selector) {
            let item = NSMenuItem(title: title, action: action, keyEquivalent: "")
            item.target = self
            if #available(macOS 27, *) { item.preferredImageVisibility = .hidden }
            menu.addItem(item)
        }
        add(String(localized: "Account…"), #selector(openAccountPane))
        menu.addItem(.separator())
        if signedIn {
            add(String(localized: "Sign Out…"), #selector(signOut))
        } else {
            add(String(localized: "Sign In…"), #selector(signIn))
        }
    }

    @objc private func openAccountPane() { account.openAccount() }
    @objc private func signIn() { account.signIn() }
    @objc private func signOut() { account.signOut() }

    // MARK: Navigation

    /// The page itself is one URL; a new page load drops the previous
    /// page's subscriptions (its sinks would otherwise evaluate into a
    /// page that no longer has the global).
    func webView(_ webView: WKWebView, didStartProvisionalNavigation navigation: WKNavigation!) {
        bridge.resetPage()
    }

    func webView(_ webView: WKWebView, didFinish navigation: WKNavigation!) {
        bridge.pageLoaded(into: webView)
        if debug {
            webView.evaluateJavaScript("JSON.stringify({secure: window.isSecureContext, uuid: typeof crypto.randomUUID, cookie: typeof document.cookie})") { v, e in
                NSLog("MonoplanMac: window page context %@ %@", String(describing: v), String(describing: e))
            }
        }
    }

    /// Everything that is not the app's own page opens in the default
    /// browser: the website link in the menu, a link in a note. A
    /// download the page asks for (an anchor with `download`, which is
    /// how "Export JSON" saves its blob) becomes a `WKDownload` and goes
    /// through the save panel below; without this WebKit would navigate
    /// the window to the raw blob instead.
    func webView(_ webView: WKWebView, decidePolicyFor navigationAction: WKNavigationAction,
                 decisionHandler: @escaping (WKNavigationActionPolicy) -> Void) {
        guard let url = navigationAction.request.url else {
            decisionHandler(.cancel)
            return
        }
        if navigationAction.shouldPerformDownload {
            decisionHandler(.download)
            return
        }
        if isOwnPage(url) {
            decisionHandler(.allow)
            return
        }
        if navigationAction.targetFrame == nil || navigationAction.navigationType == .linkActivated {
            NSWorkspace.shared.open(url)
        }
        decisionHandler(.cancel)
    }

    /// `target="_blank"`: no new webview, the browser.
    func webView(_ webView: WKWebView, createWebViewWith configuration: WKWebViewConfiguration,
                 for navigationAction: WKNavigationAction, windowFeatures: WKWindowFeatures) -> WKWebView? {
        if let url = navigationAction.request.url, !isOwnPage(url) {
            NSWorkspace.shared.open(url)
        }
        return nil
    }

    // MARK: Downloads the page starts

    func webView(_ webView: WKWebView, navigationAction: WKNavigationAction, didBecome download: WKDownload) {
        download.delegate = self
    }

    /// Where a download lands: a save panel seeded with the page's
    /// filename, as a sheet on the window. Cancel drops the download.
    /// WebKit refuses to write over an existing file, and the panel has
    /// already asked the user about replacing one, so the stale file is
    /// removed first.
    func download(_ download: WKDownload, decideDestinationUsing response: URLResponse,
                  suggestedFilename: String, completionHandler: @escaping (URL?) -> Void) {
        let panel = NSSavePanel()
        panel.nameFieldStringValue = suggestedFilename
        panel.canCreateDirectories = true
        let finish: (NSApplication.ModalResponse) -> Void = { result in
            guard result == .OK, let url = panel.url else {
                completionHandler(nil)
                return
            }
            try? FileManager.default.removeItem(at: url)
            completionHandler(url)
        }
        guard let window else {
            finish(panel.runModal())
            return
        }
        panel.beginSheetModal(for: window, completionHandler: finish)
    }

    func download(_ download: WKDownload, didFailWithError error: Error, resumeData: Data?) {
        NSLog("MonoplanMac: download failed %@", String(describing: error))
    }

    private func isOwnPage(_ url: URL) -> Bool {
        if url.scheme?.lowercased() == AppScheme.scheme { return true }
        if let devURL, url.scheme == devURL.scheme, url.host == devURL.host, url.port == devURL.port { return true }
        return url.scheme == "about" || url.scheme == "blob"
    }

    // MARK: Dialogs the page opens

    /// A file input the page clicks ("Import JSON" in a workspace's
    /// menu): WebKit shows nothing on macOS unless the app supplies the
    /// panel. The input's `accept` is not passed through, so the filter
    /// is the app's: the page's only file input takes JSON, so the panel
    /// takes JSON. Widen this if the page ever grows another input.
    func webView(_ webView: WKWebView, runOpenPanelWith parameters: WKOpenPanelParameters,
                 initiatedByFrame frame: WKFrameInfo, completionHandler: @escaping ([URL]?) -> Void) {
        let panel = NSOpenPanel()
        panel.canChooseFiles = true
        panel.allowedContentTypes = [.json]
        panel.canChooseDirectories = parameters.allowsDirectories
        panel.allowsMultipleSelection = parameters.allowsMultipleSelection
        let finish: (NSApplication.ModalResponse) -> Void = { result in
            completionHandler(result == .OK ? panel.urls : nil)
        }
        guard let window else {
            finish(panel.runModal())
            return
        }
        panel.beginSheetModal(for: window, completionHandler: finish)
    }

    func webView(_ webView: WKWebView, runJavaScriptAlertPanelWithMessage message: String,
                 initiatedByFrame frame: WKFrameInfo, completionHandler: @escaping () -> Void) {
        let alert = NSAlert()
        alert.messageText = message
        guard let window else {
            alert.runModal()
            completionHandler()
            return
        }
        alert.beginSheetModal(for: window) { _ in completionHandler() }
    }

    func webView(_ webView: WKWebView, runJavaScriptConfirmPanelWithMessage message: String,
                 initiatedByFrame frame: WKFrameInfo, completionHandler: @escaping (Bool) -> Void) {
        let alert = NSAlert()
        alert.messageText = message
        alert.addButton(withTitle: String(localized: "OK"))
        alert.addButton(withTitle: String(localized: "Cancel"))
        guard let window else {
            completionHandler(alert.runModal() == .alertFirstButtonReturn)
            return
        }
        alert.beginSheetModal(for: window) { completionHandler($0 == .alertFirstButtonReturn) }
    }
}

/// A JavaScript string literal: JSON-encoded, so nothing in `s` can end
/// the literal.
/// The New Workspace sheet: a standard alert with a name field, Create
/// enabled once the name has a non-blank character. Return in the field
/// is Create (the alert's default button), Escape is Cancel.
final class NewWorkspacePrompt: NSObject, NSTextFieldDelegate {
    private let alert = NSAlert()
    private let field = NSTextField(frame: NSRect(x: 0, y: 0, width: 260, height: 24))

    override init() {
        super.init()
        alert.messageText = String(localized: "New Workspace")
        alert.informativeText = String(localized: "Create a shareable workspace")
        field.placeholderString = String(localized: "Name")
        field.delegate = self
        alert.accessoryView = field
        alert.addButton(withTitle: String(localized: "Create"))
        alert.addButton(withTitle: String(localized: "Cancel"))
        alert.buttons[0].isEnabled = false
    }

    /// `completion` gets the trimmed name, or nil on Cancel.
    func run(on window: NSWindow, completion: @escaping (String?) -> Void) {
        alert.window.initialFirstResponder = field
        alert.beginSheetModal(for: window) { [field] response in
            let name = field.stringValue.trimmingCharacters(in: .whitespacesAndNewlines)
            completion(response == .alertFirstButtonReturn && !name.isEmpty ? name : nil)
        }
    }

    func controlTextDidChange(_ obj: Notification) {
        alert.buttons[0].isEnabled = !field.stringValue.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
    }
}

private func jsString(_ s: String) -> String {
    let data = try! JSONSerialization.data(withJSONObject: [s], options: [])
    return String(String(decoding: data, as: UTF8.self).dropFirst().dropLast())
}

// MARK: - The message handler and the subscriptions

final class MirrorHostBridge: NSObject, WKScriptMessageHandlerWithReply {
    static let handlerName = "monoplan"

    private let hostProvider: () -> MonoplanCore.Host?
    /// Blocking host calls, one at a time, in request order.
    private let work = DispatchQueue(label: "app.monoplan.mac.mirror", qos: .userInitiated)
    /// Main-queue state: the live subscriptions of the current page.
    private var subscriptions: [String: Subscription] = [:]
    /// Bumped per page; a sink from an earlier page is ignored.
    private var generation = 0
    /// The host the current page's session came from; `import` and
    /// the proxy use it rather than asking again, so a host swap
    /// mid-page fails cleanly instead of crossing hosts.
    private var pageHost: MonoplanCore.Host?
    private var lastSync: SyncStatus?
    /// Whether the window is in full screen, replayed to a new page.
    private var fullScreen = false
    /// What `/api` is forwarded with: the server from the session, the
    /// token from the host. Read on the scheme handler's thread.
    private let credentialLock = NSLock()
    private var credential: AppSchemeHandler.Credential?

    init(hostProvider: @escaping () -> MonoplanCore.Host?) {
        self.hostProvider = hostProvider
    }

    var apiCredential: AppSchemeHandler.Credential? {
        credentialLock.lock()
        defer { credentialLock.unlock() }
        return credential
    }

    /// The page's latest `undoState`; set on the main thread.
    var onUndoState: ((Bool, Bool) -> Void)?
    /// The email of the session a page booted with; on the main thread.
    var onSession: ((String) -> Void)?

    /// A new page is loading (or the old one is going): close every
    /// subscription so the host releases the peer slots, and let the
    /// sinks of this generation fall silent.
    func resetPage() {
        generation += 1
        onUndoState?(false, false)
        let old = subscriptions
        subscriptions = [:]
        pageHost = nil
        work.async {
            for (_, sub) in old { sub.close() }
        }
    }

    func pageLoaded(into webView: WKWebView) {
        if let lastSync { push(lastSync, into: webView) }
        if fullScreen { webView.evaluateJavaScript(HostInbound.fullScreen(true).script) }
    }

    func pushFullScreen(_ active: Bool, into webView: WKWebView?) {
        fullScreen = active
        webView?.evaluateJavaScript(HostInbound.fullScreen(active).script)
    }

    func pushSync(_ status: SyncStatus, into webView: WKWebView?) {
        lastSync = status
        if let webView { push(status, into: webView) }
    }

    private func push(_ status: SyncStatus, into webView: WKWebView) {
        webView.evaluateJavaScript(
            HostInbound.sync(online: status.online, lastSyncedAt: status.lastSyncedAt, pending: status.pending).script)
    }

    /// Ask the page to flush to the host and wait for the imports that
    /// causes (the promise `handOff` returns), then for the work queue
    /// to have run them. Bounded: a page that hangs must not hold the
    /// window or quit hostage.
    func handOff(in webView: WKWebView, completion: @escaping () -> Void) {
        var done = false
        let finish = { [work] in
            // Whatever the page posted is already queued behind this.
            work.async { DispatchQueue.main.async {
                if done { return }
                done = true
                completion()
            } }
        }
        webView.callAsyncJavaScript("return \(HostInbound.handOff.script)", arguments: [:], in: nil, in: .page) { _ in finish() }
        DispatchQueue.main.asyncAfter(deadline: .now() + 3) {
            if !done { NSLog("MonoplanMac: window hand-off timed out") }
            finish()
        }
    }

    // MARK: WKScriptMessageHandlerWithReply

    func userContentController(_ userContentController: WKUserContentController, didReceive message: WKScriptMessage,
                               replyHandler: @escaping (Any?, String?) -> Void) {
        let request: HostRequest
        do {
            request = try HostRequest(message: message.body)
        } catch {
            replyHandler(nil, error.localizedDescription)
            return
        }
        let reply = { (result: Result<Any?, Error>) in
            DispatchQueue.main.async {
                switch result {
                case .success(let v): replyHandler(v, nil)
                case .failure(let e): replyHandler(nil, e.localizedDescription)
                }
            }
        }
        switch request {
        case .session:
            // The page's first request: bind this page to the host that
            // is running now.
            guard let host = hostProvider() else {
                reply(.failure(ClientError.notConnected))
                return
            }
            pageHost = host
            work.async { [weak self] in
                let result = Result { try host.session() }
                if case .success(let s) = result {
                    let token = host.deviceToken()
                    self?.credentialLock.lock()
                    self?.credential = token.map { AppSchemeHandler.Credential(serverURL: s.serverUrl, token: $0) }
                    self?.credentialLock.unlock()
                    DispatchQueue.main.async { self?.onSession?(s.email) }
                }
                reply(result.map { s in
                    WindowSession(
                        accountId: s.accountId, email: s.email, deviceId: s.deviceId,
                        primaryDocId: s.primaryDocId, serverURL: s.serverUrl,
                        docs: s.docs.map { .init(docId: $0.docId, name: $0.name) }
                    ).reply
                })
            }

        case .subscribe(let docId):
            guard let host = pageHost else {
                reply(.failure(ClientError.notConnected))
                return
            }
            let gen = generation
            let sink = WindowSink(docId: docId) { [weak self, weak webView = message.webView] inbound in
                DispatchQueue.main.async {
                    guard let self, self.generation == gen, let webView else { return }
                    webView.evaluateJavaScript(inbound.script)
                }
            }
            work.async { [weak self] in
                let result = Result { try host.subscribe(workspace: docId, sink: sink) }
                DispatchQueue.main.async {
                    guard let self else { return }
                    switch result {
                    case .success(let sub):
                        if self.generation != gen {
                            // The page went while the snapshot exported.
                            self.work.async { sub.close() }
                            replyHandler(nil, ClientError.hostStopped.localizedDescription)
                            return
                        }
                        self.subscriptions[docId]?.close()
                        self.subscriptions[docId] = sub
                        replyHandler(HostReply.subscribe(snapshot: sub.snapshot(), peer: sub.peerId()), nil)
                    case .failure(let e):
                        replyHandler(nil, e.localizedDescription)
                    }
                }
            }

        case .unsubscribe(let docId):
            let sub = subscriptions.removeValue(forKey: docId)
            work.async {
                sub?.close()
                reply(.success(NSNull()))
            }

        case .import(let docId, let bytes):
            guard let host = pageHost, let sub = subscriptions[docId] else {
                reply(.failure(ClientError.notConnected))
                return
            }
            work.async {
                reply(Result { try host.import(subscription: sub, bytes: bytes) }.map { NSNull() })
            }

        case .window(let gesture):
            // On the main thread, in the mouse-down's run-loop turn or
            // the next; `currentEvent` is that mouse-down (or the first
            // drag after it), which is what the window drag wants.
            if let window = message.webView?.window { Self.perform(gesture, on: window) }
            replyHandler(NSNull(), nil)

        case .undoState(let canUndo, let canRedo):
            onUndoState?(canUndo, canRedo)
            replyHandler(NSNull(), nil)
        }
    }

    /// What the title bar would have done with the gesture.
    private static func perform(_ gesture: HostRequest.WindowGesture, on window: NSWindow) {
        switch gesture {
        case .drag:
            guard let event = NSApp.currentEvent,
                  event.type == .leftMouseDown || event.type == .leftMouseDragged
            else { return }
            window.performDrag(with: event)
        case .doubleClick:
            // Desktop & Dock → "Double-click a window's title bar to".
            switch UserDefaults.standard.string(forKey: "AppleActionOnDoubleClick") {
            case "None": break
            case "Minimize": window.performMiniaturize(nil)
            default: window.performZoom(nil)
            }
        }
    }
}

/// The uniffi callback for one subscription. Rust calls it on the host
/// loop's thread; `deliver` hops to the main queue and evaluates into
/// the page. It never touches `Host` itself, so an `import` is never
/// made from inside an `update` (the loop would be waiting on itself).
private final class WindowSink: MirrorSink, @unchecked Sendable {
    private let docId: String
    private let deliver: (HostInbound) -> Void

    init(docId: String, deliver: @escaping (HostInbound) -> Void) {
        self.docId = docId
        self.deliver = deliver
    }

    func update(bytes: Data) {
        deliver(.update(docId: docId, bytes: bytes))
    }

    func stopped() {
        deliver(.stopped(docId: docId))
    }
}

// MARK: - The scheme handler

final class AppSchemeHandler: NSObject, WKURLSchemeHandler {
    struct Credential {
        let serverURL: String
        let token: String
    }

    /// The Vite build (`Contents/Resources/web`); nil in a bare
    /// `swift run`, where every file 404s.
    private let webRoot: URL?
    private let credential: () -> Credential?
    /// Tasks WebKit has not stopped. Touched on the main thread only;
    /// a reply to a stopped task throws, so each step checks.
    private var live = Set<ObjectIdentifier>()
    private let session: URLSession = {
        let c = URLSessionConfiguration.ephemeral
        c.httpCookieAcceptPolicy = .never
        c.httpShouldSetCookies = false
        return URLSession(configuration: c)
    }()

    init(webRoot: URL?, credential: @escaping () -> Credential?) {
        self.webRoot = webRoot
        self.credential = credential
    }

    func webView(_ webView: WKWebView, start task: WKURLSchemeTask) {
        live.insert(ObjectIdentifier(task))
        guard let url = task.request.url else {
            fail(task, status: 400)
            return
        }
        switch AppScheme.route(url) {
        case .file(let rel):
            serveFile(rel, to: task)
        case .api(let pathAndQuery):
            proxy(pathAndQuery, task: task)
        case .notFound:
            fail(task, status: 404)
        }
    }

    func webView(_ webView: WKWebView, stop task: WKURLSchemeTask) {
        live.remove(ObjectIdentifier(task))
    }

    private func isLive(_ task: WKURLSchemeTask) -> Bool {
        live.contains(ObjectIdentifier(task))
    }

    private func serveFile(_ rel: String, to task: WKURLSchemeTask) {
        guard let webRoot else {
            fail(task, status: 404)
            return
        }
        let file = webRoot.appendingPathComponent(rel)
        DispatchQueue.global(qos: .userInitiated).async {
            let data = try? Data(contentsOf: file)
            DispatchQueue.main.async {
                guard self.isLive(task) else { return }
                guard let data else {
                    self.fail(task, status: 404)
                    return
                }
                // Hashed assets could be cached forever; the entry page
                // must not be. A bundle build is one version at a time
                // anyway, so no-cache throughout keeps a dev loop honest.
                self.respond(task, status: 200, headers: [
                    "Content-Type": AppScheme.mimeType(for: rel),
                    "Cache-Control": "no-cache",
                ], body: data)
            }
        }
    }

    /// `/api/*` to the host's server as the page asked, plus the device
    /// token (`spec/auth.md`: `Authorization: Bearer`, the CLI's way).
    private func proxy(_ pathAndQuery: String, task: WKURLSchemeTask) {
        guard let credential = credential(), let target = URL(string: credential.serverURL + pathAndQuery) else {
            fail(task, status: 503, code: "no_host", message: "the app has no signed-in host to forward to")
            return
        }
        var req = URLRequest(url: target)
        req.httpMethod = task.request.httpMethod
        req.httpBody = task.request.httpBody
        for name in AppScheme.forwardedRequestHeaders {
            if let v = task.request.value(forHTTPHeaderField: name) { req.setValue(v, forHTTPHeaderField: name) }
        }
        req.setValue("Bearer \(credential.token)", forHTTPHeaderField: "Authorization")
        req.timeoutInterval = 30
        session.dataTask(with: req) { data, response, error in
            DispatchQueue.main.async {
                guard self.isLive(task) else { return }
                guard let http = response as? HTTPURLResponse else {
                    self.fail(task, status: 502, code: "upstream_unreachable",
                              message: error?.localizedDescription ?? "no response")
                    return
                }
                var headers: [String: String] = ["Cache-Control": "no-store"]
                for name in AppScheme.forwardedResponseHeaders {
                    if let v = http.value(forHTTPHeaderField: name) { headers[name] = v }
                }
                self.respond(task, status: http.statusCode, headers: headers, body: data ?? Data())
            }
        }.resume()
    }

    private func respond(_ task: WKURLSchemeTask, status: Int, headers: [String: String], body: Data) {
        guard isLive(task), let url = task.request.url,
              let response = HTTPURLResponse(url: url, statusCode: status, httpVersion: "HTTP/1.1", headerFields: headers)
        else { return }
        task.didReceive(response)
        task.didReceive(body)
        task.didFinish()
        live.remove(ObjectIdentifier(task))
    }

    /// An error in the API's own shape (`api.ts` reads `code` and
    /// `message`), so a proxy failure surfaces like a server one.
    private func fail(_ task: WKURLSchemeTask, status: Int, code: String = "not_found", message: String = "not found") {
        let body = (try? JSONSerialization.data(withJSONObject: ["code": code, "message": message])) ?? Data()
        respond(task, status: status, headers: ["Content-Type": "application/json"], body: body)
    }
}
