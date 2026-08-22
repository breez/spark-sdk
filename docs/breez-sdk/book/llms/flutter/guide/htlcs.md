# Conditional Payments

Conditional payments use Hash Time-Locked Contracts (HTLCs) to lock funds with a cryptographic hash of a secret preimage and an expiration time. The payment can only be claimed by revealing the preimage before expiration. If not claimed in time, the funds are automatically returned to the sender. This enables use cases like atomic cross-chain swaps.

The SDK supports both sending conditional payments via Spark HTLCs and receiving them via HODL invoices.

**Developer note**

Preimages are required to be unique and are not managed by the SDK. It is your responsibility as a developer to manage them, including how to generate them, store them, and provide them when claiming payments.

## Sending Spark HTLC payments

HTLC payments use the standard payment API described in [Sending payments](send_payment.md). To create an HTLC payment, prepare the payment normally, then provide the Spark HTLC options when [sending](send_payment.md#spark). These options include the payment hash (SHA-256 hash of the preimage) and the expiry duration.

```dart
String paymentRequest = "<spark address>";
// Set the amount you wish the pay the receiver
BigInt? amountSats = BigInt.from(50000);
final prepareRequest = PrepareSendPaymentRequest(
    paymentRequest: PaymentRequest.input(input: paymentRequest),
    amount: amountSats,
    tokenIdentifier: null,
    conversionOptions: null,
    feePolicy: null);
final prepareResponse = await sdk.prepareSendPayment(request: prepareRequest);

// If the fees are acceptable, continue to create the HTLC Payment
final paymentMethod = prepareResponse.paymentMethod;
if (paymentMethod is SendPaymentMethod_SparkAddress) {
  final fee = paymentMethod.fee;
  print("Fees: $fee sats");
}

String preimage = "<32-byte unique preimage hex>";
List<int> preimageBytes = hex.decode(preimage);
Digest paymentHashDigest = sha256.convert(preimageBytes);
String paymentHash = hex.encode(paymentHashDigest.bytes);

// Set the HTLC options
final htlcOptions = SparkHtlcOptions(
    paymentHash: paymentHash, expiryDurationSecs: BigInt.from(1000));
final options = SendPaymentOptions.sparkAddress(htlcOptions: htlcOptions);

final request =
    SendPaymentRequest(prepareResponse: prepareResponse, options: options);
final sendResponse = await sdk.sendPayment(request: request);
final payment = sendResponse.payment;
```



## Receiving using HODL invoices

API docs: https://breez.github.io/spark-sdk/breez_sdk_spark/struct.BreezSdk.html#method.receive_payment

You can receive using HODL invoices — Lightning invoices where the payment is held until you claim it by revealing the preimage. To create one, provide a `paymentHash` when calling `receivePayment` with the `ReceivePaymentMethod.Bolt11Invoice` payment method.

```dart
String preimage = "<32-byte unique preimage hex>";
List<int> preimageBytes = hex.decode(preimage);
Digest paymentHashDigest = sha256.convert(preimageBytes);
String paymentHash = hex.encode(paymentHashDigest.bytes);

final response = await sdk.receivePayment(
    request: ReceivePaymentRequest(
        paymentMethod: ReceivePaymentMethod.bolt11Invoice(
            description: "HODL invoice",
            amountSats: BigInt.from(50000),
            expirySecs: null,
            paymentHash: paymentHash)));

final invoice = response.paymentRequest;
print("HODL invoice: $invoice");
```



## Listing claimable conditional payments

API docs: https://breez.github.io/spark-sdk/breez_sdk_spark/struct.BreezSdk.html#method.list_payments

Once detected, claimable HTLC payments are immediately listed as pending in the [list of payments](/llms/flutter/guide/list_payments.md). Additionally, a `SdkEvent.PaymentPending` event is emitted to notify your application. See [Listening to events](/llms/flutter/guide/events.md) for more details.

To list only claimable HTLC payments, you can filter by HTLC status. This works for both Spark HTLC payments and HODL invoices.

```dart
final request = ListPaymentsRequest(
  typeFilter: [PaymentType.receive],
  statusFilter: [PaymentStatus.pending],
  paymentDetailsFilter: [
    PaymentDetailsFilter.spark(
      htlcStatus: [SparkHtlcStatus.waitingForPreimage],
    ),
    PaymentDetailsFilter.lightning(
      htlcStatus: [SparkHtlcStatus.waitingForPreimage],
    ),
  ],
);

final response = await sdk.listPayments(request: request);
final payments = response.payments;

for (final payment in payments) {
  final details = payment.details;
  if (details is PaymentDetails_Spark && details.htlcDetails != null) {
    print("Spark HTLC expiry time: ${details.htlcDetails!.expiryTime}");
  } else if (details is PaymentDetails_Lightning) {
    print("Lightning HTLC expiry time: ${details.htlcDetails.expiryTime}");
  }
}
```



## Claiming conditional payments

API docs: https://breez.github.io/spark-sdk/breez_sdk_spark/struct.BreezSdk.html#method.claim_htlc_payment

To claim an HTLC payment, provide the preimage that matches the payment hash. This works for both Spark HTLC payments and HODL invoices.

```dart
String preimage = "<preimage hex>";
final response = await sdk.claimHtlcPayment(
    request: ClaimHtlcPaymentRequest(preimage: preimage));
final payment = response.payment;
```
