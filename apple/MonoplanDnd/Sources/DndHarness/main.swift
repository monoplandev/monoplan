// DndHarness — fake rows in a `DndListView`, to see the drag and drop
// before it goes near the app.
//
//   DndHarness                 a window to drive by hand
//   DndHarness --script <dir>  scripted drags against an off-screen
//                              window; every step is written to <dir>
//                              as NNN-label.png and a line of
//                              frames.jsonl, and the expected outcomes
//                              are checked (exit 1 on a miss)

import AppKit
import MonoplanDnd
import MonoplanDndCore

struct FakeItem {
    var id: String
    var title: String
    var done = false
}

/// One row's content: a tick box, a title, a hairline under it.
final class FakeRow: NSView {
    let check = NSButton(checkboxWithTitle: "", target: nil, action: nil)
    let title = NSTextField(labelWithString: "")
    var onToggle: (() -> Void)?

    override var isFlipped: Bool { true }

    init() {
        super.init(frame: .zero)
        check.target = self
        check.action = #selector(toggled)
        title.lineBreakMode = .byTruncatingTail
        title.font = .systemFont(ofSize: 13)
        addSubview(check)
        addSubview(title)
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }

    @objc private func toggled() { onToggle?() }

    override func layout() {
        super.layout()
        check.frame = NSRect(x: 10, y: (bounds.height - 18) / 2, width: 18, height: 18)
        title.frame = NSRect(x: 36, y: (bounds.height - 17) / 2, width: max(0, bounds.width - 48), height: 17)
    }

    override func draw(_ dirtyRect: NSRect) {
        NSColor.separatorColor.setFill()
        NSRect(x: 36, y: bounds.height - 1, width: bounds.width - 36, height: 1).fill()
    }
}

/// The trailer: a field after the last row; Return adds a row.
final class FakeAddRow: NSView {
    let field = NSTextField()
    var onAdd: ((String) -> Void)?

    override var isFlipped: Bool { true }

    init() {
        super.init(frame: .zero)
        field.placeholderString = "Add an item…"
        field.isBordered = false
        field.drawsBackground = false
        field.focusRingType = .none
        field.font = .systemFont(ofSize: 13)
        field.target = self
        field.action = #selector(submit)
        addSubview(field)
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }

    @objc func submit() {
        let text = field.stringValue.trimmingCharacters(in: .whitespaces)
        guard !text.isEmpty else { return }
        field.stringValue = ""
        onAdd?(text)
    }

    override func layout() {
        super.layout()
        field.frame = NSRect(x: 36, y: (bounds.height - 17) / 2, width: max(0, bounds.width - 48), height: 17)
    }

    /// The slot's presses are its content's: anywhere on it is the field.
    override func mouseDown(with event: NSEvent) {
        window?.makeFirstResponder(field)
    }
}

/// A drop target outside the list, standing in for a sidebar row.
final class DropZone: NSView {
    var hot = false { didSet { needsDisplay = true } }
    private let label = NSTextField(labelWithString: "Bin")

    override init(frame: NSRect) {
        super.init(frame: frame)
        label.alignment = .center
        label.textColor = .secondaryLabelColor
        addSubview(label)
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }

    override func layout() {
        super.layout()
        label.frame = NSRect(x: 0, y: bounds.midY - 9, width: bounds.width, height: 18)
    }

    override func draw(_ dirtyRect: NSRect) {
        (hot ? NSColor.controlAccentColor.withAlphaComponent(0.25) : NSColor.quaternaryLabelColor).setFill()
        NSBezierPath(roundedRect: bounds.insetBy(dx: 8, dy: 8), xRadius: 8, yRadius: 8).fill()
    }
}

final class Harness: NSObject, DndListDataSource, DndListDelegate {
    var items: [String: FakeItem] = [:]
    let list: DndListView
    let addRow = FakeAddRow()
    let zone = DropZone(frame: .zero)
    let status = NSTextField(labelWithString: "Drag rows to reorder, or onto the Bin.")
    let window: NSWindow
    /// What the list reported, newest last: the script checks these.
    var log: [String] = []

