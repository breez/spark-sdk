import 'package:breez_sdk_spark_flutter/breez_sdk_spark.dart';

Future<void> createTokenAllowance(BreezSdk sdk) async {
  // ANCHOR: create-token-allowance
  final response = await sdk.createTokenAllowance(
      request: CreateTokenAllowanceRequest(
          spenderPublicKey: "<spender identity public key>",
          tokenIdentifier: "<token identifier>",
          maxPerPayment: TokenAllowanceLimit.amount(amount: BigInt.from(5000000)),
          maxTotal: TokenAllowanceLimit.amount(amount: BigInt.from(100000000)),
          expiryTime: BigInt.from(1798761600),
          allowedRecipients: []));
  print("Allowance id: ${response.allowance.id}");
  // ANCHOR_END: create-token-allowance
}

Future<void> listTokenAllowances(BreezSdk sdk) async {
  // ANCHOR: list-token-allowances
  final response = await sdk.listTokenAllowances(
      request: ListTokenAllowancesRequest(
          role: TokenAllowanceRole.owner,
          counterpartyPublicKey: null,
          tokenIdentifier: null,
          includeInactive: false,
          offset: null,
          limit: null));
  for (final allowance in response.allowances) {
    print("${allowance.id}: spent ${allowance.spentAmount}");
  }
  // ANCHOR_END: list-token-allowances
}

Future<void> revokeTokenAllowance(BreezSdk sdk) async {
  // ANCHOR: revoke-token-allowance
  await sdk.revokeTokenAllowance(
      request: RevokeTokenAllowanceRequest(allowanceId: "<allowance id>"));
  // ANCHOR_END: revoke-token-allowance
}

Future<void> pullPayment(BreezSdk sdk) async {
  // ANCHOR: pull-payment
  final prepareResponse = await sdk.preparePullPayment(
      request: PreparePullPaymentRequest(
          payerPublicKey: "<payer identity public key>",
          tokenIdentifier: "<token identifier>",
          receivers: [
        PullReceiver(amount: BigInt.from(5000000), receiverPublicKey: null)
      ]));
  print("Pulling ${prepareResponse.amount}");

  final response = await sdk.pullPayment(
      request: PullPaymentRequest(prepareResponse: prepareResponse));
  print("Pull transaction: ${response.txHash}");
  // ANCHOR_END: pull-payment
}
