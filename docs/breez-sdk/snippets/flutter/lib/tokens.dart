import 'package:breez_sdk_spark_flutter/breez_sdk_spark.dart';

Future<void> fetchTokenBalances(BreezSdk sdk) async {
  // ANCHOR: fetch-token-balances
  // ensureSynced: true will ensure the SDK is synced with the Spark network
  // before returning the balance
  final info = await sdk.getInfo(request: GetInfoRequest(ensureSynced: false));
  
  // Token balances are a map of token identifier to balance
  final tokenBalances = info.tokenBalances;
  tokenBalances.forEach((tokenId, tokenBalance) {
    print('Token ID: $tokenId');
    print('Balance: ${tokenBalance.balance}');
    print('Name: ${tokenBalance.tokenMetadata.name}');
    print('Ticker: ${tokenBalance.tokenMetadata.ticker}');
    print('Decimals: ${tokenBalance.tokenMetadata.decimals}');
  });
  // ANCHOR_END: fetch-token-balances
}

Future<void> sendTokenPayment(BreezSdk sdk) async {
  // ANCHOR: send-token-payment
  final paymentRequest = '<spark address>';
  final tokenIdentifier = '<token identifier>';
  // Set the amount of tokens you wish to send
  final amount = BigInt.from(1000);
  
  final prepareResponse = await sdk.prepareSendPayment(
    request: PrepareSendPaymentRequest(
      paymentRequest: paymentRequest,
      amount: amount,
      tokenIdentifier: tokenIdentifier,
    ),
  );
  
  // If the fees are acceptable, continue to send the token payment
  if (prepareResponse.paymentMethod is SendPaymentMethod_SparkAddress) {
    final method = prepareResponse.paymentMethod as SendPaymentMethod_SparkAddress;
    print('Token ID: ${method.tokenIdentifier}');
    print('Fees: ${method.fee} sats');
  }

  // Send the token payment
  final sendResponse = await sdk.sendPayment(
    request: SendPaymentRequest(
      prepareResponse: prepareResponse,
      options: null,
    ),
  );
  final payment = sendResponse.payment;
  print('Payment: $payment');
  // ANCHOR_END: send-token-payment
}

