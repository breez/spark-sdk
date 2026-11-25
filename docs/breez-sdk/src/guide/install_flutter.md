# Flutter

We recommend to add our official flutter package as a git dependency.

```yaml
dependencies:
  breez_sdk_spark_flutter:
    git:
      url: https://github.com/breez/breez-sdk-spark-flutter
```

The package needs Flutter 3.38 or later, and supports Android, iOS (15.0 or later) and macOS. Its build hook downloads the SDK's prebuilt native library for each target your app builds for, so no Rust toolchain is needed. An app that also targets other platforms still builds for them, but SDK calls fail there.

The SDK needs network access. On Android, release builds need the `INTERNET` permission in `android/app/src/main/AndroidManifest.xml`. On macOS, add the `com.apple.security.network.client` entitlement to `macos/Runner/DebugProfile.entitlements` and `macos/Runner/Release.entitlements`.

Moving from an earlier version of the package? See [Migrating to the uniffi-dart Flutter package](flutter_migration.md).

## Example App

For a full working example app, see the [Flutter CLI example app](https://github.com/breez/spark-sdk/tree/main/crates/breez-sdk/bindings/examples/cli/langs/flutter).
