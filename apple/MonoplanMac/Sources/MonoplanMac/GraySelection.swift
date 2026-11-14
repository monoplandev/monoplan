// A selected row is gray, not the accent colour, in every SwiftUI
// `List` of the app (the sidebar, the item lists, Settings).
//
// AppKit draws a selected row in the accent colour while the row is
// "emphasized" (its table is first responder in the key window) and in
// gray otherwise. SwiftUI has no say in it, so the row view SwiftUI's
// lists use (`SwiftUI.ListTableRowView`, private) is patched to never
// be emphasized: the getter answers false and the setter stores false.
// The item lists draw from the getter; the sidebar's selection is a
// visual effect view the setter configures. Text and icons follow, as
// they take their colour from the same flag.
//
// Probed on macOS 26 (2026-11-14). If SwiftUI renames the class this
// does nothing and the selection is the accent colour again.

import AppKit
import ObjectiveC

enum GraySelection {
    static func install() {
        guard let rowClass = NSClassFromString("SwiftUI.ListTableRowView"),
            rowClass.isSubclass(of: NSTableRowView.self)
        else { return }
        let pairs = [
            (#selector(getter: NSTableRowView.isEmphasized), #selector(getter: NSTableRowView.monoplan_isEmphasized)),
            (#selector(setter: NSTableRowView.isEmphasized), #selector(setter: NSTableRowView.monoplan_isEmphasized)),
        ]
        for (original, replacement) in pairs {
            guard let inherited = class_getInstanceMethod(rowClass, original),
                let ours = class_getInstanceMethod(NSTableRowView.self, replacement)
            else { return }
            // Give the class its own copy of a method it only inherits,
            // so the exchange never touches `NSTableRowView` itself.
            class_addMethod(
                rowClass, original, method_getImplementation(inherited), method_getTypeEncoding(inherited))
            guard let own = class_getInstanceMethod(rowClass, original) else { return }
            method_exchangeImplementations(own, ours)
        }
    }
}

private extension NSTableRowView {
    /// Exchanged with `isEmphasized` on SwiftUI's row class, so the
    /// setter's call below is the original setter.
    @objc var monoplan_isEmphasized: Bool {
        get { false }
        set { self.monoplan_isEmphasized = false }
    }
}
