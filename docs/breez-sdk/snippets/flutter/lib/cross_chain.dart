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
