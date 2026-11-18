import 'package:breez_sdk_spark_flutter/breez_sdk_spark.dart';

Future<BridgingClient> newClient() async {
  // ANCHOR: new-bridging-client
  BridgingConfig config = defaultBridgingConfig(apiKey: "<your api key>");
  BridgingClient client = await newBridgingClient(config: config);
  // ANCHOR_END: new-bridging-client
  return client;
}

BridgingClient clientFromSdk(BreezSdk sdk) {
  // ANCHOR: get-bridging-client
  BridgingClient client = sdk.getBridgingClient();
  // ANCHOR_END: get-bridging-client
  return client;
}

Future<void> createBridge(BridgingClient client) async {
  // ANCHOR: create-bridge
  // Who to pay: a Spark address, a Lightning address or an EVM/Solana/Tron
  // address.
  String recipient = "<recipient>";

  // List the routes that pay them and pick what the payer pays in, e.g.
  // USDC on Base.
  final routesResponse = await client.getRoutes(
    request: GetBridgeRoutesRequest(recipient: recipient),
  );
  BridgeRoutePair route = routesResponse.routes.firstWhere(
    (r) => r.source.asset == "USDC" && r.source.chain == "base",
  );

  // Ask for $10. The amount is in the recipient's stablecoin when they are
  // paid in one, the payer's otherwise (6 decimals for USDC). The deposit is
  // refunded to the payer's address if the recipient can't be paid.
  final response = await client.createBridge(
    request: CreateBridgeRequest(
      recipient: recipient,
      route: route,
      amount: BigInt.from(10000000),
      // Required to pay a Lightning address.
      refundAddress: "<payer address on Base>",
      feePolicy: null,
      maxSlippageBps: null,
    ),
  );

  // Show the payer what to pay, and store the bridge id and status token to
  // look up the status later.
  print("Pay with: ${response.paymentRequest}");
  print("Bridge id: ${response.bridgeId}");
  // ANCHOR_END: create-bridge
}

Future<void> getBridgeStatus(
  BridgingClient client,
  String bridgeId,
  String? statusToken,
) async {
  // ANCHOR: get-bridge-status
  final response = await client.getBridgeStatus(
    request: GetBridgeStatusRequest(
      bridgeId: bridgeId,
      statusToken: statusToken,
    ),
  );
  switch (response.status) {
    case BridgeStatus.completed:
      print("Delivered ${response.amountOut}");
      break;
    case BridgeStatus.refunded:
    case BridgeStatus.failed:
    case BridgeStatus.expired:
      print("Not delivered: ${response.status}");
      break;
    default:
      print("Not final yet: ${response.status}");
  }
  // ANCHOR_END: get-bridge-status
}