    init(count: Int, size: NSSize) {
        var config = DndConfig()
        config.clearOnBlankClick = true
        list = DndListView(config: config)
        window = NSWindow(
            contentRect: NSRect(origin: NSPoint(x: 200, y: 200), size: size),
            styleMask: [.titled, .closable, .resizable, .miniaturizable],
            backing: .buffered, defer: false
        )
        super.init()

        var order: [String] = []
        for i in 0..<count {
            let id = "r\(i)"
            items[id] = FakeItem(id: id, title: "Item \(i)")
            order.append(id)
        }
        list.dataSource = self
        list.delegate = self
        list.badgeText = { "\($0) items" }
        list.setOrder(order, animated: false)
        list.setTrailer(addRow)
        var added = 0
        addRow.onAdd = { [weak self] text in
            guard let self else { return }
            added += 1
            let id = "n\(added)"
            self.items[id] = FakeItem(id: id, title: text)
            self.list.setOrder(self.list.order + [id])
            self.list.scrollToTrailer()
        }

        let content = NSView(frame: NSRect(origin: .zero, size: size))
        let zoneWidth: CGFloat = 120
        let statusHeight: CGFloat = 28
        list.frame = NSRect(x: 0, y: statusHeight, width: size.width - zoneWidth, height: size.height - statusHeight)
        list.autoresizingMask = [.width, .height]
        zone.frame = NSRect(x: size.width - zoneWidth, y: statusHeight, width: zoneWidth, height: size.height - statusHeight)
        zone.autoresizingMask = [.minXMargin, .height]
        status.frame = NSRect(x: 10, y: 6, width: size.width - 20, height: 16)
        status.autoresizingMask = [.width]
        status.textColor = .secondaryLabelColor
        status.font = .systemFont(ofSize: 11)
        content.addSubview(list)
        content.addSubview(zone)
        content.addSubview(status)
        window.contentView = content
        window.title = "DndHarness"
    }

    private func note(_ line: String) {
        log.append(line)
        status.stringValue = line
    }

    // MARK: DndListDataSource

    func dndListMakeRowContent(_ list: DndListView) -> NSView { FakeRow() }

    func dndList(_ list: DndListView, configure content: NSView, key: String, state: DndRowState) {
        guard let row = content as? FakeRow, let item = items[key] else { return }
        row.title.stringValue = item.title
        row.title.textColor = state.selected && state.emphasized ? .alternateSelectedControlTextColor : .labelColor
        row.check.state = item.done ? .on : .off
        row.onToggle = { [weak self, weak list] in
            self?.items[key]?.done.toggle()
            list?.reloadRows([key])
        }
    }

    // MARK: DndListDelegate

    func dndList(_ list: DndListView, didMove keys: [String], before: String?) {
        note("move \(keys.joined(separator: ",")) before \(before ?? "end")")
    }

    func dndListSelectionDidChange(_ list: DndListView) {
        status.stringValue = "selected \(list.selectedKeys.count)"
    }

    func dndListNavigatePastEnd(_ list: DndListView) -> Bool {
        note("past end")
        list.window?.makeFirstResponder(addRow.field)
        list.scrollToTrailer()
        return true
    }

    func dndList(_ list: DndListView, didDoubleClick key: String) {
        note("open \(key)")
    }

    func dndList(_ list: DndListView, menuFor keys: [String]) -> NSMenu? {
        let menu = NSMenu()
        menu.addItem(withTitle: "\(keys.count) selected", action: nil, keyEquivalent: "")
        return menu
    }

    private func overZone(_ windowPoint: NSPoint) -> Bool {
        zone.bounds.contains(zone.convert(windowPoint, from: nil))
    }

    func dndList(_ list: DndListView, dragDidMoveTo windowPoint: NSPoint, keys: [String]) {
        zone.hot = overZone(windowPoint)
    }

    func dndList(_ list: DndListView, dragWillEndAt windowPoint: NSPoint, keys: [String]) -> Bool {
        guard overZone(windowPoint) else { return false }
        let gone = Set(keys)
        note("bin \(keys.joined(separator: ","))")
        // After the drag has wound down: the list is mid-drop here.
        DispatchQueue.main.async {
            list.setOrder(list.order.filter { !gone.contains($0) })
        }
        return true
    }

    func dndListDragDidEnd(_ list: DndListView) {
        zone.hot = false
    }
}

// MARK: - Script

final class Script {
    let harness: Harness
    let dir: URL
    var step = 0
    var failures = 0
    var lines: [String] = []

