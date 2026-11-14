// Edit › Undo and Redo: the target of both menu items, so ⌘Z and ⇧⌘Z
// mean one thing at a time. While a text field or text view is editing
// (the key window's first responder is an editable `NSTextView`, the
// field editor included) they are that text's own undo, through its
// `undoManager`, titled as AppKit titles them ("Undo Typing"). Anywhere
// else they are the doc's: the engine's one history over every open
// workspace (`Engine.undo` / `redo`, `cli/src/undo.rs`), where each
// write the app made is one step and the newest goes first wherever it
// was made. The window rereads on the engine's `changed`, as after any
// write.
//
// Public API only: the menu items have an explicit target, so nothing
// depends on which responder answers a nil-targeted `undo:`.

import AppKit
import MonoplanCore

final class EditUndo: NSObject, NSMenuItemValidation {
    private let engine: EngineController

    init(engine: EngineController) {
        self.engine = engine
    }

    /// The text being edited, if any: then undo is its.
    private var editingText: NSTextView? {
        guard let text = NSApp.keyWindow?.firstResponder as? NSTextView, text.isEditable else { return nil }
        return text
    }

    @objc func undo(_ sender: Any?) {
        if let text = editingText {
            text.undoManager?.undo()
        } else {
            step(redo: false)
        }
    }

    @objc func redo(_ sender: Any?) {
        if let text = editingText {
            text.undoManager?.redo()
        } else {
            step(redo: true)
        }
    }

    func validateMenuItem(_ item: NSMenuItem) -> Bool {
        let redo = item.action == #selector(redo(_:))
        if let text = editingText {
            let manager = text.undoManager
            item.title = redo
                ? manager?.redoMenuItemTitle ?? String(localized: "Redo")
                : manager?.undoMenuItemTitle ?? String(localized: "Undo")
            return (redo ? manager?.canRedo : manager?.canUndo) ?? false
        }
        item.title = redo ? String(localized: "Redo") : String(localized: "Undo")
        // Non-blocking: the loop publishes it after every command.
        guard let engine = engine.calls?.engine else { return false }
        return redo ? engine.canRedo() : engine.canUndo()
    }

    private func step(redo: Bool) {
        guard let calls = engine.calls else {
            NSSound.beep()
            return
        }
        calls.run({ try redo ? $0.redo() : $0.undo() }) { result in
            switch result {
            case .success(true): break
            // Nothing left to step (a sync rearmed the history since
            // the menu was validated), or the engine refused.
            case .success(false), .failure: NSSound.beep()
            }
        }
    }
}
