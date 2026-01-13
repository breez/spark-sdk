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

Future<void> fetchTokenMetadata(BreezSdk sdk) async {
  // ANCHOR: fetch-token-metadata
  final response = await sdk.getTokensMetadata(
    request: GetTokensMetadataRequest(
      tokenIdentifiers: ['<token identifier 1>', '<token identifier 2>']
      )
    );
  
  final tokensMetadata = response.tokensMetadata;
  for (final tokenMetadata in tokensMetadata) {
    print('Token ID: $tokenMetadata.identifier');
    print('Name: ${tokenMetadata.name}');
    print('Ticker: ${tokenMetadata.ticker}');
    print('Decimals: ${tokenMetadata.decimals}');
    print('Max Supply: ${tokenMetadata.maxSupply}');
    print('Is Freezable: ${tokenMetadata.isFreezable}');
  }
  // ANCHOR_END: fetch-token-metadata
}

Future<ReceivePaymentResponse> receiveTokenPaymentSparkInvoice(BreezSdk sdk) async {
  // ANCHOR: receive-token-payment-spark-invoice
  String tokenIdentifier = '<token identifier>';
  String optionalDescription = "<invoice description>";
  BigInt optionalAmount = BigInt.from(5000);
  BigInt optionalExpiresAt = BigInt.from(1716691200);
  String optionalSenderPublicKey = "<sender public key>";

  ReceivePaymentRequest request =
      ReceivePaymentRequest(paymentMethod: ReceivePaymentMethod.sparkInvoice(
        tokenIdentifier: tokenIdentifier,
        description: optionalDescription,
        amount: optionalAmount,
        expiresAt: optionalExpiresAt,
        senderPublicKey: optionalSenderPublicKey,
      ));
  ReceivePaymentResponse response = await sdk.receivePayment(
    request: request,
  );

  String paymentRequest = response.paymentRequest;
  print("Payment request: $paymentRequest");
  BigInt receiveFee = response.fee;
  print("Fees: $receiveFee token base units");
  // ANCHOR_END: receive-token-payment-spark-invoice
  return response;
}

Future<void> sendTokenPayment(BreezSdk sdk) async {
  // ANCHOR: send-token-payment
  final paymentRequest = '<spark address or invoice>';
  // Token identifier must match the invoice in case it specifies one.
  final tokenIdentifier = '<token identifier>';
  // Set the amount of tokens you wish to send.
  final optionalAmount = BigInt.from(1000);

  final prepareResponse = await sdk.prepareSendPayment(
    request: PrepareSendPaymentRequest(
      paymentRequest: paymentRequest,
      amount: optionalAmount,
      tokenIdentifier: tokenIdentifier,
    ),
  );
  
  // If the fees are acceptable, continue to send the token payment
  if (prepareResponse.paymentMethod is SendPaymentMethod_SparkAddress) {
    final method = prepareResponse.paymentMethod as SendPaymentMethod_SparkAddress;
    print('Token ID: ${method.tokenIdentifier}');
    print('Fees: ${method.fee} token base units');
  }
  if (prepareResponse.paymentMethod is SendPaymentMethod_SparkInvoice) {
    final method = prepareResponse.paymentMethod as SendPaymentMethod_SparkInvoice;
    print('Token ID: ${method.tokenIdentifier}');
    print('Fees: ${method.fee} token base units');
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

Future<void> fetchTokenConversionLimits(BreezSdk sdk) async {
  // ANCHOR: fetch-token-conversion-limits
  // Fetch limits for converting Bitcoin to a token
  final fromBitcoinResponse = await sdk.fetchTokenConversionLimits(
    request: FetchTokenConversionLimitsRequest(
      conversionType: TokenConversionType.fromBitcoin(),
      tokenIdentifier: '<token identifier>',
    ),
  );

  if (fromBitcoinResponse.minFromAmount != null) {
    print('Minimum BTC to convert: ${fromBitcoinResponse.minFromAmount} sats');
  }
  if (fromBitcoinResponse.minToAmount != null) {
    print('Minimum tokens to receive: ${fromBitcoinResponse.minToAmount} base units');
  }

  // Fetch limits for converting a token to Bitcoin
  final toBitcoinResponse = await sdk.fetchTokenConversionLimits(
    request: FetchTokenConversionLimitsRequest(
      conversionType: TokenConversionType.toBitcoin(
        fromTokenIdentifier: '<token identifier>',
      ),
      tokenIdentifier: null,
    ),
  );

  if (toBitcoinResponse.minFromAmount != null) {
    print('Minimum tokens to convert: ${toBitcoinResponse.minFromAmount} base units');
  }
  if (toBitcoinResponse.minToAmount != null) {
    print('Minimum BTC to receive: ${toBitcoinResponse.minToAmount} sats');
  }
  // ANCHOR_END: fetch-token-conversion-limits
}

Future<void> prepareSendPaymentTokenConversion(BreezSdk sdk) async {
  // ANCHOR: prepare-send-payment-token-conversion
  final paymentRequest = '<spark address or invoice>';
  // Token identifier must match the invoice in case it specifies one.
  final tokenIdentifier = '<token identifier>';
  // Set the amount of tokens you wish to send.
  final optionalAmount = BigInt.from(1000);
  // Set to use Bitcoin funds to pay via token conversion
  int optionalMaxSlippageBps = 50;
  int optionalCompletionTimeoutSecs = 30;
  final tokenConversionOptions = TokenConversionOptions(
    conversionType: TokenConversionType.fromBitcoin(),
    maxSlippageBps: optionalMaxSlippageBps,
    completionTimeoutSecs: optionalCompletionTimeoutSecs,
  );

  final prepareResponse = await sdk.prepareSendPayment(
    request: PrepareSendPaymentRequest(
      paymentRequest: paymentRequest,
      amount: optionalAmount,
      tokenIdentifier: tokenIdentifier,
      tokenConversionOptions: tokenConversionOptions
    ),
  );
  
  // If the fees are acceptable, continue to send the token payment
  if (prepareResponse.tokenConversionFee != null) {
    print(
        "Estimated token conversion fee: ${prepareResponse.tokenConversionFee} sats");
  }
  // ANCHOR_END: prepare-send-payment-token-conversion
}
