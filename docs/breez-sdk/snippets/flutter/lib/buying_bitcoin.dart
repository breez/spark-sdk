import 'package:breez_sdk_spark_flutter/breez_sdk_spark.dart';

Future<void> buyBitcoin(BreezSdk sdk) async {
  // ANCHOR: buy-bitcoin
  // Optionally, prefill the purchase amount
  BigInt? optionalAmountSat = BigInt.from(100000);
  // Optionally, set a redirect URL for after the purchase is completed
  String? optionalRedirectUrl = "https://example.com/purchase-complete";

  final request = BuyBitcoinRequest_Moonpay(
    delivery: MoonpayDelivery_Bitcoin(amountSat: optionalAmountSat),
    redirectUrl: optionalRedirectUrl,
  );

  final response = await sdk.buyBitcoin(request: request);
  print("Open this URL in a browser to complete the purchase:");
  print(response.url);
  // ANCHOR_END: buy-bitcoin
}

Future<void> buyBitcoinViaCrossChain(BreezSdk sdk) async {
  // ANCHOR: buy-bitcoin-cross-chain
  // USD amount to receive, in 6-decimal base units ($50)
  final amount = BigInt.from(50000000);

  final request = BuyBitcoinRequest_Moonpay(
    delivery: MoonpayDelivery_CrossChain(amount: amount, feeMode: null),
    redirectUrl: null,
  );

  final response = await sdk.buyBitcoin(request: request);
  print("Open this URL in a browser to complete the purchase:");
  print(response.url);

  final info = response.crossChainInfo;
  if (info != null) {
    print("USDC to buy: ${info.depositAmount}");
    print(
      "Expected to receive: "
      "${info.expectedReceivedAmount} ${info.destinationAsset}",
    );
    print("Conversion fee: ${info.serviceFeeAmount}");
  }
  // ANCHOR_END: buy-bitcoin-cross-chain
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
