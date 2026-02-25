import 'package:breez_sdk_spark_flutter/breez_sdk_spark.dart';

/// Extension to add copyWith to the FRB-generated Config class.
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
    String? realTimeSyncServerUrl,
    bool? privateEnabledDefault,
    OptimizationConfig? optimizationConfig,
    StableBalanceConfig? stableBalanceConfig,
    int? maxConcurrentClaims,
    bool? supportLnurlVerify,
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
      realTimeSyncServerUrl:
          realTimeSyncServerUrl ?? this.realTimeSyncServerUrl,
      privateEnabledDefault:
          privateEnabledDefault ?? this.privateEnabledDefault,
      optimizationConfig: optimizationConfig ?? this.optimizationConfig,
      stableBalanceConfig: stableBalanceConfig ?? this.stableBalanceConfig,
      maxConcurrentClaims: maxConcurrentClaims ?? this.maxConcurrentClaims,
      supportLnurlVerify: supportLnurlVerify ?? this.supportLnurlVerify,
    );
  }
}
