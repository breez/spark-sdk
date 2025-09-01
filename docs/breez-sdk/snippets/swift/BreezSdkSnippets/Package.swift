// swift-tools-version: 5.9
// The swift-tools-version declares the minimum version of Swift required to build this package.

import PackageDescription

let package = Package(
    name: "BreezSdkSnippets",
    platforms: [.macOS("15.0")],
    dependencies: [
        .package(url: "https://github.com/apple/swift-argument-parser", from: "1.2.3"),
        .package(url: "https://github.com/breez/breez-sdk-spark-swift", exact: "0.1.1")
        // To use a local version of breez-sdk-spark, comment-out the above and un-comment:
        // .package(name: "bindings-swift", path: "/local-path/breez-sdk-spark/crate/breez-sdk/bindings/langs/swift")
    ],
    targets: [
        // Targets are the basic building blocks of a package, defining a module or a test suite.
        // Targets can depend on other targets in this package and products from dependencies.
        .executableTarget(
            name: "BreezSdkSnippets",
            dependencies: [
                .product(name: "BreezSdkSpark", package: "breez-sdk-spark-swift"),
                // To use a local version of breez-sdk-spark, comment-out the above and un-comment:
                // .product(name: "BreezSdkSpark", package: "bindings-swift"),
            ],
            path: "Sources",
            linkerSettings: [
                .linkedFramework("SystemConfiguration")
            ]),
    ]
)
