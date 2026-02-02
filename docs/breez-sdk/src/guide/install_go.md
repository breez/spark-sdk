# Go

We recommend using our official Go package: [breez/breez-sdk-spark-go](https://github.com/breez/breez-sdk-spark-go).

```console
go get github.com/breez/breez-sdk-spark-go
```

## Integration

For [Android](#android) and [Windows](#windows) the provided binding libraries need to be copied into a location where they need to be found during runtime. 

For [iOS](#ios) the native binary framework need additionaly installing using [Swift Package Manager](#swift-package-manager) or [CocoaPods](#cocoapods).

### Android

Copy the binding libraries into the jniLibs directory of your app
```bash
cp vendor/github.com/breez/breez-sdk-spark-go/breez_sdk_spark/lib/android-aarch64/*.so android/app/src/main/jniLibs/arm64-v8a/
cp vendor/github.com/breez/breez-sdk-spark-go/breez_sdk_spark/lib/android-amd64/*.so android/app/src/main/jniLibs/x86_64/
```
So they are in the following structure
```
└── android
    ├── app
        └── src
            └── main
                └── jniLibs
                    ├── arm64-v8a
                        ├── libbreez_sdk_spark_bindings.so
                        └── libc++_shared.so
                    └── x86_64
                        ├── libbreez_sdk_spark_bindings.so
                        └── libc++_shared.so
                └── AndroidManifest.xml
        └── build.gradle
    └── build.gradle
```

### Darwin (macOS)

For development, `go run` and `go build` work out of the box since the bundled `.dylib` is referenced via `rpath` pointing into the Go module cache.

For deployment, create a universal dylib and place it in your app bundle's Frameworks directory:

```bash
lipo -create \
  vendor/github.com/breez/breez-sdk-spark-go/breez_sdk_spark/lib/darwin-aarch64/libbreez_sdk_spark_bindings.dylib \
  vendor/github.com/breez/breez-sdk-spark-go/breez_sdk_spark/lib/darwin-amd64/libbreez_sdk_spark_bindings.dylib \
  -output YourMacOSApp/Contents/Frameworks/libbreez_sdk_spark_bindings.dylib
```

### iOS

When targeting iOS, you must also install the native binary framework. This is the same framework used by the [Swift Breez SDK package](install_ios_swift.md) and can be installed via [Swift Package Manager](#swift-package-manager) or [CocoaPods](#cocoapods).

**Note:** The Go and Swift packages (installed via SPM or CocoaPods) **MUST** have the same version. A version mismatch between the two will cause linking or runtime errors.


#### Swift Package Manager

##### Installation via Xcode

Via `File > Add Packages...`, add

```
https://github.com/breez/breez-sdk-spark-swift.git
```

as a package dependency in Xcode.

##### Installation via Swift Package Manifest

Add the following to the dependencies array of your `Package.swift`:

``` swift
.package(url: "https://github.com/breez/breez-sdk-spark-swift.git"),
```

#### CocoaPods

Add the Breez SDK to your `Podfile` like so and run `pod install`:

``` ruby
target '<YourApp>' do
  use_frameworks!
  pod 'breez_sdk_sparkFFI'
end
```

### Windows

Copy the binding library to the same directory as the executable file or include the library into the windows install packager.
```bash
cp vendor/github.com/breez/breez-sdk-spark-go/breez_sdk_spark/lib/windows-amd64/*.dll build/windows/
```