    var list: DndListView { harness.list }

    init(dir: URL) {
        self.dir = dir
        harness = Harness(count: 60, size: NSSize(width: 520, height: 428))
        harness.list.drivesFrames = false
        harness.window.contentView?.layoutSubtreeIfNeeded()
    }

    /// The centre of the slot `index` rows down the viewport.
    func slot(_ index: Int, dy: CGFloat = 0) -> CGPoint {
        CGPoint(x: 150, y: CGFloat(index) * 32 + 16 + dy)
    }

    /// Back to r0...r59, scrolled to the top.
    func reset() {
        list.setOrder((0..<60).map { "r\($0)" }, animated: false)
        list.scrollView.contentView.scroll(to: .zero)
        list.scrollView.reflectScrolledClipView(list.scrollView.contentView)
    }

    func frames(_ n: Int) {
        for _ in 0..<n { list.step(dt: 1.0 / 60) }
    }

    func check(_ what: String, _ ok: Bool) {
        print("\(ok ? "PASS" : "FAIL")  \(what)")
        if !ok { failures += 1 }
    }

    func capture(_ label: String) {
        step += 1
        let name = String(format: "%03d-%@", step, label)
        guard let content = harness.window.contentView else { return }
        content.layoutSubtreeIfNeeded()
        content.displayIfNeeded()
        let frame = list.debugFrame

        func rect(_ r: CGRect) -> String {
            "[\(Int(r.minX)),\(Int(r.minY.rounded())),\(Int(r.width)),\(Int(r.height))]"
        }
        let rows = frame.rows
            .map { "{\"key\":\"\($0.key)\",\"y\":\(Int($0.frame.minY.rounded())),\"sel\":\($0.selected)}" }
            .joined(separator: ",")
        lines.append(
            "{\"step\":\"\(name)\",\"scrollTop\":\(Int(frame.scrollTop.rounded()))," +
            "\"listHeight\":\(Int(frame.listHeight))," +
            "\"placeholder\":\(frame.placeholder.map(rect) ?? "null")," +
            "\"trailer\":\(frame.trailer.map(rect) ?? "null")," +
            "\"stack\":[\(frame.stack.map(rect).joined(separator: ","))]," +
            "\"order\":\"\(list.order.prefix(10).joined(separator: " "))\"," +
            "\"rows\":[\(rows)]}"
        )

        guard let rep = content.bitmapImageRepForCachingDisplay(in: content.bounds) else { return }
        content.cacheDisplay(in: content.bounds, to: rep)
        // The stack lives in another window: paint it on from its frames.
        if let context = NSGraphicsContext(bitmapImageRep: rep) {
            NSGraphicsContext.saveGraphicsState()
            NSGraphicsContext.current = context
            for (i, stackRect) in frame.stack.enumerated().reversed() {
                // List coordinates are y-down; the bitmap is y-up.
                let inContent = content.convert(stackRect, from: list)
                NSColor.windowBackgroundColor.setFill()
                inContent.fill()
                if i < frame.stackImages.count {
                    context.cgContext.draw(frame.stackImages[i], in: inContent)
                }
                NSColor.controlAccentColor.setStroke()
                NSBezierPath(rect: inContent.insetBy(dx: 0.5, dy: 0.5)).stroke()
            }
            NSGraphicsContext.restoreGraphicsState()
        }
        if let png = rep.representation(using: .png, properties: [:]) {
            try? png.write(to: dir.appendingPathComponent("\(name).png"))
        }
    }

