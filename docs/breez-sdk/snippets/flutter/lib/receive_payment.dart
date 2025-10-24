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
  BigInt receiveFeeSats = response.fee;
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
  BigInt receiveFeeSats = response.fee;
  print("Fees: $receiveFeeSats sats");
  // ANCHOR_END: receive-payment-onchain
  return response;
}

Future<ReceivePaymentResponse> receivePaymentSparkAddress(BreezSdk sdk) async {
  // ANCHOR: receive-payment-spark-address
  ReceivePaymentRequest request =
      ReceivePaymentRequest(paymentMethod: ReceivePaymentMethod.sparkAddress());
  ReceivePaymentResponse response = await sdk.receivePayment(
    request: request,
  );

  String paymentRequest = response.paymentRequest;
  print("Payment request: $paymentRequest");
  BigInt receiveFeeSats = response.fee;
  print("Fees: $receiveFeeSats sats");
  // ANCHOR_END: receive-payment-spark-address
  return response;
}

Future<ReceivePaymentResponse> receivePaymentSparkInvoice(BreezSdk sdk) async {
  // ANCHOR: receive-payment-spark-invoice
  String optionalDescription = "<invoice description>";
  BigInt optionalAmountSats = BigInt.from(5000);
  BigInt optionalExpiryTimeSeconds = BigInt.from(1716691200);
  String optionalSenderPublicKey = "<sender public key>";

  ReceivePaymentRequest request =
      ReceivePaymentRequest(paymentMethod: ReceivePaymentMethod.sparkInvoice(
        description: optionalDescription,
        amount: optionalAmountSats,
        expiryTime: optionalExpiryTimeSeconds,
        senderPublicKey: optionalSenderPublicKey,
      ));
  ReceivePaymentResponse response = await sdk.receivePayment(
    request: request,
  );

  String paymentRequest = response.paymentRequest;
  print("Payment request: $paymentRequest");
  BigInt receiveFeeSats = response.fee;
  print("Fees: $receiveFeeSats sats");
  // ANCHOR_END: receive-payment-spark-invoice
  return response;
}

Future<void> waitForPayment(BreezSdk sdk) async {
  // ANCHOR: wait-for-payment
  // Waiting for a payment given its payment request (Bolt11 or Spark invoice)
  String paymentRequest = "<Bolt11 or Spark invoice>"; 

  // Wait for a payment to be completed using a payment request
  WaitForPaymentResponse paymentRequestResponse = await sdk.waitForPayment(
    request: WaitForPaymentRequest(
      identifier: WaitForPaymentIdentifier.paymentRequest(paymentRequest),
    ),
  );
  print("Payment received with ID: ${paymentRequestResponse.payment.id}");

  // Waiting for a payment given its payment id
  String paymentId = "<payment id>";

  // Wait for a payment to be completed using a payment request
  WaitForPaymentResponse paymentIdResponse = await sdk.waitForPayment(
    request: WaitForPaymentRequest(
      identifier: WaitForPaymentIdentifier.paymentId(paymentId),
    ),
  );
  
  print("Payment received with ID: ${paymentIdResponse.payment.id}"); 
  // ANCHOR_END: wait-for-payment
}
