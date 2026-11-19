import 'generated/breez_sdk_spark.dart';

/// Copies a [Config], replacing the given fields, so callers can adjust a
/// default config without listing every field.
extension ConfigCopyWith on Config {
  Config copyWith({
    String? apiKey,
    Network? network,
    int? syncIntervalSecs,
    MaxFee? maxDepositClaimFee,
    String? lnurlDomain,
    bool? preferSparkOverLightning,
    bool? exitChainAutoFetchEnabled,
    bool? watchtowerFeeLadderEnabled,
    List<ExternalInputParser>? externalInputParsers,
    bool? useDefaultExternalInputParsers,
    String? realTimeSyncServerUrl,
    bool? privateEnabledDefault,
    LeafOptimizationConfig? leafOptimizationConfig,
    TokenOptimizationConfig? tokenOptimizationConfig,
    StableBalanceConfig? stableBalanceConfig,
    int? maxConcurrentClaims,
    SparkConfig? sparkConfig,
    bool? backgroundTasksEnabled,
    ProxyConfig? proxy,
    CrossChainConfig? crossChainConfig,
  }) {
    return Config(
      apiKey: apiKey ?? this.apiKey,
      network: network ?? this.network,
      syncIntervalSecs: syncIntervalSecs ?? this.syncIntervalSecs,
      maxDepositClaimFee: maxDepositClaimFee ?? this.maxDepositClaimFee,
      lnurlDomain: lnurlDomain ?? this.lnurlDomain,
      preferSparkOverLightning: preferSparkOverLightning ?? this.preferSparkOverLightning,
      exitChainAutoFetchEnabled: exitChainAutoFetchEnabled ?? this.exitChainAutoFetchEnabled,
      watchtowerFeeLadderEnabled: watchtowerFeeLadderEnabled ?? this.watchtowerFeeLadderEnabled,
      externalInputParsers: externalInputParsers ?? this.externalInputParsers,
      useDefaultExternalInputParsers: useDefaultExternalInputParsers ?? this.useDefaultExternalInputParsers,
      realTimeSyncServerUrl: realTimeSyncServerUrl ?? this.realTimeSyncServerUrl,
      privateEnabledDefault: privateEnabledDefault ?? this.privateEnabledDefault,
      leafOptimizationConfig: leafOptimizationConfig ?? this.leafOptimizationConfig,
      tokenOptimizationConfig: tokenOptimizationConfig ?? this.tokenOptimizationConfig,
      stableBalanceConfig: stableBalanceConfig ?? this.stableBalanceConfig,
      maxConcurrentClaims: maxConcurrentClaims ?? this.maxConcurrentClaims,
      sparkConfig: sparkConfig ?? this.sparkConfig,
      backgroundTasksEnabled: backgroundTasksEnabled ?? this.backgroundTasksEnabled,
      proxy: proxy ?? this.proxy,
      crossChainConfig: crossChainConfig ?? this.crossChainConfig,
    );
  }
}
