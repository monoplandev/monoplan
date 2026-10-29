// swift-tools-version:5.9
//
// MonoplanMac — the macOS menubar app. Decided 2026-10-26 (see
// spec/daemon-plan.md "Decision"): the tray and the future window app are
// one process, and that process is a *client* of `monoplan daemon`, never
// a host. It holds no doc: it watches the daemon's socket and re-fetches
// the agenda projection (the projection tier in spec/daemon-plan.md).
// Amended 2026-10-27: the app *hosts* the doc in-process (`Host` from
// MonoplanCore, the same Rust loop as `monoplan daemon`). Its UI is an
// in-process client of that host through the FFI (2026-10-29,
// spec/mac-window-plan.md); the socket is for the CLI. Run
// `bun run build:apple` once before building: MonoplanCore wraps a
// prebuilt XCFramework.
//
// Two targets so the protocol code is testable without AppKit:
// `MonoplanMacKit` (MessagePack codec, host client over a transport
// protocol, agenda model) and
// the `MonoplanMac` executable (status item, menu). Run from the repo root
// with `bun run mac`; `swift test --package-path apple/MonoplanMac` runs
// the codec and model tests.

import PackageDescription

let package = Package(
    name: "MonoplanMac",
    platforms: [.macOS(.v14)],
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
    ]
)
