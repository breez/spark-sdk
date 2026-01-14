import 'package:breez_sdk_spark_flutter/breez_sdk_spark.dart';

Future<PrepareSendPaymentResponse> prepareSendPaymentLightningBolt11(
    BreezSdk sdk) async {
  // ANCHOR: prepare-send-payment-lightning-bolt11
  String paymentRequest = "<bolt11 invoice>";
  // Optionally set the amount you wish the pay the receiver
  BigInt optionalAmountSats = BigInt.from(5000);

  final request = PrepareSendPaymentRequest(
      paymentRequest: paymentRequest, amount: optionalAmountSats);
  final response = await sdk.prepareSendPayment(request: request);

  // If the fees are acceptable, continue to create the Send Payment
  final paymentMethod = response.paymentMethod;
  if (paymentMethod is SendPaymentMethod_Bolt11Invoice) {
    // Fees to pay via Lightning
    final lightningFeeSats = paymentMethod.lightningFeeSats;
    // Or fees to pay (if available) via a Spark transfer
    final sparkTransferFeeSats = paymentMethod.sparkTransferFeeSats;
    print("Lightning Fees: $lightningFeeSats sats");
    print("Spark Transfer Fees: $sparkTransferFeeSats sats");
  }
  // ANCHOR_END: prepare-send-payment-lightning-bolt11
  return response;
}

Future<PrepareSendPaymentResponse> prepareSendPaymentOnchain(
    BreezSdk sdk) async {
  // ANCHOR: prepare-send-payment-onchain
  String paymentRequest = "<bitcoin address>";
  // Set the amount you wish the pay the receiver
  BigInt amountSats = BigInt.from(50000);

  final request = PrepareSendPaymentRequest(
      paymentRequest: paymentRequest, amount: amountSats);
  final response = await sdk.prepareSendPayment(request: request);

  // If the fees are acceptable, continue to create the Send Payment
  final paymentMethod = response.paymentMethod;
  if (paymentMethod is SendPaymentMethod_BitcoinAddress) {
    final feeQuote = paymentMethod.feeQuote;
    final slowFeeSats =
        feeQuote.speedSlow.userFeeSat + feeQuote.speedSlow.l1BroadcastFeeSat;
    final mediumFeeSats = feeQuote.speedMedium.userFeeSat +
        feeQuote.speedMedium.l1BroadcastFeeSat;
    final fastFeeSats =
        feeQuote.speedFast.userFeeSat + feeQuote.speedFast.l1BroadcastFeeSat;
    print("Slow Fees: $slowFeeSats sats");
    print("Medium Fees: $mediumFeeSats sats");
    print("Fast Fees: $fastFeeSats sats");
  }
  // ANCHOR_END: prepare-send-payment-onchain
  return response;
}

Future<PrepareSendPaymentResponse> prepareSendPaymentSparkAddress(
    BreezSdk sdk) async {
  // ANCHOR: prepare-send-payment-spark-address
  String paymentRequest = "<spark address>";
  // Set the amount you wish the pay the receiver
  BigInt amountSats = BigInt.from(50000);

  final request = PrepareSendPaymentRequest(
      paymentRequest: paymentRequest, amount: amountSats);
  final response = await sdk.prepareSendPayment(request: request);

  // If the fees are acceptable, continue to create the Send Payment
  final paymentMethod = response.paymentMethod;
  if (paymentMethod is SendPaymentMethod_SparkAddress) {
    final feeSats = paymentMethod.fee;
    print("Fees: $feeSats sats");
  }
  // ANCHOR_END: prepare-send-payment-spark-address
  return response;
}

