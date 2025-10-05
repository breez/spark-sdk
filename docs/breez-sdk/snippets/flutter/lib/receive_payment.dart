import 'package:breez_sdk_spark_flutter/breez_sdk_spark.dart';

Future<ReceivePaymentResponse> receivePaymentLightning(
    BreezSdk sdk) async {
  // ANCHOR: receive-payment-lightning-bolt11
  String description = "<invoice description>";
  // Optionally set the invoice amount you wish the payer to send
  BigInt optionalAmountSats = BigInt.from(5000);

  // Create an invoice and set the amount you wish the payer to send
  ReceivePaymentRequest request = ReceivePaymentRequest(
      paymentMethod: ReceivePaymentMethod.bolt11Invoice(
          description: description, amountSats: optionalAmountSats));
  ReceivePaymentResponse response = await sdk.receivePayment(
    request: request,
  );

  String paymentRequest = response.paymentRequest;
  print("Payment request: $paymentRequest");
  BigInt receiveFeeSats = response.feeSats;
  print("Fees: $receiveFeeSats sats");
  // ANCHOR_END: receive-payment-lightning-bolt11
  return response;
}

Future<ReceivePaymentResponse> receivePaymentOnchain(
    BreezSdk sdk) async {
  // ANCHOR: receive-payment-onchain
  ReceivePaymentRequest request = ReceivePaymentRequest(
      paymentMethod: ReceivePaymentMethod.bitcoinAddress());
  ReceivePaymentResponse response = await sdk.receivePayment(
    request: request,
  );

  String paymentRequest = response.paymentRequest;
  print("Payment request: $paymentRequest");
  BigInt receiveFeeSats = response.feeSats;
  print("Fees: $receiveFeeSats sats");
  // ANCHOR_END: receive-payment-onchain
  return response;
}

Future<ReceivePaymentResponse> receivePaymentSpark(BreezSdk sdk) async {
  // ANCHOR: receive-payment-spark
  ReceivePaymentRequest request =
      ReceivePaymentRequest(paymentMethod: ReceivePaymentMethod.sparkAddress());
  ReceivePaymentResponse response = await sdk.receivePayment(
    request: request,
  );

  String paymentRequest = response.paymentRequest;
  print("Payment request: $paymentRequest");
  BigInt receiveFeeSats = response.feeSats;
  print("Fees: $receiveFeeSats sats");
  // ANCHOR_END: receive-payment-spark
  return response;
}

Future<Payment> waitForPayment(BreezSdk sdk, String paymentRequest) async {
  // ANCHOR: wait-for-payment
  // Wait for a payment to be completed using a payment request
  WaitForPaymentResponse response = await sdk.waitForPayment(
    request: WaitForPaymentRequest(
      identifier: WaitForPaymentIdentifier.paymentRequest(paymentRequest),
    ),
  );
  
  print("Payment received with ID: ${response.payment.id}");
  // ANCHOR_END: wait-for-payment
  return response.payment;
}
