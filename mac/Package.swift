// swift-tools-version:5.9
import PackageDescription

let package = Package(
    name: "are-you-up-mac",
    platforms: [.macOS(.v13)],
    products: [
        .executable(name: "are-you-up", targets: ["AreYouUp"]),
    ],
    targets: [
        .target(name: "AreYouUpCore"),
        .executableTarget(name: "AreYouUp", dependencies: ["AreYouUpCore"]),
        .testTarget(name: "AreYouUpCoreTests", dependencies: ["AreYouUpCore"]),
    ]
)
