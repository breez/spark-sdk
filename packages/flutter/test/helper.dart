import 'package:breez_sdk_spark_flutter/breez_sdk_spark.dart';

extension ConfigCopyWith on Config {
  Config copyWith({
    String? apiKey,
    Network? network,
    int? syncIntervalSecs,
    Fee? maxDepositClaimFee,
    String? lnurlDomain,
    bool? preferSparkOverLightning,
  }) {
    return Config(
      apiKey: apiKey ?? this.apiKey,
      network: network ?? this.network,
      syncIntervalSecs: syncIntervalSecs ?? this.syncIntervalSecs,
      maxDepositClaimFee: maxDepositClaimFee ?? this.maxDepositClaimFee,
      lnurlDomain: lnurlDomain ?? this.lnurlDomain,
      preferSparkOverLightning: preferSparkOverLightning ?? this.preferSparkOverLightning,
    );
  }
}