Future<PrepareSendPaymentResponse> prepareSendPaymentSparkInvoice(
    BreezSdk sdk) async {
  // ANCHOR: prepare-send-payment-spark-invoice
  String paymentRequest = "<spark invoice>";
  // Optionally set the amount you wish the pay the receiver
  BigInt optionalAmountSats = BigInt.from(50000);

  final request = PrepareSendPaymentRequest(
      paymentRequest: paymentRequest, amount: optionalAmountSats);
  final response = await sdk.prepareSendPayment(request: request);

  // If the fees are acceptable, continue to create the Send Payment
  final paymentMethod = response.paymentMethod;
  if (paymentMethod is SendPaymentMethod_SparkInvoice) {
    final feeSats = paymentMethod.fee;
    print("Fees: $feeSats sats");
  }
  // ANCHOR_END: prepare-send-payment-spark-invoice
  return response;
}

Future<PrepareSendPaymentResponse> prepareSendPaymentTokenConversion(
    BreezSdk sdk) async {
  // ANCHOR: prepare-send-payment-token-conversion
  String paymentRequest = "<payment request>";
  // Set to use token funds to pay via token conversion
  int optionalMaxSlippageBps = 50;
  int optionalCompletionTimeoutSecs = 30;
  final conversionOptions = ConversionOptions(
    conversionType: ConversionType.toBitcoin(
      fromTokenIdentifier: "<token identifier>",
    ),
    maxSlippageBps: optionalMaxSlippageBps,
    completionTimeoutSecs: optionalCompletionTimeoutSecs,
  );

  final request = PrepareSendPaymentRequest(
      paymentRequest: paymentRequest,
      conversionOptions: conversionOptions);
  final response = await sdk.prepareSendPayment(request: request);

  // If the fees are acceptable, continue to create the Send Payment
  if (response.conversionEstimate != null) {
    print(
        "Estimated conversion amount: ${response.conversionEstimate!.amount} token base units");
    print(
        "Estimated conversion fee: ${response.conversionEstimate!.fee} token base units");
  }
  // ANCHOR_END: prepare-send-payment-token-conversion
  return response;
}

Future<SendPaymentResponse> sendPaymentLightningBolt11(
    BreezSdk sdk, PrepareSendPaymentResponse prepareResponse) async {
  // ANCHOR: send-payment-lightning-bolt11
  final options = SendPaymentOptions.bolt11Invoice(
      preferSpark: false, completionTimeoutSecs: 10);
  String? optionalIdempotencyKey = "<idempotency key uuid>";
  final request = SendPaymentRequest(
      prepareResponse: prepareResponse,
      options: options,
      idempotencyKey: optionalIdempotencyKey);
  SendPaymentResponse response = await sdk.sendPayment(request: request);
  Payment payment = response.payment;
  // ANCHOR_END: send-payment-lightning-bolt11
  print(payment);
  return response;
}

Future<SendPaymentResponse> sendPaymentOnchain(
    BreezSdk sdk, PrepareSendPaymentResponse prepareResponse) async {
  // ANCHOR: send-payment-onchain
  final options = SendPaymentOptions.bitcoinAddress(
      confirmationSpeed: OnchainConfirmationSpeed.medium);
  String? optionalIdempotencyKey = "<idempotency key uuid>";
  final request = SendPaymentRequest(
      prepareResponse: prepareResponse,
      options: options,
      idempotencyKey: optionalIdempotencyKey);
  SendPaymentResponse response = await sdk.sendPayment(request: request);
  Payment payment = response.payment;
  // ANCHOR_END: send-payment-onchain
  print(payment);
  return response;
}

Future<SendPaymentResponse> sendPaymentSpark(
    BreezSdk sdk, PrepareSendPaymentResponse prepareResponse) async {
  // ANCHOR: send-payment-spark
  String? optionalIdempotencyKey = "<idempotency key uuid>";
  final request = SendPaymentRequest(
      prepareResponse: prepareResponse, idempotencyKey: optionalIdempotencyKey);
  SendPaymentResponse response = await sdk.sendPayment(request: request);
  Payment payment = response.payment;
  // ANCHOR_END: send-payment-spark
  print(payment);
  return response;
}
