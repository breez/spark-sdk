# Breez SDK bindings for Flutter

## Requirements

Flutter 3.38 or later, on Android, iOS or macOS. The package's build hook
bundles the SDK's native library: it downloads the prebuilt library for each
target from the matching release and checks its sha256, so apps don't need a
Rust toolchain. An app that also targets other platforms still builds for
them, but SDK calls fail there.

The SDK needs network access. On Android, release builds need the `INTERNET`
permission in `android/app/src/main/AndroidManifest.xml`. On macOS, add the
`com.apple.security.network.client` entitlement to
`macos/Runner/DebugProfile.entitlements` and `macos/Runner/Release.entitlements`.

## Installation

Add the `breez_sdk_spark_flutter` as a dependency in your pubspec file.

```yaml
dependencies:
  breez_sdk_spark_flutter:
    git:
      url: https://github.com/breez/breez-sdk-spark-flutter
```

## Usage

Import the package and connect:

```dart
import 'package:breez_sdk_spark_flutter/breez_sdk_spark.dart';

Future<void> main() async {
  final config = defaultConfig(network: Network.mainnet).copyWith(apiKey: '<breez api key>');
  final sdk = await connect(
    request: ConnectRequest(
      config: config,
      seed: MnemonicSeed(mnemonic: '<mnemonic words>', passphrase: null),
      storageDir: '<app data directory>',
    ),
  );
}
```

Dart programs that don't run on Flutter, such as command line tools, import
`package:breez_sdk_spark_flutter/breez_sdk_spark_dart.dart` instead. It is the
same API without the Flutter passkey provider.

## Build options

Set these under `hooks: user_defines: breez_sdk_spark_flutter:` in your app's
pubspec:

| Option | Meaning |
|---|---|
| `prebuilt_dir` | A local directory holding the release's library files, for offline builds |
| `prebuilt_base_url` | A mirror to download the release's library files from |
| `build_from_source` | `true` builds the libraries with cargo from a spark-sdk checkout |
| `cargo_profile` | The cargo profile for source builds, `release` by default |

Behind an HTTP proxy, the download needs Flutter 3.44 or later, the first
release that passes `HTTPS_PROXY` to build hooks. On earlier releases, use
`prebuilt_dir`.

## Documentation

Please refer to Flutter examples on the Breez SDK documentation for more information on how to use the SDK.

- [Breez SDK](https://docs.breez.technology/)