    func run() -> Int32 {
        try? FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        capture("idle")

        // 1. One row, down four slots.
        list.pointerDown(at: slot(2))
        capture("press-r2")
        list.pointerDragged(to: slot(2, dy: 2))
        check("2 px is still a click", !list.model.isDragging)
        list.pointerDragged(to: slot(2, dy: 4))
        frames(1)
        capture("drag-began")
        check("drag began in place: placeholder under r2's slot", list.debugFrame.placeholder?.minY == 64)
        list.pointerDragged(to: slot(4))
        frames(2)
        capture("drag-over-slot4")
        list.pointerDragged(to: slot(6))
        frames(6)
        capture("drag-over-slot6")
        // The pointer took hold 4 px below the row's centre and keeps it.
        check("stack follows the pointer", abs((list.debugFrame.stack.first?.midY ?? 0) - (slot(6).y - 4)) < 1)
        list.pointerUp()
        capture("dropped")
        check("r2 landed before r7", Array(list.order.prefix(8)) == ["r0", "r1", "r3", "r4", "r5", "r6", "r2", "r7"])
        check("host was told", harness.log.last == "move r2 before r7")

        // 2. Three rows in two runs, dragged up to the top.
        reset()
        list.pointerDown(at: slot(1)); list.pointerUp()
        list.pointerDown(at: slot(4), modifiers: .command); list.pointerUp(modifiers: .command)
        list.pointerDown(at: slot(5), modifiers: .shift); list.pointerUp(modifiers: .shift)
        capture("multi-selected")
        check("two runs selected", list.selectedKeys == ["r1", "r4", "r5"])
        list.pointerDown(at: slot(4))
        list.pointerDragged(to: slot(4, dy: -6))
        frames(1)
        capture("multi-drag-began")
        check("three rows left the list", list.debugFrame.rows.count == list.debugFrame.mountedCount
            && !list.debugFrame.rows.contains { ["r1", "r4", "r5"].contains($0.key) })
        list.pointerDragged(to: slot(0))
        frames(6)
        capture("multi-drag-top")
        list.pointerUp()
        capture("multi-dropped")
        check("block landed first", Array(list.order.prefix(6)) == ["r1", "r4", "r5", "r0", "r2", "r3"])
        check("block is still the selection", list.selectedKeys == ["r1", "r4", "r5"])

        // 3. Autoscroll: hold at the bottom edge for a second, then drop.
        reset()
        list.pointerDown(at: slot(0))
        list.pointerDragged(to: slot(0, dy: 8))
        list.pointerDragged(to: CGPoint(x: 150, y: 390))
        frames(1)
        capture("autoscroll-start")
        let hoverBefore = list.model.drag?.hoverIndex ?? -1
        frames(60)
        capture("autoscroll-1s")
        check("list scrolled under a still pointer", list.debugFrame.scrollTop > 300)
        check("hover slot followed the scroll", (list.model.drag?.hoverIndex ?? -1) > hoverBefore + 8)
        let hover = list.model.drag?.hoverIndex ?? 0
        let landBefore = list.model.drag?.collapsed[hover]
        list.pointerDragged(to: CGPoint(x: 150, y: 200))
        check("autoscroll stops off the edge", { self.frames(5); return true }()
            && abs(list.debugFrame.scrollTop - { self.frames(5); return self.list.debugFrame.scrollTop }()) < 0.5)
        list.pointerDragged(to: CGPoint(x: 150, y: 390))
        list.pointerUp()
        capture("autoscroll-dropped")
        check("r0 moved far down", (list.order.firstIndex(of: "r0") ?? 0) >= hover - 1 && landBefore != nil)

        // 4. Escape.
        reset()
        list.pointerDown(at: slot(3))
        list.pointerDragged(to: slot(7))
        frames(4)
        capture("before-escape")
        list.perform(.clear)
        capture("after-escape")
        check("escape cancels with nothing moved", !list.model.isDragging && list.order[3] == "r3" && list.selectedKeys == ["r3"])
        check("no stack left", list.debugFrame.stack.isEmpty && list.debugFrame.placeholder == nil)
        list.pointerUp()

        // 5. Out of the list onto a foreign target.
        let key = list.key(at: slot(2)) ?? ""
        list.pointerDown(at: slot(2))
        list.pointerDragged(to: slot(3))
        list.pointerDragged(to: CGPoint(x: 460, y: 150))
        frames(8)
        capture("over-bin")
        check("off the list: no placeholder, no nudge", list.debugFrame.placeholder == nil)
        check("target is hot", harness.zone.hot)
        list.pointerUp()
        RunLoop.main.run(until: Date().addingTimeInterval(0.05))
        capture("binned")
        check("target consumed the drop", harness.log.last == "bin \(key)" && !list.order.contains(key))

        // 6. Keyboard: move the selection, then reveal the last row.
        reset()
        list.pointerDown(at: slot(1)); list.pointerUp()
        list.perform(.extend(.down))
        list.perform(.moveSelection(.down))
        capture("moved-by-key")
        check("⌘↓ moved the block one row", Array(list.order.prefix(5)) == ["r0", "r3", "r1", "r2", "r4"])
        list.perform(.selectOnlyLast)
        frames(40)
        capture("revealed-last")
        check("⌥↓ scrolled to the end", abs(list.debugFrame.scrollTop - (list.debugFrame.listHeight - 400)) < 1.5)

        // 7. The trailer: after the last row, and ↓ past the end is its.
        check("trailer sits under the last row", list.debugFrame.trailer?.minY == 400 - 32
            && list.debugFrame.rows.last.map { $0.frame.maxY == 400 - 32 } == true)
        list.perform(.navigate(.down))
        check("↓ on the last row hands on", harness.log.last == "past end"
            && harness.window.firstResponder === harness.addRow.field.currentEditor())
        harness.window.makeFirstResponder(list)

        // A short list: the slot follows the rows, not the viewport.
        list.setOrder(["r0", "r1", "r2"], animated: false)
        capture("trailer-short")
        check("trailer under three rows", list.debugFrame.trailer?.minY == 96)
        list.pointerDown(at: slot(1)); list.pointerUp()
        list.pointerDown(at: slot(3)); list.pointerUp()
        check("a press the trailer leaves is a blank click", list.selectedKeys.isEmpty)
        harness.addRow.field.stringValue = "Typed"
        harness.addRow.submit()
        capture("trailer-added")
        check("the row lands where the trailer was", list.order == ["r0", "r1", "r2", "n1"]
            && list.debugFrame.rows.last?.frame.minY == 96 && list.debugFrame.trailer?.minY == 128)

        // A drag: the slot rides the rows' end, the nudge slot included.
        list.pointerDown(at: slot(0))
        list.pointerDragged(to: slot(0, dy: 8))
        list.pointerDragged(to: slot(2))
        frames(2)
        capture("trailer-drag")
        check("one-row drag leaves the trailer where it was", list.debugFrame.trailer?.minY == 128)
        list.pointerDragged(to: CGPoint(x: 460, y: 150))
        capture("trailer-drag-off")
        check("off the list it comes up a row", list.debugFrame.trailer?.minY == 96)
        list.pointerDragged(to: CGPoint(x: 150, y: 140))
        list.pointerUp()
        capture("trailer-dropped")
        check("a drop on the trailer is a drop at the end", list.order == ["r1", "r2", "n1", "r0"]
            && list.debugFrame.trailer?.minY == 128)

        try? lines.joined(separator: "\n").write(
            to: dir.appendingPathComponent("frames.jsonl"), atomically: true, encoding: .utf8
        )
        print("\(step) frames in \(dir.path), \(failures) failed")
        return failures == 0 ? 0 : 1
    }
}

