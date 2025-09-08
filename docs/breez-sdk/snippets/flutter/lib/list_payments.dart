import 'package:breez_sdk_spark_flutter/breez_sdk_spark.dart';

Future<Payment> getPayment(BreezSdk sdk) async {
  // ANCHOR: get-payment
  String paymentId = "<payment id>";
  GetPaymentRequest request = GetPaymentRequest(paymentId: paymentId);
  GetPaymentResponse response = await sdk.getPayment(request: request);
  Payment payment = response.payment;
  // ANCHOR_END: get-payment
  return payment;
}

Future<List<Payment>> listPayments(BreezSdk sdk) async {
  // ANCHOR: list-payments
  ListPaymentsRequest request = ListPaymentsRequest();
  ListPaymentsResponse response = await sdk.listPayments(request: request);
  List<Payment> payments = response.payments;
  // ANCHOR_END: list-payments
  return payments;
}

Future<List<Payment>> listPaymentsFiltered(BreezSdk sdk) async {
  // ANCHOR: list-payments-filtered
  ListPaymentsRequest request = ListPaymentsRequest(
    offset: 0,
    limit: 50,
  );
  ListPaymentsResponse response = await sdk.listPayments(request: request);
  List<Payment> payments = response.payments;
  // ANCHOR_END: list-payments-filtered
  return payments;
}
