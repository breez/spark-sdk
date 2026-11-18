import 'package:breez_sdk_spark_flutter/breez_sdk_spark.dart';

Future<void> bridgeFromCashApp(BridgingClient client) async {
  // ANCHOR: bridge-from-cash-app
  // List the routes to the recipient's external-chain address
  // (EVM/Solana/Tron) and pick a stablecoin Cash App can fund over
  // Lightning, e.g. USDC on Base.
  String recipient = "<recipient address>";
  final routesResponse = await client.getRoutes(
    request: GetBridgeRoutesRequest(recipient: recipient),
  );
  BridgeRoutePair route = routesResponse.routes.firstWhere(
    (r) =>
        r.source.chain == "lightning" &&
        r.destination.asset == "USDC" &&
        r.destination.chain == "base",
  );

  // Send $10 of USDC, funded by Cash App over Lightning. The amount is in
  // USDC base units (6 decimals), so 10000000 = 10 USDC, about $10.
  final response = await client.bridgeFromCashApp(
    request: CreateBridgeRequest(
      recipient: recipient,
      route: route,
      amount: BigInt.from(10000000),
      refundAddress: null,
      feePolicy: null,
      maxSlippageBps: null,
    ),
  );

  // Open this Cash App link to pay. The recipient then receives the stablecoin.
  final info = response.info;
  print("Open this URL in Cash App: ${response.paymentRequest}");
  print(
    "Recipient receives ~${info.expectedReceivedAmount} ${info.destinationAsset}",
  );
  // ANCHOR_END: bridge-from-cash-app
}

Future<void> bridgeToCashApp(BridgingClient client) async {
  // ANCHOR: bridge-to-cash-app
  // List the routes to the Cash App user and pick what the payer pays in,
  // e.g. USDC on Base.
  String recipient = "\$alice";
  final routesResponse = await client.getRoutes(
    request: GetBridgeRoutesRequest(recipient: recipient),
  );
  BridgeRoutePair route = routesResponse.routes.firstWhere(
    (r) => r.source.asset == "USDC" && r.source.chain == "base",
  );

  // Pay $10 of USDC to the Cash App user $alice. The amount is in USDC base
  // units (6 decimals), so 10000000 = 10 USDC, about $10. The deposit is
  // refunded to the payer's address if delivery fails.
  final response = await client.bridgeToCashApp(
    request: CreateBridgeRequest(
      recipient: recipient,
      route: route,
      amount: BigInt.from(10000000),
      refundAddress: "<payer address>",
      feePolicy: null,
      maxSlippageBps: null,
    ),
  );

  // Show the payer what to pay. The recipient then receives Bitcoin.
  final info = response.info;
  print("Pay with: ${response.paymentRequest}");
  print(
    "Deposit ${info.depositAmount} to ${info.depositAddress},"
    " recipient receives ~${info.expectedReceivedAmount} sats",
  );
  // ANCHOR_END: bridge-to-cash-app
}
