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
  config = config.copyWith(maxDepositClaimFee: Fee.rate(satPerVbyte: BigInt.from(10)));

  // Set a maximum fee of 1000 sat
  config = config.copyWith(maxDepositClaimFee: Fee.fixed(amount: BigInt.from(1000)));
  // ANCHOR_END: max-deposit-claim-fee
  print(config);
}