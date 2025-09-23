library;

import 'package:breez_sdk_spark_flutter/src/rust/models.dart';

export 'src/rust/frb_generated.dart' show BreezSdkSparkLib;
export 'src/rust/errors.dart';
export 'src/rust/events.dart';
export 'src/rust/logger.dart';
export 'src/rust/models.dart';
export 'src/rust/sdk_builder.dart';
export 'src/rust/sdk.dart';

extension SDKConfig on Config {
  Config copyWith({
  String? apiKey,
  Network? network,
  int? syncIntervalSecs,
  Fee? maxDepositClaimFee,
  String? lnurlDomain,
  }) {
    return Config(
      apiKey: apiKey ?? this.apiKey,
      network: network ?? this.network,
      syncIntervalSecs: syncIntervalSecs ?? this.syncIntervalSecs,
      maxDepositClaimFee: maxDepositClaimFee ?? this.maxDepositClaimFee,
      lnurlDomain: lnurlDomain ?? this.lnurlDomain,
    );
  }
}