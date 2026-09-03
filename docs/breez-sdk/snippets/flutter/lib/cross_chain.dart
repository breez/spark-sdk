import 'package:breez_sdk_spark_flutter/breez_sdk_spark.dart';

Future<List<CrossChainRoutePair>> getCrossChainRoutes(BreezSdk sdk) async {
  // ANCHOR: cross-chain-get-routes
  String input = "<recipient address>";
  InputType parsed = await sdk.parse(input: input);
  if (parsed is! InputType_CrossChainAddress) {
    throw Exception("Not a cross-chain address");
  }
  CrossChainAddressDetails addressDetails = parsed.field0;

  List<CrossChainRoutePair> routes = await sdk.getCrossChainRoutes(
    filter: CrossChainRouteFilter.send(addressDetails: addressDetails),
  );

  for (var route in routes) {
    print("Route via ${route.provider}: ${route.chain}/${route.asset}");
  }
  // ANCHOR_END: cross-chain-get-routes
  return routes;
}

Future<PrepareSendPaymentResponse> prepareSendPaymentCrossChain(
  BreezSdk sdk,
  CrossChainAddressDetails addressDetails,
  CrossChainRoutePair route,
) async {
  // ANCHOR: cross-chain-prepare
  // Optionally set the maximum slippage in basis points (10 to 500)
  int? optionalMaxSlippageBps = 100;

  final request = PrepareSendPaymentRequest(
    paymentRequest: PaymentRequest.crossChain(
      address: addressDetails.address,
      route: route,
      maxSlippageBps: optionalMaxSlippageBps,
      targetOverpayBps: null,
    ),
    amount: BigInt.from(50000),
    tokenIdentifier: null,
    conversionOptions: null,
    feePolicy: null,
  );
  final response = await sdk.prepareSendPayment(request: request);

  final paymentMethod = response.paymentMethod;
  if (paymentMethod is SendPaymentMethod_CrossChainAddress) {
    print("Amount in: ${paymentMethod.amountIn}");
    print("Estimated out: ${paymentMethod.estimatedOut}");
    print("Provider fee: ${paymentMethod.feeAmount}");
    print("Quote expires at: ${paymentMethod.expiresAt}");
  }
  // ANCHOR_END: cross-chain-prepare
  return response;
}

Future<SendPaymentResponse> sendPaymentCrossChain(
  BreezSdk sdk,
  PrepareSendPaymentResponse prepareResponse,
) async {
  // ANCHOR: cross-chain-send
  // Only valid for sends with no token leg (see Retry safety).
  String? optionalIdempotencyKey = "<idempotency key uuid>";
  final request = SendPaymentRequest(
    prepareResponse: prepareResponse,
    options: null,
    idempotencyKey: optionalIdempotencyKey,
  );
  final response = await sdk.sendPayment(request: request);
  print("Payment: ${response.payment}");
  // ANCHOR_END: cross-chain-send
  return response;
}

Future<List<CrossChainRoutePair>> getCrossChainReceiveRoutes(BreezSdk sdk) async {
  // ANCHOR: cross-chain-get-receive-routes
  List<CrossChainRoutePair> routes = await sdk.getCrossChainRoutes(
    filter: CrossChainRouteFilter.receive(contractAddress: null),
  );

  for (var route in routes) {
    print(
      "Route via ${route.provider}: ${route.chain}/${route.asset} -> Spark",
    );
  }
  // ANCHOR_END: cross-chain-get-receive-routes
  return routes;
}

Future<ReceivePaymentResponse> receivePaymentCrossChain(
  BreezSdk sdk,
  CrossChainRoutePair route,
) async {
  // ANCHOR: cross-chain-receive
  // amount is in the route's source-asset base units (USD-stable parity:
  // 1_000_000 = $1 on 6-decimal routes). See the guide for feeMode,
  // destination, and the slippage/overpay overrides.
  final amount = BigInt.from(1000000);
  SparkAsset? optionalDestination;
  int? optionalMaxSlippageBps = 100;
  int? optionalTargetOverpayBps;
  CrossChainFeeMode? optionalFeeMode;

  final request = ReceivePaymentRequest(
    paymentMethod: ReceivePaymentMethod.crossChain(
      route: route,
      amount: amount,
      destination: optionalDestination,
      feeMode: optionalFeeMode,
      maxSlippageBps: optionalMaxSlippageBps,
      targetOverpayBps: optionalTargetOverpayBps,
    ),
  );
  final response = await sdk.receivePayment(request: request);

  print("Payment request: ${response.paymentRequest}");
  final info = response.crossChainInfo;
  if (info != null) {
    final denom = info.tokenIdentifier != null ? "USDB" : "BTC";
    print("Deposit address: ${info.depositAddress}");
    print("Deposit amount: ${info.depositAmount}");
    print("Expected received: ${info.expectedReceivedAmount} $denom");
    print("Expires at: ${info.expiresAt}");
  }
  // ANCHOR_END: cross-chain-receive
  return response;
}
