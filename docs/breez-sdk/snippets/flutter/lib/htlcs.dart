import 'package:convert/convert.dart';
import 'package:breez_sdk_spark_flutter/breez_sdk_spark.dart';
import 'package:crypto/crypto.dart';

Future<Payment> sendHtlcPayment(BreezSdk sdk) async {
  // ANCHOR: send-htlc-payment
  String paymentRequest = "<spark address>";
  // Set the amount you wish the pay the receiver
  BigInt? amountSats = BigInt.from(50000);
  final prepareRequest = PrepareSendPaymentRequest(
      paymentRequest: paymentRequest,
      amount: amountSats,
      tokenIdentifier: null,
      conversionOptions: null,
      feePolicy: null);
  final prepareResponse = await sdk.prepareSendPayment(request: prepareRequest);

  // If the fees are acceptable, continue to create the HTLC Payment
  final paymentMethod = prepareResponse.paymentMethod;
  if (paymentMethod is SendPaymentMethod_SparkAddress) {
    final fee = paymentMethod.fee;
    print("Fees: $fee sats");
  }

  String preimage = "<32-byte unique preimage hex>";
  List<int> preimageBytes = hex.decode(preimage);
  Digest paymentHashDigest = sha256.convert(preimageBytes);
  String paymentHash = hex.encode(paymentHashDigest.bytes);

  // Set the HTLC options
  final htlcOptions = SparkHtlcOptions(
      paymentHash: paymentHash, expiryDurationSecs: BigInt.from(1000));
  final options = SendPaymentOptions.sparkAddress(htlcOptions: htlcOptions);

  final request =
      SendPaymentRequest(prepareResponse: prepareResponse, options: options);
  final sendResponse = await sdk.sendPayment(request: request);
  final payment = sendResponse.payment;
  // ANCHOR_END: send-htlc-payment
  return payment;
}

Future<List<Payment>> listClaimableHtlcPayments(BreezSdk sdk) async {
  // ANCHOR: list-claimable-htlc-payments
  final request = ListPaymentsRequest(
    typeFilter: [PaymentType.receive],
    statusFilter: [PaymentStatus.pending],
    paymentDetailsFilter: [PaymentDetailsFilter.spark(
      htlcStatus: [SparkHtlcStatus.waitingForPreimage],
    )],
  );

  final response = await sdk.listPayments(request: request);
  final payments = response.payments;
  // ANCHOR_END: list-claimable-htlc-payments
  return payments;
}

Future<Payment> claimHtlcPayment(BreezSdk sdk) async {
  // ANCHOR: claim-htlc-payment
  String preimage = "<preimage hex>";
  final response = await sdk.claimHtlcPayment(
      request: ClaimHtlcPaymentRequest(preimage: preimage));
  final payment = response.payment;
  // ANCHOR_END: claim-htlc-payment
  return payment;
}
