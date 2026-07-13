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
  // With the default FeesExcluded mode, amount is the receiver's net target
  // on Spark in destination-asset base units (sats for BTC, token base units
  // for USDB). The SDK pads the sender's deposit to cover fees + overpay.
  // With FeesIncluded, amount is the sender's deposit in source-asset units.
  final amount = BigInt.from(1000);
  // Optionally set the destination Spark-side asset. null = auto: active
  // stable-balance token if the route supports it, otherwise BTC.
  SparkAsset? optionalDestination;
  // Optionally set the maximum slippage in basis points (10 to 500)
  int? optionalMaxSlippageBps = 100;
  // Optionally override the overpay buffer (0 to 500 bps). Defaults to 15.
  int? optionalTargetOverpayBps;
  // Optionally override the fee mode. Defaults to FeesExcluded.
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
