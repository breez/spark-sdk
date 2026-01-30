# iOS/Swift

We support integration via the [Swift Package Manager](https://www.swift.org/package-manager/) and via [CocoaPods](https://cocoapods.org/).
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

## CocoaPods

Add the Breez SDK to your `Podfile` like so:

``` ruby
target '<YourApp>' do
  use_frameworks!
  pod 'BreezSdkSpark'
end
```
