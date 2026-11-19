import Foundation

/// A contiguous run of selected rows. `anchor` is the stable end set
/// when the block was made, `to` the fluid end that shift-extend moves.
public struct DndBlock: Equatable {
    public var anchor: String
    public var to: String
}

/// Multi-block selection, the port of `js/web/src/dnd/core/selection.ts`.
/// Blocks are kept sorted by position and never overlap or touch: every
/// mutation ends in `merge`.
///
/// Two deliberate differences from the web:
/// - `updateOrder` drops selected keys that left the order (the web
///   resolves a missing key to index 0, selecting the first row).
/// - `merge` keeps the whole merged range when the active anchor ends up
///   interior to it (the web drops everything below the anchor:
///   ⌘-clicking the one row between two blocks loses the lower block).
public final class DndSelection {
    private final class Block {
        var anchor: String
        var to: String
        init(_ anchor: String, _ to: String) {
            self.anchor = anchor
            self.to = to
        }
    }

    private var blocks: [Block] = []
    private var active: Block?
    private var orderIndex: [String: Int] = [:]
    private var indexToKey: [String] = []
    private var selectedSetCache: Set<String>?

    /// Called after every change to the selected set or the active block.
    public var onChange: (() -> Void)?

    public init(order: [String] = []) {
        index(order)
    }

    private func notify() {
        selectedSetCache = nil
        onChange?()
    }

    private func index(_ order: [String]) {
        indexToKey = order
        orderIndex.removeAll(keepingCapacity: true)
        for (i, key) in order.enumerated() { orderIndex[key] = i }
        selectedSetCache = nil
    }

    // MARK: Order

    /// Re-index against a new order. Blocks are kept by their end keys,
    /// so a block spans rows inserted inside it. Selected keys that are
    /// gone are dropped, and `onChange` fires if any were.
    public func updateOrder(_ order: [String]) {
        guard order != indexToKey else { return }
        let before = selectedKeys
        let activeAnchor = active?.anchor
        index(order)
        let kept = before.filter { orderIndex[$0] != nil }
        guard kept.count != before.count else {
            // Blocks may have become neighbours (the rows between them left).
            merge()
            return
        }
        blocks = kept.map { Block($0, $0) }
        active = blocks.first { $0.anchor == activeAnchor } ?? blocks.last
        merge()
        notify()
    }

    // MARK: Queries

    public var snapshot: (blocks: [DndBlock], active: DndBlock?) {
        (
            blocks.map { DndBlock(anchor: $0.anchor, to: $0.to) },
            active.map { DndBlock(anchor: $0.anchor, to: $0.to) }
        )
    }

    public var hasSelection: Bool { !blocks.isEmpty }

    public func isSelected(_ key: String) -> Bool {
        guard let idx = orderIndex[key] else { return false }
        return blocks.contains { range($0).contains(idx) }
    }

    /// Selected keys in list order.
    public var selectedKeys: [String] {
        var keys: [String] = []
        for block in blocks {
            for i in range(block) { keys.append(indexToKey[i]) }
        }
        return keys
    }

    /// Memoised set view of `selectedKeys` for per-frame lookups.
    public var selectedKeySet: Set<String> {
        if let cached = selectedSetCache { return cached }
        let set = Set(selectedKeys)
        selectedSetCache = set
        return set
    }

    public var activeBlock: DndBlock? {
        active.map { DndBlock(anchor: $0.anchor, to: $0.to) }
    }

    public var top: String? {
        blocks.first.map { indexToKey[range($0).lowerBound] }
    }

    public var bottom: String? {
        blocks.last.map { indexToKey[range($0).upperBound] }
    }

    public var first: String? { indexToKey.first }
    public var last: String? { indexToKey.last }

    /// The row below, or `key` itself at the end.
    public func next(_ key: String) -> String {
        guard let idx = orderIndex[key], idx < indexToKey.count - 1 else { return key }
        return indexToKey[idx + 1]
    }

    /// The row above, or `key` itself at the top.
    public func prev(_ key: String) -> String {
        guard let idx = orderIndex[key], idx > 0 else { return key }
        return indexToKey[idx - 1]
    }

    // MARK: Operations

    public func selectOnly(_ key: String) {
        guard orderIndex[key] != nil else { return }
        let block = Block(key, key)
        blocks = [block]
        active = block
        notify()
    }

