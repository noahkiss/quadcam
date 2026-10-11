// swift-tools-version:5.9
// quadcam-intel: QuadCam's helper for Apple's on-device frameworks (docs/development.md,
// "The Swift helper"). It builds with the newest SDK and launches on macOS 13: each feature
// checks the running OS and reports itself unavailable on an older one.

import PackageDescription

let package = Package(
    name: "quadcam-intel",
    platforms: [.macOS(.v13)],
    targets: [
        .executableTarget(
            name: "quadcam-intel",
            path: "Sources/quadcam-intel"
        )
    ]
)
