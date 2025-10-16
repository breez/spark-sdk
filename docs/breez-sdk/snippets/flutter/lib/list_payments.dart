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
  // Filter by asset (Bitcoin or Token)
  AssetFilter assetFilter = AssetFilter.token(tokenIdentifier: "token_identifier_here");
  // To filter by Bitcoin instead:
  // AssetFilter assetFilter = AssetFilter.bitcoin();

  ListPaymentsRequest request = ListPaymentsRequest(
    // Filter by payment type
    typeFilter: [PaymentType.send, PaymentType.receive],
    // Filter by status
    statusFilter: [PaymentStatus.completed],
    assetFilter: assetFilter,
    // Time range filters
    fromTimestamp: BigInt.from(1704067200), // Unix timestamp
    toTimestamp: BigInt.from(1735689600),   // Unix timestamp
    // Pagination
    offset: 0,
    limit: 50,
    // Sort order (true = oldest first, false = newest first)
    sortAscending: false,
  );
  ListPaymentsResponse response = await sdk.listPayments(request: request);
  List<Payment> payments = response.payments;
  // ANCHOR_END: list-payments-filtered
  return payments;
}
