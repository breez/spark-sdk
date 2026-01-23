import 'package:breez_sdk_spark/breez_sdk_spark.dart';

Future<void> buyBitcoinBasic(BreezSdk sdk) async {
  // ANCHOR: buy-bitcoin-basic
  final request = BuyBitcoinRequest(
      address: "bc1qexample...", // Your Bitcoin address
      lockedAmountSat: null,
      maxAmountSat: null,
      redirectUrl: null);

  final response = await sdk.buyBitcoin(request: request);
  print("Open this URL in a browser to complete the purchase:");
  print(response.url);
  // ANCHOR_END: buy-bitcoin-basic
}

Future<void> buyBitcoinWithAmount(BreezSdk sdk) async {
  // ANCHOR: buy-bitcoin-with-amount
  // Lock the purchase to a specific amount (e.g., 0.001 BTC = 100,000 sats)
  final request = BuyBitcoinRequest(
      address: "bc1qexample...",
      lockedAmountSat: 100000, // Pre-fill with 100,000 sats
      maxAmountSat: null,
      redirectUrl: null);

  final response = await sdk.buyBitcoin(request: request);
  print("Open this URL in a browser to complete the purchase:");
  print(response.url);
  // ANCHOR_END: buy-bitcoin-with-amount
}

Future<void> buyBitcoinWithLimits(BreezSdk sdk) async {
  // ANCHOR: buy-bitcoin-with-limits
  // Set both a locked amount and maximum amount
  final request = BuyBitcoinRequest(
      address: "bc1qexample...",
      lockedAmountSat: 50000, // Pre-fill with 50,000 sats
      maxAmountSat: 500000, // Limit to 500,000 sats max
      redirectUrl: null);

  final response = await sdk.buyBitcoin(request: request);
  print("Open this URL in a browser to complete the purchase:");
  print(response.url);
  // ANCHOR_END: buy-bitcoin-with-limits
}

Future<void> buyBitcoinWithRedirect(BreezSdk sdk) async {
  // ANCHOR: buy-bitcoin-with-redirect
  // Provide a custom redirect URL for after the purchase
  final request = BuyBitcoinRequest(
      address: "bc1qexample...",
      lockedAmountSat: 100000,
      maxAmountSat: null,
      redirectUrl: "https://example.com/purchase-complete");

  final response = await sdk.buyBitcoin(request: request);
  print("Open this URL in a browser to complete the purchase:");
  print(response.url);
  // ANCHOR_END: buy-bitcoin-with-redirect
}
