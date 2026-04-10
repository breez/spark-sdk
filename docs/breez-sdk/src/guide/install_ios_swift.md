# iOS/Swift

We support integration via the [Swift Package Manager](https://www.swift.org/package-manager/).
See [breez/breez-sdk-spark-swift](https://github.com/breez/breez-sdk-spark-swift) for more information.

## Swift Package Manager

### Installation via Xcode

Via `File > Add Packages...`, add

```
https://github.com/breez/breez-sdk-spark-swift.git
```

as a package dependency in Xcode.

### Installation via Swift Package Manifest

Add the following to the dependencies array of your `Package.swift`:

``` swift
.package(url: "https://github.com/breez/breez-sdk-spark-swift.git", from: "{VERSION}"),
```

## Example App

For a full working example app, see the [Swift CLI example app](https://github.com/breez/spark-sdk/tree/main/crates/breez-sdk/bindings/examples/cli/langs/swift).
