import 'package:breez_sdk_spark_flutter/breez_sdk_spark.dart';
import 'helper.dart';
import 'dart:async';

Future<void> configureMaxDepositClaimFee() async {
  // ANCHOR: max-deposit-claim-fee
  // Create the default config
  var config = defaultConfig(network: Network.mainnet)
      .copyWith(apiKey: "<breez api key>");

  // Disable automatic claiming
  config = config.copyWith(maxDepositClaimFee: null);

  // Set a maximum feerate of 10 sat/vB
  config = config.copyWith(
      maxDepositClaimFee: MaxFee.rate(satPerVbyte: BigInt.from(10)));

  // Set a maximum fee of 1000 sat
  config = config.copyWith(
      maxDepositClaimFee: MaxFee.fixed(amount: BigInt.from(1000)));

  // Set the maximum fee to the fastest network recommended fee at the time of claim
  // with a leeway of 1 sats/vbyte
  config = config.copyWith(
      maxDepositClaimFee:
          MaxFee.networkRecommended(leewaySatPerVbyte: BigInt.from(1)));
  // ANCHOR_END: max-deposit-claim-fee
  print(config);
}

Future<void> configurePrivateEnabledDefault() async {
  // ANCHOR: private-enabled-default
  // Disable Spark private mode by default
  var config = defaultConfig(network: Network.mainnet)
      .copyWith(privateEnabledDefault: false);
  // ANCHOR_END: private-enabled-default
  print(config);
}

Future<void> configureOptimizationConfiguration() async {
  // ANCHOR: optimization-configuration
  var config = defaultConfig(network: Network.mainnet).copyWith(
      optimizationConfig:
          OptimizationConfig(autoEnabled: true, multiplicity: 1));
  // ANCHOR_END: optimization-configuration
  print(config);
}

Future<void> configureStableBalance() async {
  // ANCHOR: stable-balance-config
  var config = defaultConfig(network: Network.mainnet).copyWith(
      // Enable stable balance with auto-conversion to a specific token
      stableBalanceConfig: StableBalanceConfig(
          tokenIdentifier: "<token_identifier>",
          thresholdSats: BigInt.from(10000),
          maxSlippageBps: 100,
          reservedSats: BigInt.from(1000)
          ));
  // ANCHOR_END: stable-balance-config
  print(config);
}
