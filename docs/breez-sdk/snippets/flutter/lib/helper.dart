import 'package:breez_sdk_spark_flutter/breez_sdk_spark.dart';

extension ConfigCopyWith on Config {
  Config copyWith({
    String? apiKey,
    Network? network,
    int? syncIntervalSecs,
    MaxFee? maxDepositClaimFee,
    String? lnurlDomain,
    bool? preferSparkOverLightning,
    List<ExternalInputParser>? externalInputParsers,
    bool? useDefaultExternalInputParsers,
    bool? privateEnabledDefault,
    OptimizationConfig? optimizationConfig,
    StableBalanceConfig? stableBalanceConfig,
  }) {
    return Config(
      apiKey: apiKey ?? this.apiKey,
      network: network ?? this.network,
      syncIntervalSecs: syncIntervalSecs ?? this.syncIntervalSecs,
      maxDepositClaimFee: maxDepositClaimFee ?? this.maxDepositClaimFee,
      lnurlDomain: lnurlDomain ?? this.lnurlDomain,
      preferSparkOverLightning:
          preferSparkOverLightning ?? this.preferSparkOverLightning,
      externalInputParsers: externalInputParsers ?? this.externalInputParsers,
      useDefaultExternalInputParsers:
          useDefaultExternalInputParsers ?? this.useDefaultExternalInputParsers,
      privateEnabledDefault:
          privateEnabledDefault ?? this.privateEnabledDefault,
      optimizationConfig: optimizationConfig ?? this.optimizationConfig,
      stableBalanceConfig: stableBalanceConfig ?? this.stableBalanceConfig,
    );
  }
}
