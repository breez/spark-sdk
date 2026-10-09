import 'package:breez_sdk_spark_flutter/breez_sdk_spark.dart';

Future<void> bridgeFromCashApp(BreezSdk sdk) async {
  // ANCHOR: bridge-from-cash-app
  // Parse the recipient's external-chain address (EVM/Solana/Tron).
  String input = "<recipient address>";
  InputType parsed = await sdk.parse(input: input);
  if (parsed is! InputType_CrossChainAddress) {
    throw Exception("Not a cross-chain address");
  }
  CrossChainAddressDetails addressDetails = parsed.field0;

  // List the stablecoin destinations Cash App can fund over Lightning and
  // pick one, e.g. USDC on Base.
  List<CrossChainRoutePair> routes = await sdk.getCrossChainRoutes(
    filter: CrossChainRouteFilter.send(
      addressDetails: addressDetails,
      deliveryMethod: DeliveryMethod.lightning,
    ),
  );
  CrossChainRoutePair route =
      routes.firstWhere((r) => r.asset == "USDC" && r.chain == "base");

  // Send $10 of USDC, funded by Cash App over Lightning. The amount is in the
  // route asset's base units (USDC, 6 decimals), so 10000000 = 10 USDC,
  // about $10.
  final response = await sdk.bridgeFromCashApp(
    request: BridgeFromCashAppRequest(
      address: addressDetails.address,
      route: route,
      amount: BigInt.from(10000000),
      feePolicy: null,
      maxSlippageBps: null,
    ),
  );

  // Open this Cash App URL to pay. The recipient then receives the stablecoin.
  print("Open this URL in Cash App: ${response.url}");
  print("Recipient receives ~${response.estimatedOut} ${response.asset}");
  // ANCHOR_END: bridge-from-cash-app
}

Future<void> bridgeToCashApp(BreezSdk sdk) async {
  // ANCHOR: bridge-to-cash-app
  // List the stablecoin sources that can pay a Cash App user over Lightning
  // and pick one, e.g. USDC on Base.
  List<CrossChainRoutePair> routes = await sdk.getCrossChainRoutes(
    filter: CrossChainRouteFilter.receive(
      contractAddress: null,
      deliveryMethod: DeliveryMethod.lightning,
    ),
  );
  CrossChainRoutePair route =
      routes.firstWhere((r) => r.asset == "USDC" && r.chain == "base");

  // Pay $10 of USDC to the Cash App user $alice. The amount is in the route
  // asset's base units (USDC, 6 decimals), so 10000000 = 10 USDC, about $10.
  // The deposit is refunded to the payer's address if delivery fails.
  final response = await sdk.bridgeToCashApp(
    request: BridgeToCashAppRequest(
      recipient: "\$alice",
      route: route,
      amount: BigInt.from(10000000),
      feePolicy: null,
      refundAddress: "<payer address>",
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
