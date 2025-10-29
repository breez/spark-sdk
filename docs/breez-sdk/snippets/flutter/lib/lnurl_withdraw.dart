import 'package:breez_sdk_spark_flutter/breez_sdk_spark.dart';

Future<void> lnurlWithdraw(BreezSdk sdk) async {
  // ANCHOR: lnurl-withdraw
  /// Endpoint can also be of the form:
  /// lnurlw://domain.com/lnurl-withdraw?key=val
  String lnurlWithdrawUrl =
      "lnurl1dp68gurn8ghj7mr0vdskc6r0wd6z7mrww4exctthd96xserjv9mn7um9wdekjmmw843xxwpexdnxzen9vgunsvfexq6rvdecx93rgdmyxcuxverrvcursenpxvukzv3c8qunsdecx33nzwpnvg6ryc3hv93nzvecxgcxgwp3h33lxk";

  InputType inputType = await sdk.parse(input: lnurlWithdrawUrl);
  if (inputType is InputType_LnurlWithdraw) {
    // Amount to withdraw in sats between min/max withdrawable amounts
    BigInt amountSats = BigInt.from(5000);
    LnurlWithdrawRequestDetails withdrawRequest = inputType.field0;
    int optionalCompletionTimeoutSecs = 30;
    
    LnurlWithdrawRequest request = LnurlWithdrawRequest(
      amountSats: amountSats,
      withdrawRequest: withdrawRequest,
      completionTimeoutSecs: optionalCompletionTimeoutSecs,
    );

    LnurlWithdrawResponse response = await sdk.lnurlWithdraw(request: request);

    Payment? payment = response.payment;
    print('Payment: $payment');
  }
  // ANCHOR_END: lnurl-withdraw
}
