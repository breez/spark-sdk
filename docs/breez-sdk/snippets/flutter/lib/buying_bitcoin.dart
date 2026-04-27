import 'package:breez_sdk_spark_flutter/breez_sdk_spark.dart';

Future<void> buyBitcoin(BreezSdk sdk) async {
  // ANCHOR: buy-bitcoin
  // Optionally, lock the purchase to a specific amount
  final optionalLockedAmountSat = BigInt.from(100000);
  // Optionally, set a redirect URL for after the purchase is completed
  final optionalRedirectUrl = "https://example.com/purchase-complete";

  final request = BuyBitcoinRequest_Moonpay(
      lockedAmountSat: optionalLockedAmountSat,
      redirectUrl: optionalRedirectUrl);

  final response = await sdk.buyBitcoin(request: request);
  print("Open this URL in a browser to complete the purchase:");
  print(response.url);
  // ANCHOR_END: buy-bitcoin
}

Future<void> buyBitcoinViaCashapp(BreezSdk sdk) async {
  // ANCHOR: buy-bitcoin-cashapp
  // Cash App requires the amount to be specified up front.
  final amountSats = BigInt.from(50000);

  final request = BuyBitcoinRequest_CashApp(amountSats: amountSats);

  final response = await sdk.buyBitcoin(request: request);
  print("Open this URL in Cash App to complete the purchase:");
  print(response.url);
  // ANCHOR_END: buy-bitcoin-cashapp
}
