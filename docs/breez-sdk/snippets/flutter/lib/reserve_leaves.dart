import 'package:breez_sdk_spark_flutter/breez_sdk_spark.dart';

Future<PrepareSendPaymentResponse> prepareSendPaymentReserveLeaves(
    BreezSdk sdk) async {
  // ANCHOR: prepare-send-payment-reserve-leaves
  String paymentRequest = "<payment request>";
  BigInt? amountSats = BigInt.from(50000);

  final prepareResponse = await sdk.prepareSendPayment(
      request: PrepareSendPaymentRequest(
          paymentRequest: paymentRequest,
          amount: amountSats,
          tokenIdentifier: null,
          conversionOptions: null,
          feePolicy: null,
          reserveLeaves: true));

  // The reservation ID can be used to cancel the reservation if needed
  final reservationId = prepareResponse.reservationId;
  if (reservationId != null) {
    print("Reservation ID: $reservationId");
  }

  // Send payment as usual using the prepare response
  // sdk.sendPayment(request: SendPaymentRequest(prepareResponse: prepareResponse, options: null, idempotencyKey: null));
  // ANCHOR_END: prepare-send-payment-reserve-leaves
  return prepareResponse;
}

Future<void> cancelPrepareSendPayment(BreezSdk sdk) async {
  // ANCHOR: cancel-prepare-send-payment
  String reservationId = "<reservation id from prepare response>";

  await sdk.cancelPrepareSendPayment(
      request: CancelPrepareSendPaymentRequest(
          reservationId: reservationId));
  // ANCHOR_END: cancel-prepare-send-payment
}
