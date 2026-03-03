// swift-tools-version:5.9

import PackageDescription

let package = Package(
    name: "breez-cli",
    platforms: [
        .macOS("15.0"),
    ],
    dependencies: [
        .package(url: "https://github.com/breez/breez-sdk-spark-swift.git", from: "0.10.0"),
    ],
    targets: [
        .systemLibrary(name: "CEditLine"),
        .executableTarget(
            name: "breez-cli",
            dependencies: [
                .product(name: "BreezSdkSpark", package: "breez-sdk-spark-swift"),
                "CEditLine",
            ],
            path: "Sources/BreezCLI"
        ),
    ]
)
