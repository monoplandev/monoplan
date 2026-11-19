// swift-tools-version:5.9
//
// MonoplanDnd — the web's virtualised drag-and-drop list
// (`js/web/src/dnd/`) on AppKit, with the same primitives: rows are
// plain views positioned by frame inside a scroll view, and the nudge,
// placeholder, drag stack and autoscroll are ours. No `NSTableView`, no
// `NSDraggingSession`. Contract and web → AppKit mapping in `spec.md`.
//
// A spike (2026-11-20): nothing in the apps links this yet.
//
//   MonoplanDndCore   Foundation-only port of the web's `core/`:
//                     selection, virtualisation, key map, the drag
//                     state machine. No AppKit, so UIKit can share it.
//   MonoplanDnd       `DndListView` and the drag overlay (AppKit).
//   DndHarness        A window of fake rows to try it by hand, and
//                     `--script <dir>` to run scripted drags and dump
//                     every frame as PNG + JSON.
//
//   swift test --package-path apple/MonoplanDnd
//   swift run  --package-path apple/MonoplanDnd DndHarness
//   swift run  --package-path apple/MonoplanDnd DndHarness --script <dir>

import PackageDescription

let package = Package(
    name: "MonoplanDnd",
    platforms: [.macOS(.v14), .iOS(.v17)],
    products: [
        .library(name: "MonoplanDndCore", targets: ["MonoplanDndCore"]),
        .library(name: "MonoplanDnd", targets: ["MonoplanDnd"]),
    ],
    targets: [
        .target(name: "MonoplanDndCore", path: "Sources/MonoplanDndCore"),
        .target(
            name: "MonoplanDnd",
            dependencies: ["MonoplanDndCore"],
            path: "Sources/MonoplanDnd"
        ),
        .executableTarget(
            name: "DndHarness",
            dependencies: ["MonoplanDnd"],
            path: "Sources/DndHarness"
        ),
        .testTarget(
            name: "MonoplanDndCoreTests",
            dependencies: ["MonoplanDndCore"],
            path: "Tests/MonoplanDndCoreTests"
        ),
    ]
)
