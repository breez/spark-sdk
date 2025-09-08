# Breez Spark SDK bindings for Flutter

## Installation

Add the `breez_sdk_spark_flutter` as a dependency in your pubspec file.

```yaml
dependencies:
  breez_sdk_spark_flutter:
    git:
      url: https://github.com/breez/breez-sdk-spark-flutter
```

## Usage

To start using this package first import it in your Dart file.

```dart
import 'package:breez_sdk_spark_flutter/breez_sdk_spark.dart';

Future<void> main() async {
  await BreezSdkSparkLib.init();

  ...
}
```

Extending the `Config` to set the API key:

```dart
extension ConfigCopyWith on Config {
  Config copyWith({
    String? apiKey,
    Network? network,
    int? syncIntervalSecs,
    Fee? maxDepositClaimFee,
  }) {
    return Config(
      apiKey: apiKey ?? this.apiKey,
      network: network ?? this.network,
      syncIntervalSecs: syncIntervalSecs ?? this.syncIntervalSecs,
      maxDepositClaimFee: maxDepositClaimFee ?? this.maxDepositClaimFee,
    );
  }
}
```

## Documentation

Please refer to Flutter examples on Breez SDK - Nodeless *(Spark Implementation)* documentation for more information on how to use the SDK.

- [Breez SDK - Nodeless *(Spark Implementation)*](https://sdk-doc-spark.breez.technology/)