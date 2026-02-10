import 'package:breez_sdk_spark_flutter/breez_sdk_spark.dart';

Future<void> buyBitcoin(BreezSdk sdk) async {
  // ANCHOR: buy-bitcoin
  // Buy Bitcoin with funds deposited directly into the user's wallet.
  // Optionally lock the purchase to a specific amount and provide a redirect URL.
  final request = BuyBitcoinRequest(
      lockedAmountSat: BigInt.from(100000),
      redirectUrl: "https://example.com/purchase-complete");

  final response = await sdk.buyBitcoin(request: request);
  print("Open this URL in a browser to complete the purchase:");
  print(response.url);
  // ANCHOR_END: buy-bitcoin
}
