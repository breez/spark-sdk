# Conditional Payments

Conditional payments use Hash Time-Locked Contracts (HTLCs) to lock funds with a cryptographic hash of a secret preimage and an expiration time. The payment can only be claimed by revealing the preimage before expiration. If not claimed in time, the funds are automatically returned to the sender. This enables use cases like atomic cross-chain swaps.

The SDK supports both sending conditional payments via Spark HTLCs and receiving them via HODL invoices.

**Developer note**

Preimages are required to be unique and are not managed by the SDK. It is your responsibility as a developer to manage them, including how to generate them, store them, and provide them when claiming payments.

## Sending Spark HTLC payments

HTLC payments use the standard payment API described in [Sending payments](send_payment.md). To create an HTLC payment, prepare the payment normally, then provide the Spark HTLC options when [sending](send_payment.md#spark). These options include the payment hash (SHA-256 hash of the preimage) and the expiry duration.

```csharp
var paymentRequest = "<spark address>";
// Set the amount you wish the pay the receiver
ulong? amountSats = 50_000UL;
var prepareRequest = new PrepareSendPaymentRequest(
    paymentRequest: new PaymentRequest.Input(input: paymentRequest),
    amount: amountSats,
    tokenIdentifier: null,
    conversionOptions: null,
    feePolicy: null
);
var prepareResponse = await sdk.PrepareSendPayment(request: prepareRequest);

// If the fees are acceptable, continue to create the HTLC Payment
if (prepareResponse.paymentMethod is SendPaymentMethod.SparkAddress sparkMethod)
{
    var fee = sparkMethod.fee;
    Console.WriteLine($"Fees: {fee} sats");
}

var preimage = "<32-byte unique preimage hex>";
var preimageBytes = Convert.FromHexString(preimage);
var paymentHashBytes = System.Security.Cryptography.SHA256.HashData(preimageBytes);
var paymentHash = Convert.ToHexString(paymentHashBytes).ToLower();

// Set the HTLC options
var options = new SendPaymentOptions.SparkAddress(
    htlcOptions: new SparkHtlcOptions(
        paymentHash: paymentHash,
        expiryDurationSecs: 1000
    )
);

var request = new SendPaymentRequest(
    prepareResponse: prepareResponse,
    options: options
);
var sendResponse = await sdk.SendPayment(request: request);
var payment = sendResponse.payment;
```



## Receiving using HODL invoices

API docs: https://breez.github.io/spark-sdk/breez_sdk_spark/struct.BreezSdk.html#method.receive_payment

You can receive using HODL invoices — Lightning invoices where the payment is held until you claim it by revealing the preimage. To create one, provide a `PaymentHash` when calling `ReceivePayment` with the `ReceivePaymentMethod.Bolt11Invoice` payment method.

```csharp
var preimage = "<32-byte unique preimage hex>";
var preimageBytes = Convert.FromHexString(preimage);
var paymentHashBytes = System.Security.Cryptography.SHA256.HashData(preimageBytes);
var paymentHash = Convert.ToHexString(paymentHashBytes).ToLower();

var response = await sdk.ReceivePayment(
    request: new ReceivePaymentRequest(
        paymentMethod: new ReceivePaymentMethod.Bolt11Invoice(
            description: "HODL invoice",
            amountSats: 50_000UL,
            expirySecs: null,
            paymentHash: paymentHash,
            receiverIdentityPublicKey: null
        )
    )
);

var invoice = response.paymentRequest;
Console.WriteLine($"HODL invoice: {invoice}");
```



## Listing claimable conditional payments

API docs: https://breez.github.io/spark-sdk/breez_sdk_spark/struct.BreezSdk.html#method.list_payments

Once detected, claimable HTLC payments are immediately listed as pending in the [list of payments](/llms/csharp/guide/list_payments.md). Additionally, a `SdkEvent.PaymentPending` event is emitted to notify your application. See [Listening to events](/llms/csharp/guide/events.md) for more details.

To list only claimable HTLC payments, you can filter by HTLC status. This works for both Spark HTLC payments and HODL invoices.

```csharp
var request = new ListPaymentsRequest(
    typeFilter: new PaymentType[] { PaymentType.Receive },
    statusFilter: new PaymentStatus[] { PaymentStatus.Pending },
    paymentDetailsFilter: new PaymentDetailsFilter[] {
        new PaymentDetailsFilter.Spark(
            htlcStatus: new SparkHtlcStatus[] {
                SparkHtlcStatus.WaitingForPreimage
            },
            conversionRefundNeeded: null
        ),
        new PaymentDetailsFilter.Lightning(
            htlcStatus: new SparkHtlcStatus[] {
                SparkHtlcStatus.WaitingForPreimage
            }
        )
    }
);

var response = await sdk.ListPayments(request: request);
var payments = response.payments;

foreach (var payment in payments)
{
    if (payment.details is PaymentDetails.Spark sparkDetails && sparkDetails.htlcDetails != null)
    {
        Console.WriteLine($"Spark HTLC expiry time: {sparkDetails.htlcDetails.expiryTime}");
    }
    else if (payment.details is PaymentDetails.Lightning lightningDetails)
    {
        Console.WriteLine($"Lightning HTLC expiry time: {lightningDetails.htlcDetails.expiryTime}");
    }
}
```



## Claiming conditional payments

API docs: https://breez.github.io/spark-sdk/breez_sdk_spark/struct.BreezSdk.html#method.claim_htlc_payment

To claim an HTLC payment, provide the preimage that matches the payment hash. This works for both Spark HTLC payments and HODL invoices.

```csharp
var preimage = "<preimage hex>";
var response = await sdk.ClaimHtlcPayment(
    request: new ClaimHtlcPaymentRequest(preimage: preimage)
);
var payment = response.payment;
```
