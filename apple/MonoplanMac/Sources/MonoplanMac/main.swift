// The menubar app's entry point. Top-level code rather than `@main` so the
// activation policy is set before the run loop starts: `.accessory` gives a
// status item with no Dock icon or menu bar of its own, which is what a
// tray-only process should be. When a window is added later, opening it
// switches to `.regular` and closing it switches back.
//
// The tray holds no doc. It watches the daemon (one socket connection,
// `Watch`) and re-fetches `agenda --days 0 --json` when the daemon says
// something changed, once a minute (the relative times move), and when
// the menu opens.

import AppKit
import MonoplanMacKit

/// Where "Open Monoplan" and item rows go. The dev stack serves the web
/// client on 5176 (see `local/process-compose.yaml`), so
/// `MONOPLAN_WEB_URL=http://localhost:5176 bun run tray` points it there.
let webURL: URL = {
    if let raw = ProcessInfo.processInfo.environment["MONOPLAN_WEB_URL"],
       let url = URL(string: raw) {
        return url
    }
    return URL(string: "https://alpha.monoplan.app")!
}()

/// `MONOPLAN_TRAY_DEBUG=1` logs every menu rebuild (connection state,
/// rows, fetch error) so the tray can be checked from a terminal.
let debug = ProcessInfo.processInfo.environment["MONOPLAN_TRAY_DEBUG"] == "1"

final class TrayController: NSObject, NSMenuDelegate {
    private let statusItem: NSStatusItem
    private let menu = NSMenu()
    private let client: DaemonClient
    private var lines: [MenuLine] = []
    private var fetchError: String?
    private var refreshQueued = false
    private var minuteTimer: Timer?

    override init() {
        statusItem = NSStatusBar.system.statusItem(withLength: NSStatusItem.squareLength)
        client = DaemonClient(location: ProfileLocation.resolve())
        super.init()

        if let button = statusItem.button {
            // Template image so it follows the menu bar's light/dark tint.
            let image = NSImage(systemSymbolName: "checklist", accessibilityDescription: "Monoplan")
            image?.isTemplate = true
            button.image = image
            button.toolTip = "Monoplan"
        }
        menu.delegate = self
        statusItem.menu = menu
        rebuildMenu()

        client.onState = { [weak self] state in
            if case .connected = state { self?.refresh() } else { self?.rebuildMenu() }
        }
        client.onEvent = { [weak self] event in
            switch event {
            case .changed: self?.scheduleRefresh()
            case .sync: self?.rebuildMenu()
            }
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
                    self.fetchError = "could not read agenda: \(error.localizedDescription)"
                }
            case .success(let other):
                self.fetchError = "unexpected reply: \(other)"
            case .failure(let error):
                if case ClientError.notConnected = error { break }
                self.fetchError = error.localizedDescription
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
            NSLog("MonoplanMac: state=%@ lines=%@ error=%@",
                  String(describing: client.state),
                  lines.map { "\($0.title)\($0.note.map { " [\($0)]" } ?? "")" }.joined(separator: " | "),
                  fetchError ?? "-")
        }
        menu.removeAllItems()

        let header = NSMenuItem(title: "Today · \(Self.headerFormatter.string(from: Date()))", action: nil, keyEquivalent: "")
        header.isEnabled = false
        menu.addItem(header)

        switch client.state {
        case .connecting(let reason):
            addDisabled("Daemon not running · \(reason)")
        case .connected:
            if let fetchError {
                addDisabled(fetchError)
            } else if lines.isEmpty {
                addDisabled("Nothing today")
            } else {
                for line in lines { menu.addItem(rowItem(line)) }
            }
        }

        if case .connected(let sync) = client.state, let notice = sync.offlineNotice() {
            menu.addItem(.separator())
            addDisabled(notice)
        }

        menu.addItem(.separator())
        let open = NSMenuItem(title: "Open Monoplan", action: #selector(openWebsite), keyEquivalent: "o")
        open.target = self
        menu.addItem(open)
        let quit = NSMenuItem(title: "Quit Monoplan", action: #selector(quit), keyEquivalent: "q")
        quit.target = self
        menu.addItem(quit)
    }

    private func addDisabled(_ title: String) {
        let item = NSMenuItem(title: title, action: nil, keyEquivalent: "")
        item.isEnabled = false
        menu.addItem(item)
    }

    private func rowItem(_ line: MenuLine) -> NSMenuItem {
        let item = NSMenuItem(title: "", action: #selector(openItem(_:)), keyEquivalent: "")
        item.target = self
        item.representedObject = line.id
        let title = NSMutableAttributedString(
            string: line.done ? "✓ \(line.title)" : line.title,
            attributes: line.done ? [.foregroundColor: NSColor.secondaryLabelColor] : [:]
        )
        if let note = line.note {
            title.append(NSAttributedString(
                string: "   \(note)",
                attributes: [
                    .foregroundColor: note == "overdue" ? NSColor.systemRed : NSColor.secondaryLabelColor,
                    .font: NSFont.menuFont(ofSize: NSFont.smallSystemFontSize),
                ]
            ))
        }
        item.attributedTitle = title
        return item
    }

    private static let headerFormatter: DateFormatter = {
        let f = DateFormatter()
        f.setLocalizedDateFormatFromTemplate("EEE d MMM")
        return f
    }()

    // MARK: Actions

    @objc private func openWebsite() {
        NSWorkspace.shared.open(webURL)
    }

    /// Item links are `#item_<id>` (spec/urls.md "Item URLs name only the item").
    @objc private func openItem(_ sender: NSMenuItem) {
        guard let id = sender.representedObject as? String,
              var comps = URLComponents(url: webURL, resolvingAgainstBaseURL: false)
        else { return }
        comps.fragment = "item_\(id)"
        if let url = comps.url { NSWorkspace.shared.open(url) }
    }

    /// Quits this process only. The daemon is a separate process and
    /// keeps syncing; `monoplan daemon stop` is the way to end it.
    @objc private func quit() {
        client.stop()
        NSApp.terminate(nil)
    }
}

final class AppDelegate: NSObject, NSApplicationDelegate {
    private var tray: TrayController?

    func applicationDidFinishLaunching(_ notification: Notification) {
        tray = TrayController()
    }
}

let app = NSApplication.shared
let delegate = AppDelegate()
app.delegate = delegate
app.setActivationPolicy(.accessory)
app.run()
