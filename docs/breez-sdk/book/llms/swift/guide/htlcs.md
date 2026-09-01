# Conditional Payments

Conditional payments use Hash Time-Locked Contracts (HTLCs) to lock funds with a cryptographic hash of a secret preimage and an expiration time. The payment can only be claimed by revealing the preimage before expiration. If not claimed in time, the funds are automatically returned to the sender. This enables use cases like atomic cross-chain swaps.

The SDK supports both sending conditional payments via Spark HTLCs and receiving them via HODL invoices.

**Developer note**

Preimages are required to be unique and are not managed by the SDK. It is your responsibility as a developer to manage them, including how to generate them, store them, and provide them when claiming payments.

## Sending Spark HTLC payments

HTLC payments use the standard payment API described in [Sending payments](send_payment.md). To create an HTLC payment, prepare the payment normally, then provide the Spark HTLC options when [sending](send_payment.md#spark). These options include the payment hash (SHA-256 hash of the preimage) and the expiry duration.

```swift
let paymentRequest = "<spark address>"
// Set the amount you wish to pay the receiver
let amountSats = BInt(50_000)
let prepareRequest = PrepareSendPaymentRequest(
    paymentRequest: .input(input: paymentRequest),
    amount: amountSats,
    tokenIdentifier: nil,
    conversionOptions: nil,
    feePolicy: nil
)
let prepareResponse = try await sdk.prepareSendPayment(request: prepareRequest)

// If the fees are acceptable, continue to create the HTLC Payment
if case let .sparkAddress(_, fee, _) = prepareResponse.paymentMethod {
    print("Fees: \(fee) sats")
}

let preimage = "<32-byte unique preimage hex>"
let preimageData = Data(hexString: preimage)!
let paymentHashDigest = SHA256.hash(data: preimageData)
let paymentHash = Data(paymentHashDigest).hexEncodedString()

// Set the HTLC options
let htlcOptions = SparkHtlcOptions(
    paymentHash: paymentHash,
    expiryDurationSecs: 1000
)
let options = SendPaymentOptions.sparkAddress(htlcOptions: htlcOptions)

let request = SendPaymentRequest(
    prepareResponse: prepareResponse,
    options: options
)
let sendResponse = try await sdk.sendPayment(request: request)
let payment = sendResponse.payment
```



## Receiving using HODL invoices

API docs: https://breez.github.io/spark-sdk/breez_sdk_spark/struct.BreezSdk.html#method.receive_payment

You can receive using HODL invoices — Lightning invoices where the payment is held until you claim it by revealing the preimage. To create one, provide a `paymentHash` when calling `receivePayment` with the `ReceivePaymentMethod.bolt11Invoice` payment method.

```swift
let preimage = "<32-byte unique preimage hex>"
let preimageData = Data(hexString: preimage)!
let paymentHashDigest = SHA256.hash(data: preimageData)
let paymentHash = Data(paymentHashDigest).hexEncodedString()

let response = try await sdk.receivePayment(
    request: ReceivePaymentRequest(
        paymentMethod: ReceivePaymentMethod.bolt11Invoice(
            description: "HODL invoice",
            amountSats: 50_000,
            expirySecs: nil,
            paymentHash: paymentHash,
            receiverIdentityPublicKey: nil
        )
    )
)

let invoice = response.paymentRequest
print("HODL invoice: \(invoice)")
```



## Listing claimable conditional payments

API docs: https://breez.github.io/spark-sdk/breez_sdk_spark/struct.BreezSdk.html#method.list_payments

Once detected, claimable HTLC payments are immediately listed as pending in the [list of payments](/llms/swift/guide/list_payments.md). Additionally, a `SdkEvent.paymentPending` event is emitted to notify your application. See [Listening to events](/llms/swift/guide/events.md) for more details.

To list only claimable HTLC payments, you can filter by HTLC status. This works for both Spark HTLC payments and HODL invoices.

```swift
let request = ListPaymentsRequest(
    typeFilter: [PaymentType.receive],
    statusFilter: [PaymentStatus.pending],
    paymentDetailsFilter: [
        PaymentDetailsFilter.spark(
            htlcStatus: [SparkHtlcStatus.waitingForPreimage],
            conversionRefundNeeded: nil
        ),
        PaymentDetailsFilter.lightning(
            htlcStatus: [SparkHtlcStatus.waitingForPreimage]
        ),
    ]
)

let response = try await sdk.listPayments(request: request)
let payments = response.payments

for payment in payments {
    if case let .spark(_, htlcDetails, _) = payment.details, let htlc = htlcDetails {
        print("Spark HTLC expiry time: \(htlc.expiryTime)")
    } else if case let .lightning(_, _, _, htlcDetails, _, _, _, _) = payment.details {
        print("Lightning HTLC expiry time: \(htlcDetails.expiryTime)")
    }
}
```



## Claiming conditional payments

API docs: https://breez.github.io/spark-sdk/breez_sdk_spark/struct.BreezSdk.html#method.claim_htlc_payment

To claim an HTLC payment, provide the preimage that matches the payment hash. This works for both Spark HTLC payments and HODL invoices.

```swift
let preimage = "<preimage hex>"
let response = try await sdk.claimHtlcPayment(
    request: ClaimHtlcPaymentRequest(preimage: preimage)
)
let payment = response.payment
```
