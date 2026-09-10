# Sending and receiving tokens

Spark supports tokens using the [BTKN protocol](https://docs.spark.money/learn/tokens/hello-btkn). The Breez SDK enables you to send and receive these tokens using the standard payments API.

## Fetching token balances

API docs: https://breez.github.io/spark-sdk/breez_sdk_spark/struct.BreezSdk.html#method.get_info

Token balances for all tokens currently held in the wallet can be retrieved along with general wallet information. Each token balance includes both the balance amount and the token metadata (identifier, name, ticker, issuer public key, etc.).

```dart
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
```



**Developer note**

Token balances are cached for fast responses. For details on ensuring up-to-date balances, see the <a href="./get_info.md#fetching-the-balance">Fetching the balance</a> section.

## Fetching token metadata

API docs: https://breez.github.io/spark-sdk/breez_sdk_spark/struct.BreezSdk.html#method.get_tokens_metadata

Token metadata can be fetched for specific tokens by providing their identifiers. This is especially useful for retrieving metadata for tokens that are not currently held in the wallet. The metadata is cached locally after the first fetch for faster subsequent lookups.

```dart
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
```



## Receiving a token payment

API docs: https://breez.github.io/spark-sdk/breez_sdk_spark/struct.BreezSdk.html#method.receive_payment

Token payments can be received using either a Spark address or invoice. Using an invoice is useful to impose restrictions on the payment, such as the token to receive, amount, expiry, and who can pay it.

### Spark address

Token payments use the same Spark address as Bitcoin payments - no separate address is required. Your application can retrieve the Spark address as described in the [Receiving a payment](./receive_payment.md#spark) guide. The payer will use this address to send tokens to the wallet.

### Spark invoice

Spark token invoices can be created using the same API as Bitcoin Spark invoices. The only difference is that a token identifier is provided.

```dart
String tokenIdentifier = '<token identifier>';
String optionalDescription = "<invoice description>";
BigInt optionalAmount = BigInt.from(5000);
// Optionally set the expiry UNIX timestamp in seconds
BigInt optionalExpiryTimeSeconds = BigInt.from(1716691200);
String optionalSenderPublicKey = "<sender public key>"; 

ReceivePaymentRequest request =
    ReceivePaymentRequest(paymentMethod: ReceivePaymentMethod.sparkInvoice(
      tokenIdentifier: tokenIdentifier,
      description: optionalDescription,
      amount: optionalAmount,
      expiryTime: optionalExpiryTimeSeconds,
      senderPublicKey: optionalSenderPublicKey,
    ));
ReceivePaymentResponse response = await sdk.receivePayment(
  request: request,
);

String paymentRequest = response.paymentRequest;
print("Payment request: $paymentRequest");
BigInt receiveFee = response.fee;
print("Fees: $receiveFee token base units");
```



## Sending a token payment

API docs: https://breez.github.io/spark-sdk/breez_sdk_spark/struct.BreezSdk.html#method.prepare_send_payment

To send tokens, provide a Spark address as the payment request. The token identifier must be specified in one of two ways:

1. **Using a Spark invoice**: If the payee provides a Spark address with an embedded token identifier and amount (a Spark invoice), the SDK automatically extracts and uses those values.
2. **Manual specification**: For a plain Spark address without embedded payment details, your application must provide both the token identifier and amount parameters when preparing the payment.

Your application can use the [parse](./parse.md) functionality to determine if a Spark address contains embedded token payment details before preparing the payment.

The code example below demonstrates manual specification. Follow the standard prepare/send payment flow as described in the [Sending a payment](./send_payment.md) guide.

**Developer note**

Payments can be sent without holding an asset by converting on-the-fly as a step before sending a payment. See <a href="./token_conversion.md">Converting tokens</a> for more information.

```dart
final paymentRequest = '<spark address or invoice>';
// Token identifier must match the invoice in case it specifies one.
final tokenIdentifier = '<token identifier>';
// Set the amount of tokens you wish to send (in token base units).
final amount = BigInt.from(1000);

final prepareResponse = await sdk.prepareSendPayment(
  request: PrepareSendPaymentRequest(
    paymentRequest: PaymentRequest.input(input: paymentRequest),
    amount: amount,
    tokenIdentifier: tokenIdentifier,
    conversionOptions: null,
    feePolicy: null,
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
```



To pay several recipients at once, see [Sending to multiple recipients](./batch_send.md): one transaction can pay multiple payees, across several tokens, mixing Spark addresses and invoices. A batch that pays an invoice stays on one token.

## Listing token payments

API docs: https://breez.github.io/spark-sdk/breez_sdk_spark/struct.BreezSdk.html#method.list_payments

Token payments are included in the regular payment history alongside Bitcoin payments. Your application can retrieve and distinguish token payments from other payment types using the standard payment listing functionality. See the [Listing payments](./list_payments.md) guide for more details.
