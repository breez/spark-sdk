import 'package:breez_sdk_spark_flutter/breez_sdk_spark.dart';

Future<PrepareSendPaymentResponse> prepareSendPaymentLightningBolt11(
    BreezSdk sdk) async {
  // ANCHOR: prepare-send-payment-lightning-bolt11
  String paymentRequest = "<bolt11 invoice>";
  // Optionally set the amount you wish the pay the receiver
  BigInt optionalAmountSats = BigInt.from(5000);

  final request = PrepareSendPaymentRequest(
      paymentRequest: paymentRequest, amountSats: optionalAmountSats);
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
      paymentRequest: paymentRequest, amountSats: amountSats);
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

Future<PrepareSendPaymentResponse> prepareSendPaymentSpark(BreezSdk sdk) async {
  // ANCHOR: prepare-send-payment-spark
  String paymentRequest = "<spark address>";
  // Set the amount you wish the pay the receiver
  BigInt amountSats = BigInt.from(50000);

  final request = PrepareSendPaymentRequest(
      paymentRequest: paymentRequest, amountSats: amountSats);
  final response = await sdk.prepareSendPayment(request: request);

  // If the fees are acceptable, continue to create the Send Payment
  final paymentMethod = response.paymentMethod;
  if (paymentMethod is SendPaymentMethod_SparkAddress) {
    final feeSats = paymentMethod.feeSats;
    print("Fees: $feeSats sats");
  }
  // ANCHOR_END: prepare-send-payment-spark
  return response;
}

Future<SendPaymentResponse> sendPaymentLightningBolt11(
    BreezSdk sdk, PrepareSendPaymentResponse prepareResponse) async {
  // ANCHOR: send-payment-lightning-bolt11
  final options = SendPaymentOptions.bolt11Invoice(
    preferSpark: true,
    returnPendingAfterSecs: 0,
  );
  final request =
      SendPaymentRequest(prepareResponse: prepareResponse, options: options);
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
  final request =
      SendPaymentRequest(prepareResponse: prepareResponse, options: options);
  SendPaymentResponse response = await sdk.sendPayment(request: request);
  Payment payment = response.payment;
  // ANCHOR_END: send-payment-onchain
  print(payment);
  return response;
}

Future<SendPaymentResponse> sendPaymentSpark(
    BreezSdk sdk, PrepareSendPaymentResponse prepareResponse) async {
  // ANCHOR: send-payment-spark
  final request = SendPaymentRequest(prepareResponse: prepareResponse);
  SendPaymentResponse response = await sdk.sendPayment(request: request);
  Payment payment = response.payment;
  // ANCHOR_END: send-payment-spark
  print(payment);
  return response;
}
