// swift-tools-version:5.9
//
// MonoplanMac — the macOS menubar app. Decided 2026-10-26 (see
// spec/daemon-plan.md "Decision"): the tray and the future window app are
// one process, and that process is a *client* of `monoplan daemon`, never
// a host. It holds no doc: it watches the daemon's socket and re-fetches
// the agenda projection (the projection tier in spec/daemon-plan.md), so
// MonoplanCore (the doc mirror) is not a dependency and will only become
// one if a window view ever needs in-process reads.
//
// Two targets so the protocol code is testable without AppKit:
// `MonoplanMacKit` (MessagePack codec, daemon client, agenda model) and
// the `MonoplanMac` executable (status item, menu). Run from the repo root
// with `bun run tray`; `swift test --package-path apple/MonoplanMac` runs
// the codec and model tests.

import PackageDescription

let package = Package(
    name: "MonoplanMac",
    platforms: [.macOS(.v14)],
    targets: [
        .target(
            name: "MonoplanMacKit",
            path: "Sources/MonoplanMacKit"
        ),
        .executableTarget(
            name: "MonoplanMac",
            dependencies: ["MonoplanMacKit"],
            path: "Sources/MonoplanMac"
        ),
        .testTarget(
            name: "MonoplanMacKitTests",
            dependencies: ["MonoplanMacKit"],
            path: "Tests/MonoplanMacKitTests"
        ),
    ]
)
