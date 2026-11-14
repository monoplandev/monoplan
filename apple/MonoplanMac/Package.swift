// swift-tools-version:5.9
//
// MonoplanMac — the macOS menubar app. The app *runs the engine*
// in-process (`Engine` from MonoplanCore, the same Rust loop as
// `monoplan daemon`; spec/daemon-plan.md, amended 2026-10-27) and its
// tray and window reach it through the typed calls
// (spec/swift-bindings-plan.md, phase 4); the socket is for the CLI.
// Run `bun run build:apple` once before building: MonoplanCore wraps a
// prebuilt XCFramework.
//
// Two targets: `MonoplanMacKit` (Foundation-only helpers with no
// MonoplanCore dependency) and the `MonoplanMac` executable (status item, menu, window, settings). Run
// from the repo root with `bun run mac`; `swift test --package-path
// apple/MonoplanMac` runs both test targets. `MonoplanMacTests` covers
// the executable's pure code (the tray's rows) and so links
// MonoplanCore: it needs `build:apple` run first.
//
// `MonoplanMacKit` is also a product: the iOS app and its widget
// (`apple/MonoplanIOS`) consume it for `ProfileLocation`, `WhenPhrase`
// and the widget's `AgendaSnapshot`, so the package lists iOS too. The
// Kit is Foundation-only except the CLI-symlink helper, which is
// `#if os(macOS)`. Its name predates the second consumer; the hoist
// into a platform-neutral package is a bounded follow-up.

import PackageDescription

let package = Package(
    name: "MonoplanMac",
    platforms: [.macOS(.v14), .iOS(.v17)],
    products: [
        .library(name: "MonoplanMacKit", targets: ["MonoplanMacKit"]),
    ],
    dependencies: [
        .package(path: "../MonoplanCore"),
    ],
    targets: [
        .target(
            name: "MonoplanMacKit",
            path: "Sources/MonoplanMacKit"
        ),
        .executableTarget(
            name: "MonoplanMac",
            dependencies: ["MonoplanMacKit", "MonoplanCore"],
            path: "Sources/MonoplanMac"
        ),
        .testTarget(
            name: "MonoplanMacKitTests",
            dependencies: ["MonoplanMacKit"],
            path: "Tests/MonoplanMacKitTests"
        ),
        .testTarget(
            name: "MonoplanMacTests",
            dependencies: ["MonoplanMac", "MonoplanCore"],
            path: "Tests/MonoplanMacTests"
        ),
    ]
)
