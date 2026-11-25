// swift-tools-version:5.9
//
// MonoplanMac — the macOS menubar app's package half. The app itself
// is built by `Monoplan.xcodeproj` beside this file (apple/README.md),
// whose one target compiles `Sources/MonoplanMac` and links
// `MonoplanMacKit` from here and MonoplanCore; run it from the repo
// root with `bun run mac`.
//
// Two targets: `MonoplanMacKit` (Foundation-only helpers with no
// MonoplanCore dependency) and the `MonoplanMac` executable over the
// app's sources. The executable is here for tests only, nothing ships
// from it: `MonoplanMacTests` covers the app's pure code (the tray's
// rows) and needs a module to import. `swift test --package-path
// apple/MonoplanMac` runs both test targets; the second links
// MonoplanCore, so it needs `bun run build:apple` run first.
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
        .package(path: "../MonoplanDnd"),
    ],
    targets: [
        .target(
            name: "MonoplanMacKit",
            path: "Sources/MonoplanMacKit"
        ),
        .executableTarget(
            name: "MonoplanMac",
            dependencies: [
                "MonoplanMacKit", "MonoplanCore",
                .product(name: "MonoplanDnd", package: "MonoplanDnd"),
                .product(name: "MonoplanDndCore", package: "MonoplanDnd"),
            ],
            path: "Sources/MonoplanMac",
            // The tick sound (`DoneSound`): the Xcode target bundles it,
            // the test executable never plays it.
            exclude: ["done.wav"]
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
