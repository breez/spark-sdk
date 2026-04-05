import 'package:breez_sdk_spark_flutter/breez_sdk_spark.dart';

Future<void> prepareLnurlPay(BreezSdk sdk) async {
  // ANCHOR: prepare-lnurl-pay
  // Endpoint can also be of the form:
  // lnurlp://domain.com/lnurl-pay?key=val
  // lnurl1dp68gurn8ghj7mr0vdskc6r0wd6z7mrww4excttsv9un7um9wdekjmmw84jxywf5x43rvv35xgmr2enrxanr2cfcvsmnwe3jxcukvde48qukgdec89snwde3vfjxvepjxpjnjvtpxd3kvdnxx5crxwpjvyunsephsz36jf
  String lnurlPayUrl = "lightning@address.com";

  InputType inputType = await sdk.parse(input: lnurlPayUrl);
  if (inputType is InputType_LightningAddress) {
    BigInt amountSats = BigInt.from(5000);
    String optionalComment = "<comment>";
    bool optionalValidateSuccessActionUrl = true;

    PrepareLnurlPayRequest request = PrepareLnurlPayRequest(
      amount: amountSats,
      payRequest: inputType.field0.payRequest,
      comment: optionalComment,
      validateSuccessActionUrl: optionalValidateSuccessActionUrl,
      tokenIdentifier: null,
      conversionOptions: null,
      feePolicy: null,
    );
    PrepareLnurlPayResponse prepareResponse =
        await sdk.prepareLnurlPay(request: request);

    // If the fees are acceptable, continue to create the LNURL Pay
    BigInt feeSats = prepareResponse.feeSats;
    print("Fees: $feeSats sats");
  }
  // ANCHOR_END: prepare-lnurl-pay
}

Future<void> lnurlPay(
    BreezSdk sdk, PrepareLnurlPayResponse prepareResponse) async {
  // ANCHOR: lnurl-pay
  String? optionalIdempotencyKey = "<idempotency key uuid>";
  LnurlPayResponse response = await sdk.lnurlPay(
    request: LnurlPayRequest(
      prepareResponse: prepareResponse,
      idempotencyKey: optionalIdempotencyKey),
  );
  // ANCHOR_END: lnurl-pay
  print(response);
}

Future<void> prepareLnurlPayFeesIncluded(BreezSdk sdk, LnurlPayRequestDetails payRequest) async {
  // ANCHOR: prepare-lnurl-pay-fees-included
  // By default (FeePolicy.feesExcluded), fees are added on top of the amount.
  // Use FeePolicy.feesIncluded to deduct fees from the amount instead.
  // The receiver gets amount minus fees.
  String optionalComment = "<comment>";
  bool optionalValidateSuccessActionUrl = true;
  BigInt amountSats = BigInt.from(5000);

  PrepareLnurlPayRequest request = PrepareLnurlPayRequest(
    amount: amountSats,
    payRequest: payRequest,
    comment: optionalComment,
    validateSuccessActionUrl: optionalValidateSuccessActionUrl,
    tokenIdentifier: null,
    conversionOptions: null,
    feePolicy: FeePolicy.feesIncluded,
  );
  PrepareLnurlPayResponse prepareResponse =
      await sdk.prepareLnurlPay(request: request);

  // If the fees are acceptable, continue to create the LNURL Pay
  BigInt feeSats = prepareResponse.feeSats;
  print("Fees: $feeSats sats");
  // The receiver gets amountSats - feeSats
  // ANCHOR_END: prepare-lnurl-pay-fees-included
}
