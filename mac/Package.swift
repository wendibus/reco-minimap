// swift-tools-version: 6.0
import PackageDescription

// The Mac app is a thin launcher, built like the one of Reco Trainer: a native window around the web
// interface, plus the code that starts the reco-minimap program and installs signed updates of it
// ("payload"). Everything else is the Rust program in the repository root.
let package = Package(
    name: "RecoMinimapMac",
    platforms: [.macOS(.v14)],
    products: [
        .executable(name: "RecoMinimapMac", targets: ["RecoMinimapMac"])
    ],
    targets: [
        // Foundation-only logic (versions, payload store, update planning, signatures) - unit tested.
        .target(name: "LauncherCore"),
        // AppKit/WebKit shell and process management.
        .executableTarget(name: "RecoMinimapMac", dependencies: ["LauncherCore"]),
        .testTarget(name: "LauncherCoreTests", dependencies: ["LauncherCore"]),
    ],
    swiftLanguageModes: [.v5]
)
