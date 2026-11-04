// The main menu, shown while the window is open (the app is `.regular`
// then; as a bare status item it has no menu bar). Mostly standard
// items wired to the responder chain: the Edit menu is what makes ⌘Z,
// ⌘X, ⌘C, ⌘V and ⌘A reach the webview at all, since AppKit routes key
// equivalents through the menu. The page sees each key first (WebKit
// gives it the chance to `preventDefault`), so the client's own
// shortcuts keep winning inside it; the menu catches what the page
// leaves alone, such as copy in a text field. Undo and Redo are the
// exception: they target the app, which asks the page to undo (a
// focused text field's own undo first, else the doc's), so a click on
// the menu and ⌘Z do the same thing and the window has one undo
// surface; the page's sidebar menu hides its own rows there. The
// items are enabled from the page's `undoState`.
//
// New Workspace… is the one doc-level command here: the page's own
// item is hidden in the window (creating a doc wraps a key the webview
// does not hold, spec/mac-window-plan.md), so the app asks for the name
// natively and the host does the work. It sits in the app menu, where
// the user asked for it; there is no File menu to put it under.

import AppKit

enum MainMenu {
    static func build(newWorkspace: Selector, settings: Selector, reload: Selector, undo: Selector, redo: Selector,
                      zoomIn: Selector, zoomOut: Selector, actualSize: Selector, target: AnyObject) -> NSMenu {
        let main = NSMenu()

        let app = NSMenu()
        app.addItem(withTitle: String(localized: "About Monoplan"),
                    action: #selector(NSApplication.orderFrontStandardAboutPanel(_:)), keyEquivalent: "")
        app.addItem(.separator())
        // ⇧⌘N, Finder's new-folder chord; the page binds nothing on it.
        let newWorkspaceItem = app.addItem(withTitle: String(localized: "New Workspace…"), action: newWorkspace, keyEquivalent: "n")
        newWorkspaceItem.keyEquivalentModifierMask = [.command, .shift]
        newWorkspaceItem.target = target
        let settingsItem = app.addItem(withTitle: String(localized: "Settings…"), action: settings, keyEquivalent: ",")
        settingsItem.target = target
        app.addItem(.separator())
        app.addItem(withTitle: String(localized: "Hide Monoplan"), action: #selector(NSApplication.hide(_:)), keyEquivalent: "h")
        let hideOthers = app.addItem(withTitle: String(localized: "Hide Others"),
                                     action: #selector(NSApplication.hideOtherApplications(_:)), keyEquivalent: "h")
        hideOthers.keyEquivalentModifierMask = [.command, .option]
        app.addItem(withTitle: String(localized: "Show All"), action: #selector(NSApplication.unhideAllApplications(_:)), keyEquivalent: "")
        app.addItem(.separator())
        app.addItem(withTitle: String(localized: "Quit Monoplan"), action: #selector(NSApplication.terminate(_:)), keyEquivalent: "q")
        main.addItem(submenu(app, title: "Monoplan"))

        let edit = NSMenu(title: String(localized: "Edit"))
        let undoItem = edit.addItem(withTitle: String(localized: "Undo"), action: undo, keyEquivalent: "z")
        undoItem.target = target
        let redoItem = edit.addItem(withTitle: String(localized: "Redo"), action: redo, keyEquivalent: "z")
        redoItem.keyEquivalentModifierMask = [.command, .shift]
        redoItem.target = target
        edit.addItem(.separator())
        edit.addItem(withTitle: String(localized: "Cut"), action: #selector(NSText.cut(_:)), keyEquivalent: "x")
        edit.addItem(withTitle: String(localized: "Copy"), action: #selector(NSText.copy(_:)), keyEquivalent: "c")
        edit.addItem(withTitle: String(localized: "Paste"), action: #selector(NSText.paste(_:)), keyEquivalent: "v")
        edit.addItem(withTitle: String(localized: "Select All"), action: #selector(NSText.selectAll(_:)), keyEquivalent: "a")
        main.addItem(submenu(edit, title: edit.title))

        let view = NSMenu(title: String(localized: "View"))
        let reloadItem = view.addItem(withTitle: String(localized: "Reload"), action: reload, keyEquivalent: "r")
        reloadItem.target = target
        view.addItem(.separator())
        // Page zoom, Safari's items. ⌘+ is really ⌘⇧= on most layouts,
        // so Zoom In also answers plain ⌘= through a hidden twin, as
        // Safari does; the menu shows one row.
        let zoomInItem = view.addItem(withTitle: String(localized: "Zoom In"), action: zoomIn, keyEquivalent: "+")
        zoomInItem.target = target
        let zoomInTwin = view.addItem(withTitle: String(localized: "Zoom In"), action: zoomIn, keyEquivalent: "=")
        zoomInTwin.target = target
        zoomInTwin.isHidden = true
        let zoomOutItem = view.addItem(withTitle: String(localized: "Zoom Out"), action: zoomOut, keyEquivalent: "-")
        zoomOutItem.target = target
        let actualSizeItem = view.addItem(withTitle: String(localized: "Actual Size"), action: actualSize, keyEquivalent: "0")
        actualSizeItem.target = target
        view.addItem(.separator())
        // AppKit retitles this to "Exit Full Screen" while the window is in
        // full screen. ⌃⌘F is the system binding; fn-F keeps working too.
        let fullScreen = view.addItem(withTitle: String(localized: "Enter Full Screen"),
                                      action: #selector(NSWindow.toggleFullScreen(_:)), keyEquivalent: "f")
        fullScreen.keyEquivalentModifierMask = [.control, .command]
        main.addItem(submenu(view, title: view.title))

        let window = NSMenu(title: String(localized: "Window"))
        window.addItem(withTitle: String(localized: "Minimize"), action: #selector(NSWindow.performMiniaturize(_:)), keyEquivalent: "m")
        window.addItem(withTitle: String(localized: "Zoom"), action: #selector(NSWindow.performZoom(_:)), keyEquivalent: "")
        window.addItem(.separator())
        window.addItem(withTitle: String(localized: "Close"), action: #selector(NSWindow.performClose(_:)), keyEquivalent: "w")
        main.addItem(submenu(window, title: window.title))
        NSApp.windowsMenu = window

        return main
    }

    private static func submenu(_ menu: NSMenu, title: String) -> NSMenuItem {
        let item = NSMenuItem(title: title, action: nil, keyEquivalent: "")
        item.submenu = menu
        return item
    }
}
