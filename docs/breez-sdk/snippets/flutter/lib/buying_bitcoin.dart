import 'package:breez_sdk_spark_flutter/breez_sdk_spark.dart';

Future<void> buyBitcoinBasic(BreezSdk sdk) async {
  // ANCHOR: buy-bitcoin-basic
  // Buy Bitcoin using the SDK's auto-generated deposit address
  final request = BuyBitcoinRequest();

  final response = await sdk.buyBitcoin(request: request);
  print("Open this URL in a browser to complete the purchase:");
  print(response.url);
  // ANCHOR_END: buy-bitcoin-basic
}

Future<void> buyBitcoinWithAmount(BreezSdk sdk) async {
  // ANCHOR: buy-bitcoin-with-amount
  // Lock the purchase to a specific amount (e.g., 0.001 BTC = 100,000 sats)
  final request = BuyBitcoinRequest(lockedAmountSat: 100000);

  final response = await sdk.buyBitcoin(request: request);
  print("Open this URL in a browser to complete the purchase:");
  print(response.url);
  // ANCHOR_END: buy-bitcoin-with-amount
}

Future<void> buyBitcoinWithRedirect(BreezSdk sdk) async {
  // ANCHOR: buy-bitcoin-with-redirect
  // Provide a custom redirect URL for after the purchase
  final request = BuyBitcoinRequest(
      lockedAmountSat: 100000,
      redirectUrl: "https://example.com/purchase-complete");

  final response = await sdk.buyBitcoin(request: request);
  print("Open this URL in a browser to complete the purchase:");
  print(response.url);
  // ANCHOR_END: buy-bitcoin-with-redirect
}

