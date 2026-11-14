// The emoji picker: iOS has no picker view of its own, so this is the
// system's emoji keyboard, raised by a text field nobody sees. One
// emoji typed is the pick. A bar over the keyboard closes it without
// one, or takes the emoji away.

import SwiftUI
import UIKit

enum EmojiKeyboard {
    /// The user's emoji keyboard, if they have not removed it in
    /// Settings. Without it the field would raise a letter keyboard,
    /// so nothing offers the picker then.
    static var inputMode: UITextInputMode? {
        UITextInputMode.activeInputModes.first { $0.primaryLanguage == "emoji" }
    }

    static var available: Bool { inputMode != nil }

    /// The last character of what was typed, when it is an emoji (the
    /// user can still switch keyboards with the globe key).
    static func emoji(in text: String) -> String? {
        guard let last = text.last, let first = last.unicodeScalars.first else { return nil }
        let isEmoji = first.properties.isEmojiPresentation
            || (first.properties.isEmoji && last.unicodeScalars.count > 1)
        return isEmoji ? String(last) : nil
    }
}

/// A text field that opens on the emoji keyboard.
private final class EmojiTextField: UITextField {
    override var textInputMode: UITextInputMode? {
        EmojiKeyboard.inputMode ?? super.textInputMode
    }

    /// Without an identifier of its own the system ignores
    /// `textInputMode` and opens on the keyboard last used.
    override var textInputContextIdentifier: String? { "" }
}

/// The unseen field. The emoji keyboard is up while `isActive`; a pick
/// (an emoji, or nil for "remove", offered only while `canRemove`)
/// goes to `onPick` and closes it, as the bar's Done does without one.
struct EmojiKeyboardField: UIViewRepresentable {
    @Binding var isActive: Bool
    var canRemove: Bool
    let onPick: (String?) -> Void

    func makeCoordinator() -> Coordinator { Coordinator(self) }

    func makeUIView(context: Context) -> UITextField {
        let field = EmojiTextField()
        field.delegate = context.coordinator
        field.tintColor = .clear
        field.autocorrectionType = .no
        field.isAccessibilityElement = false
        return field
    }

    func updateUIView(_ field: UITextField, context: Context) {
        context.coordinator.parent = self
        if isActive, !field.isFirstResponder {
            field.inputAccessoryView = context.coordinator.bar(canRemove: canRemove)
            // Not in the middle of a view update, and after the menu
            // that asked for it has let go of the window.
            DispatchQueue.main.async { field.becomeFirstResponder() }
        } else if !isActive, field.isFirstResponder {
            field.resignFirstResponder()
        }
    }

    final class Coordinator: NSObject, UITextFieldDelegate {
        var parent: EmojiKeyboardField

        init(_ parent: EmojiKeyboardField) { self.parent = parent }

        func bar(canRemove: Bool) -> UIToolbar {
            let bar = UIToolbar()
            var items: [UIBarButtonItem] = []
            if canRemove {
                items.append(UIBarButtonItem(title: "Remove Emoji", primaryAction: UIAction { [weak self] _ in
                    self?.finish(pick: .some(nil))
                }))
            }
            items.append(.flexibleSpace())
            items.append(UIBarButtonItem(systemItem: .done, primaryAction: UIAction { [weak self] _ in
                self?.finish(pick: nil)
            }))
            bar.items = items
            bar.sizeToFit()
            return bar
        }

        /// `pick` is nil for no change, `.some(nil)` for "remove".
        private func finish(pick: String??) {
            if let pick { parent.onPick(pick) }
            parent.isActive = false
        }

        func textField(
            _ textField: UITextField, shouldChangeCharactersIn range: NSRange,
            replacementString string: String
        ) -> Bool {
            if let emoji = EmojiKeyboard.emoji(in: string) {
                finish(pick: .some(emoji))
            }
            return false
        }

        func textFieldDidEndEditing(_ textField: UITextField) {
            if parent.isActive { parent.isActive = false }
        }
    }
}
