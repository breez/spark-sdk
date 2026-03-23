import 'package:breez_sdk_spark_flutter/breez_sdk_spark.dart';
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

Future<void> configureSparkConfig() async {
  // ANCHOR: spark-config
  var config = defaultConfig(network: Network.mainnet).copyWith(
      // Connect to a custom Spark environment
      sparkConfig: SparkConfig(
          coordinatorIdentifier:
              '0000000000000000000000000000000000000000000000000000000000000001',
          threshold: 2,
          signingOperators: [
            SparkSigningOperator(
                id: 0,
                identifier:
                    '0000000000000000000000000000000000000000000000000000000000000001',
                address: 'https://0.spark.example.com',
                identityPublicKey:
                    '03acd9a5a88db102730ff83dee69d69088cc4c9d93bbee893e90fd5051b7da9651'),
            SparkSigningOperator(
                id: 1,
                identifier:
                    '0000000000000000000000000000000000000000000000000000000000000002',
                address: 'https://1.spark.example.com',
                identityPublicKey:
                    '02d2d103cacb1d6355efeab27637c74484e2a7459e49110c3fe885210369782e23'),
            SparkSigningOperator(
                id: 2,
                identifier:
                    '0000000000000000000000000000000000000000000000000000000000000003',
                address: 'https://2.spark.example.com',
                identityPublicKey:
                    '0350f07ffc21bfd59d31e0a7a600e2995273938444447cb9bc4c75b8a895dbb853'),
          ],
          sspConfig: SparkSspConfig(
              baseUrl: 'https://api.example.com',
              identityPublicKey:
                  '02e0b8d42c5d3b5fe4c5beb6ea796ab3bc8aaf28a3d3195407482c67e0b58228a5',
              schemaEndpoint: 'graphql/spark/rc'),
          expectedWithdrawBondSats: BigInt.from(10000),
          expectedWithdrawRelativeBlockLocktime: BigInt.from(1000)));
  // ANCHOR_END: spark-config
  print(config);
}