// MARK: - Entry

let app = NSApplication.shared
let args = CommandLine.arguments

if let i = args.firstIndex(of: "--script"), i + 1 < args.count {
    let script = Script(dir: URL(fileURLWithPath: args[i + 1]))
    exit(script.run())
}

final class AppDelegate: NSObject, NSApplicationDelegate {
    var harness: Harness?

    func applicationDidFinishLaunching(_ notification: Notification) {
        let main = NSMenu()
        let appItem = NSMenuItem()
        appItem.submenu = NSMenu()
        appItem.submenu?.addItem(withTitle: "Quit", action: #selector(NSApplication.terminate(_:)), keyEquivalent: "q")
        main.addItem(appItem)
        let editItem = NSMenuItem()
        editItem.submenu = NSMenu(title: "Edit")
        editItem.submenu?.addItem(withTitle: "Select All", action: #selector(NSResponder.selectAll(_:)), keyEquivalent: "a")
        main.addItem(editItem)
        NSApp.mainMenu = main

        let harness = Harness(count: 2000, size: NSSize(width: 560, height: 640))
        self.harness = harness
        harness.window.makeKeyAndOrderFront(nil)
        harness.window.makeFirstResponder(harness.list)
        NSApp.activate(ignoringOtherApps: true)
    }

    func applicationShouldTerminateAfterLastWindowClosed(_ sender: NSApplication) -> Bool { true }
}

let delegate = AppDelegate()
app.delegate = delegate
app.setActivationPolicy(.regular)
app.run()