    public func addBlock(_ key: String) {
        guard orderIndex[key] != nil else { return }
        let block = Block(key, key)
        blocks.append(block)
        active = block
        merge()
        notify()
    }

    public func extendActive(_ key: String) {
        guard let itemIdx = orderIndex[key] else { return }
        guard let active,
              let anchorIdx = orderIndex[active.anchor],
              let toIdx = orderIndex[active.to]
        else {
            selectOnly(key)
            return
        }
        // A target on the far side of the anchor pivots the anchor to the
        // current `to`, so the block grows rather than flips: shift-select
        // outside the range is always additive.
        if anchorIdx != toIdx,
           (itemIdx - anchorIdx).signum() == -(toIdx - anchorIdx).signum() {
            active.anchor = active.to
        }
        active.to = key
        merge()
        notify()
    }

    public func toggle(_ key: String) {
        if isSelected(key) {
            remove(key)
            notify()
        } else {
            addBlock(key)
        }
    }

    /// Replace the selection with exactly `keys`; adjacent ones coalesce.
    public func setSelectedKeys(_ keys: [String]) {
        blocks = keys.filter { orderIndex[$0] != nil }.map { Block($0, $0) }
        active = blocks.last
        merge()
        notify()
    }

    public func selectAll() {
        guard let first = indexToKey.first, let last = indexToKey.last else { return }
        let block = Block(first, last)
        blocks = [block]
        active = block
        notify()
    }

    public func clear() {
        guard !blocks.isEmpty else { return }
        blocks = []
        active = nil
        notify()
    }

    // MARK: Private

    private func range(_ block: Block) -> ClosedRange<Int> {
        let a = orderIndex[block.anchor] ?? 0
        let b = orderIndex[block.to] ?? 0
        return min(a, b)...max(a, b)
    }

    private func remove(_ key: String) {
        guard let itemIdx = orderIndex[key] else { return }
        var next: [Block] = []
        for block in blocks {
            let r = range(block)
            if !r.contains(itemIdx) {
                next.append(block)
            } else if r.count == 1 {
                if active === block { active = nil }
            } else if itemIdx == r.lowerBound {
                block.anchor = indexToKey[r.lowerBound + 1]
                block.to = indexToKey[r.upperBound]
                next.append(block)
            } else if itemIdx == r.upperBound {
                block.anchor = indexToKey[r.lowerBound]
                block.to = indexToKey[r.upperBound - 1]
                next.append(block)
            } else {
                // Interior: split, the lower half stays active.
                let upper = Block(indexToKey[r.lowerBound], indexToKey[itemIdx - 1])
                let lower = Block(indexToKey[itemIdx + 1], indexToKey[r.upperBound])
                next.append(upper)
                next.append(lower)
                if active === block { active = lower }
            }
        }
        blocks = next
        if active == nil {
            // The active block went: the nearest remaining one takes over.
            active = blocks.min { distance($0, itemIdx) < distance($1, itemIdx) }
        }
    }

    private func distance(_ block: Block, _ idx: Int) -> Int {
        let r = range(block)
        return min(abs(r.lowerBound - idx), abs(r.upperBound - idx))
    }

    private func merge() {
        guard blocks.count > 1 else { return }
        let activeAnchor = active?.anchor
        let activeTop = active.map { range($0).lowerBound }
        // Stable for equal tops is not needed: equal tops overlap and merge.
        blocks.sort { range($0).lowerBound < range($1).lowerBound }

        var merged: [Block] = []
        var current = blocks[0]
        var currentHasActive = active === current
        for next in blocks.dropFirst() {
            let cur = range(current)
            let nxt = range(next)
            if cur.upperBound + 1 >= nxt.lowerBound {
                let newTop = cur.lowerBound
                let newBottom = max(cur.upperBound, nxt.upperBound)
                if active === next { currentHasActive = true }
                if currentHasActive, let anchor = activeAnchor,
                   let anchorIdx = orderIndex[anchor],
                   anchorIdx == newTop || anchorIdx == newBottom {
                    // Keep the active anchor; `to` is the far end.
                    current = Block(anchor, indexToKey[anchorIdx == newTop ? newBottom : newTop])
                } else {
                    current = Block(indexToKey[newTop], indexToKey[newBottom])
                }
            } else {
                merged.append(current)
                current = next
                currentHasActive = active === next
            }
        }
        merged.append(current)

        if let activeTop {
            active = merged.first { range($0).contains(activeTop) } ?? active
        }
        blocks = merged
    }
}
