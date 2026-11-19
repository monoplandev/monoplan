import Foundation

public struct DndModifiers: OptionSet, Hashable {
    public let rawValue: Int
    public init(rawValue: Int) { self.rawValue = rawValue }

    public static let shift = DndModifiers(rawValue: 1 << 0)
    public static let command = DndModifiers(rawValue: 1 << 1)
    public static let option = DndModifiers(rawValue: 1 << 2)
}

public enum DndKey: Equatable {
    case up
    case down
    case escape
    case a
}

public enum DndKeyAction: Equatable {
    case selectOnlyFirst
    case selectOnlyLast
    case navigate(DndDirection)
    case extend(DndDirection)
    case extendToFirst
    case extendToLast
    case moveSelection(DndDirection)
    case selectAll
    case clear
}

/// The web's binding table (`core/keyboard.ts`), macOS column. Nil for a
/// key the list does not answer.
public func dndKeyAction(_ key: DndKey, _ mods: DndModifiers) -> DndKeyAction? {
    let shift = mods.contains(.shift)
    let cmd = mods.contains(.command)
    let opt = mods.contains(.option)
    switch key {
    case .escape:
        return .clear
    case .a:
        return cmd ? .selectAll : nil
    case .up, .down:
        let dir: DndDirection = key == .up ? .up : .down
        if opt, !shift { return dir == .up ? .selectOnlyFirst : .selectOnlyLast }
        if shift, cmd { return dir == .up ? .extendToFirst : .extendToLast }
        if shift { return .extend(dir) }
        if cmd { return .moveSelection(dir) }
        if !opt { return .navigate(dir) }
        return nil
    }
}
