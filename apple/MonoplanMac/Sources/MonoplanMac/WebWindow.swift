// The app's window (spec/mac-window-plan.md, phase 4): a WKWebView
// running the web client (`js/web/mirror.html`, built into the bundle)
// over a mirror of the in-process host's docs. Three pieces:
//
// - `WebWindowController`: the NSWindow and the webview, its lifecycle
//   (open from the tray, hand everything off to the host before it
//   closes or the app quits), activation policy via `onVisibility`.
// - `MirrorHostBridge`: the `monoplan` message handler the page posts
//   to (`session`, `subscribe`, `unsubscribe`, `import`, `window`,
//   `openSettings`) and the calls
//   back into `window.__monoplanHost` (`update`, `stopped`, `sync`,
//   `handOff`, `settings`). One `Host.subscribe` per open doc; a `MirrorSink` per
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
import WebKit

final class WebWindowController: NSWindowController, NSWindowDelegate, WKNavigationDelegate, WKUIDelegate {
    /// Where the host comes from at request time: the running one, or
    /// nil while there is none (the page then shows its boot error and
    /// is reloaded when a host is back, see `hostAvailable`).
    private let hostProvider: () -> MonoplanCore.Host?
    /// The tray switches the activation policy on this: a regular app
    /// with a Dock icon and menu bar while the window is up, a bare
    /// status item otherwise.
    private let onVisibility: (Bool) -> Void
    private let bridge: MirrorHostBridge
    private let schemeHandler: AppSchemeHandler
    private var webView: WKWebView?
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

    init(hostProvider: @escaping () -> MonoplanCore.Host?, onVisibility: @escaping (Bool) -> Void,
         onOpenSettings: @escaping () -> Void) {
        self.hostProvider = hostProvider
        self.onVisibility = onVisibility
        self.bridge = MirrorHostBridge(hostProvider: hostProvider, onOpenSettings: onOpenSettings)
        self.schemeHandler = AppSchemeHandler(
            webRoot: Bundle.main.resourceURL?.appendingPathComponent("web", isDirectory: true),
            credential: { [bridge] in bridge.apiCredential })
        self.devURL = ProcessInfo.processInfo.environment["MONOPLAN_MIRROR_URL"].flatMap(URL.init(string:))
        super.init(window: nil)
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

    /// The host's sync status, for the page's footer. Kept and replayed
    /// on load, pushed live afterwards.
    func pushSync(_ status: SyncStatus) {
        bridge.pushSync(status, into: webView)
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
        if #available(macOS 13.3, *) {
            webView.isInspectable = debug
        }
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
        window.contentView = webView
        window.delegate = self
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

    func windowWillClose(_ notification: Notification) {
        handedOff = false
        tearDown()
        onVisibility(false)
    }

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
    /// browser: the website link in the menu, a link in a note.
    func webView(_ webView: WKWebView, decidePolicyFor navigationAction: WKNavigationAction,
                 decisionHandler: @escaping (WKNavigationActionPolicy) -> Void) {
        guard let url = navigationAction.request.url else {
            decisionHandler(.cancel)
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

    private func isOwnPage(_ url: URL) -> Bool {
        if url.scheme?.lowercased() == AppScheme.scheme { return true }
        if let devURL, url.scheme == devURL.scheme, url.host == devURL.host, url.port == devURL.port { return true }
        return url.scheme == "about" || url.scheme == "blob"
    }

    // MARK: Dialogs the page opens

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
private func jsString(_ s: String) -> String {
    let data = try! JSONSerialization.data(withJSONObject: [s], options: [])
    return String(String(decoding: data, as: UTF8.self).dropFirst().dropLast())
}

// MARK: - The message handler and the subscriptions

final class MirrorHostBridge: NSObject, WKScriptMessageHandlerWithReply {
    static let handlerName = "monoplan"

    private let hostProvider: () -> MonoplanCore.Host?
    /// The page asked for Settings: the app's window (the tray's).
    private let onOpenSettings: () -> Void
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
    /// What `/api` is forwarded with: the server from the session, the
    /// token from the host. Read on the scheme handler's thread.
    private let credentialLock = NSLock()
    private var credential: AppSchemeHandler.Credential?

    init(hostProvider: @escaping () -> MonoplanCore.Host?, onOpenSettings: @escaping () -> Void) {
        self.hostProvider = hostProvider
        self.onOpenSettings = onOpenSettings
    }

    var apiCredential: AppSchemeHandler.Credential? {
        credentialLock.lock()
        defer { credentialLock.unlock() }
        return credential
    }

    /// A new page is loading (or the old one is going): close every
    /// subscription so the host releases the peer slots, and let the
    /// sinks of this generation fall silent.
    func resetPage() {
        generation += 1
        let old = subscriptions
        subscriptions = [:]
        pageHost = nil
        work.async {
            for (_, sub) in old { sub.close() }
        }
    }

    func pageLoaded(into webView: WKWebView) {
        if let lastSync { push(lastSync, into: webView) }
    }

    func pushSync(_ status: SyncStatus, into webView: WKWebView?) {
        lastSync = status
        if let webView { push(status, into: webView) }
    }

    private func push(_ status: SyncStatus, into webView: WKWebView) {
        webView.evaluateJavaScript(HostInbound.sync(online: status.online, lastSyncedAt: status.lastSyncedAt).script)
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

        case .openSettings:
            onOpenSettings()
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
