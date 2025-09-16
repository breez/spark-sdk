# Installing the SDK

The Breez SDK is available in the following platforms:

## iOS/Swift

We support integration via the [Swift Package Manager](https://www.swift.org/package-manager/) and via [CocoaPods](https://cocoapods.org/).
See [breez/breez-sdk-spark-swift](https://github.com/breez/breez-sdk-spark-swift) for more information.

### Swift Package Manager

#### Installation via Xcode

Via `File > Add Packages...`, add

```
https://github.com/breez/breez-sdk-spark-swift.git
```

as a package dependency in Xcode.

#### Installation via Swift Package Manifest

Add the following to the dependencies array of your `Package.swift`:

``` swift
.package(url: "https://github.com/breez/breez-sdk-spark-swift.git", from: "{VERSION}"),
```

### CocoaPods

Add the Breez SDK to your `Podfile` like so:

``` ruby
target '<YourApp>' do
  use_frameworks!
  pod 'BreezSdkSpark'
end
```

## Android/Kotlin

We recommend integrating the Breez SDK as Gradle dependency from [our Maven repository](https://mvn.breez.technology/#/releases).

To do so, add the following to your Gradle dependencies:

```gradle
repositories {
  maven {
      url("https://mvn.breez.technology/releases")
  }
}

dependencies {
  implementation("breez_sdk_spark:bindings-android:{VERSION}")
}
```

## Javascript/Typescript (Wasm)

We recommend using the official npm package: [@breeztech/breez-sdk-spark](https://www.npmjs.com/package/@breeztech/breez-sdk-spark).

> **Note:** If using Node.js, the minimum supported version is v22.

```console
npm install @breeztech/breez-sdk-spark
```
or
```console
yarn add @breeztech/breez-sdk-spark
```

## Rust

We recommend to add breez sdk as a git dependency with a specific release tag.
Check [breez/spark-sdk](https://github.com/breez/spark-sdk/releases) for the latest version.

```toml
[dependencies]
breez-sdk-spark = { git = "https://github.com/breez/spark-sdk", tag = "{VERSION}" }
```

## Flutter

We recommend to add our official flutter package as a git dependency. 

```yaml
dependencies:
  breez_sdk_spark_flutter:
    git:
      url: https://github.com/breez/breez-sdk-spark-flutter
```

## Go

We recommend using our official Go package: [breez/breez-sdk-spark-go](https://github.com/breez/breez-sdk-spark-go).

```console
go get github.com/breez/breez-sdk-spark-go
```

## Python

We recommend using our official Python package: [breez-sdk-spark](https://pypi.org/project/breez-sdk-spark).

```console
pip install breez-sdk-spark
```
