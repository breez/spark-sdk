import '../breez_sdk_spark.dart';

/// Extension to add copyWith to the FRB-generated Config class.
///
/// FRB does not generate copyWith for regular (non-sealed) classes.
/// This extension provides it so consumers can modify individual fields
/// without reconstructing the entire Config object.
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
    SparkConfig? sparkConfig,
  }) {
    return Config(
      apiKey: apiKey ?? this.apiKey,
      network: network ?? this.network,
      syncIntervalSecs: syncIntervalSecs ?? this.syncIntervalSecs,
      maxDepositClaimFee: maxDepositClaimFee ?? this.maxDepositClaimFee,
      lnurlDomain: lnurlDomain ?? this.lnurlDomain,
      preferSparkOverLightning: preferSparkOverLightning ?? this.preferSparkOverLightning,
      externalInputParsers: externalInputParsers ?? this.externalInputParsers,
      useDefaultExternalInputParsers: useDefaultExternalInputParsers ?? this.useDefaultExternalInputParsers,
      realTimeSyncServerUrl: realTimeSyncServerUrl ?? this.realTimeSyncServerUrl,
      privateEnabledDefault: privateEnabledDefault ?? this.privateEnabledDefault,
      optimizationConfig: optimizationConfig ?? this.optimizationConfig,
      stableBalanceConfig: stableBalanceConfig ?? this.stableBalanceConfig,
      maxConcurrentClaims: maxConcurrentClaims ?? this.maxConcurrentClaims,
      sparkConfig: sparkConfig ?? this.sparkConfig,
    );
  }
}
