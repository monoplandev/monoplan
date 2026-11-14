// The main menu, shown while the window is open (the app is `.regular`
// then; as a bare status item it has no menu bar). Standard items wired
// to the responder chain: the Edit menu is what makes ⌘Z, ⌘X, ⌘C, ⌘V
// and ⌘A reach a text field at all, since AppKit routes key equivalents
// through the menu.
//
// New Workspace… is the one doc-level command here: the app asks for
// the name and the engine does the work. It sits in the app menu, where
// the user asked for it; there is no File menu to put it under.

import AppKit

enum MainMenu {
    static func build(newWorkspace: Selector, settings: Selector, target: AnyObject) -> NSMenu {
        let main = NSMenu()

        let app = NSMenu()
        app.addItem(withTitle: String(localized: "About Monoplan"),
                    action: #selector(NSApplication.orderFrontStandardAboutPanel(_:)), keyEquivalent: "")
        app.addItem(.separator())
        // ⇧⌘N, Finder's new-folder chord.
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
        // The focused text field's own undo, through the responder
        // chain; the doc has no undo in this window yet.
        edit.addItem(withTitle: String(localized: "Undo"), action: Selector(("undo:")), keyEquivalent: "z")
        let redoItem = edit.addItem(withTitle: String(localized: "Redo"), action: Selector(("redo:")), keyEquivalent: "z")
        redoItem.keyEquivalentModifierMask = [.command, .shift]
        edit.addItem(.separator())
        edit.addItem(withTitle: String(localized: "Cut"), action: #selector(NSText.cut(_:)), keyEquivalent: "x")
        edit.addItem(withTitle: String(localized: "Copy"), action: #selector(NSText.copy(_:)), keyEquivalent: "c")
        edit.addItem(withTitle: String(localized: "Paste"), action: #selector(NSText.paste(_:)), keyEquivalent: "v")
        edit.addItem(withTitle: String(localized: "Select All"), action: #selector(NSText.selectAll(_:)), keyEquivalent: "a")
        main.addItem(submenu(edit, title: edit.title))

        let view = NSMenu(title: String(localized: "View"))
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
