import 'package:breez_sdk_spark_flutter/breez_sdk_spark.dart';

Future<void> preparePaymentLinkViaCashapp(BreezSdk sdk) async {
  // ANCHOR: prepare-payment-link-cashapp
  // Parse the recipient's external-chain address (EVM/Solana/Tron).
  String input = "<recipient address>";
  InputType parsed = await sdk.parse(input: input);
  if (parsed is! InputType_CrossChainAddress) {
    throw Exception("Not a cross-chain address");
  }
  CrossChainAddressDetails addressDetails = parsed.field0;

  // List the stablecoin destinations you can send to and pick one, e.g. USDC
  // on Base. These routes are funded by an external rail, not the Spark wallet.
  // Restrict to Lightning-fundable routes to match the Cash App flow below.
  List<CrossChainRoutePair> routes = await sdk.getCrossChainRoutes(
    filter: CrossChainRouteFilter_PaymentLink(
      addressDetails: addressDetails,
    ),
  );
  CrossChainRoutePair route =
      routes.firstWhere((r) => r.asset == "USDC" && r.chain == "base");

  // Send $10 of USDC, funded by Cash App over Lightning. The amount is in the
  // route asset's base units (USDC, 6 decimals), so 10000000 = 10 USDC,
  // about $10.
  final response = await sdk.preparePaymentLink(
    request: PreparePaymentLinkRequest(
      address: addressDetails.address,
      route: route,
      amount: BigInt.from(10000000),
      feePolicy: null,
      maxSlippageBps: null,
    ),
  );

  // Open this Cash App URL to pay; the recipient then receives the stablecoin.
  print("Open this URL in Cash App: ${response.url}");
  print("Recipient receives ~${response.estimatedOut} ${response.asset}");
  // ANCHOR_END: prepare-payment-link-cashapp
}
