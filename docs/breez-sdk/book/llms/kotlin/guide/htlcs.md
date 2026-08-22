# Conditional Payments

Conditional payments use Hash Time-Locked Contracts (HTLCs) to lock funds with a cryptographic hash of a secret preimage and an expiration time. The payment can only be claimed by revealing the preimage before expiration. If not claimed in time, the funds are automatically returned to the sender. This enables use cases like atomic cross-chain swaps.

The SDK supports both sending conditional payments via Spark HTLCs and receiving them via HODL invoices.

**Developer note**

Preimages are required to be unique and are not managed by the SDK. It is your responsibility as a developer to manage them, including how to generate them, store them, and provide them when claiming payments.

## Sending Spark HTLC payments

HTLC payments use the standard payment API described in [Sending payments](send_payment.md). To create an HTLC payment, prepare the payment normally, then provide the Spark HTLC options when [sending](send_payment.md#spark). These options include the payment hash (SHA-256 hash of the preimage) and the expiry duration.

```kotlin
val paymentRequest = "<spark address>"
// Set the amount you wish the pay the receiver
// Kotlin MPP (BigInteger from com.ionspin.kotlin.bignum.integer)
val amountSats = BigInteger.fromLong(50_000L)
// Android (BigInteger from java.math)
// val amountSats = BigInteger.valueOf(50_000L)
try {
    val prepareRequest = PrepareSendPaymentRequest(
        paymentRequest = PaymentRequest.Input(input = paymentRequest),
        amount = amountSats,
        tokenIdentifier = null,
        conversionOptions = null,
        feePolicy = null,
    )
    val prepareResponse = sdk.prepareSendPayment(prepareRequest)

    // If the fees are acceptable, continue to create the HTLC Payment
    val paymentMethod = prepareResponse.paymentMethod
    if (paymentMethod is SendPaymentMethod.SparkAddress) {
        val fee = paymentMethod.fee
        // Log.v("Breez", "Fees: ${fee} sats")
    }

    val preimage = "<32-byte unique preimage hex>"
    val preimageBytes = preimage.hexToByteArray()
    val digest = SHA256()
    digest.update(preimageBytes)
    val paymentHashBytes = digest.digest()
    val paymentHash = paymentHashBytes.toHexString()

    // Set the HTLC options
    val htlcOptions = SparkHtlcOptions(
        paymentHash = paymentHash,
        expiryDurationSecs = 1000u
    )
    val options = SendPaymentOptions.SparkAddress(htlcOptions = htlcOptions)

    val request = SendPaymentRequest(
        prepareResponse = prepareResponse,
        options = options
    )
    val sendResponse = sdk.sendPayment(request)
    val payment = sendResponse.payment
} catch (e: Exception) {
    // handle error
    throw e
}
```



## Receiving using HODL invoices

API docs: https://breez.github.io/spark-sdk/breez_sdk_spark/struct.BreezSdk.html#method.receive_payment

You can receive using HODL invoices — Lightning invoices where the payment is held until you claim it by revealing the preimage. To create one, provide a `paymentHash` when calling `receivePayment` with the `ReceivePaymentMethod.Bolt11Invoice` payment method.

```kotlin
try {
    val preimage = "<32-byte unique preimage hex>"
    val preimageBytes = preimage.hexToByteArray()
    val digest = SHA256()
    digest.update(preimageBytes)
    val paymentHashBytes = digest.digest()
    val paymentHash = paymentHashBytes.toHexString()

    val response = sdk.receivePayment(
        ReceivePaymentRequest(
            paymentMethod = ReceivePaymentMethod.Bolt11Invoice(
                description = "HODL invoice",
                amountSats = 50_000u,
                expirySecs = null,
                paymentHash = paymentHash
            )
        )
    )

    val invoice = response.paymentRequest
    // Log.v("Breez", "HODL invoice: $invoice")
} catch (e: Exception) {
    // handle error
    throw e
}
```



## Listing claimable conditional payments

API docs: https://breez.github.io/spark-sdk/breez_sdk_spark/struct.BreezSdk.html#method.list_payments

Once detected, claimable HTLC payments are immediately listed as pending in the [list of payments](/llms/kotlin/guide/list_payments.md). Additionally, a `SdkEvent.PaymentPending` event is emitted to notify your application. See [Listening to events](/llms/kotlin/guide/events.md) for more details.

To list only claimable HTLC payments, you can filter by HTLC status. This works for both Spark HTLC payments and HODL invoices.

```kotlin
try {
    val request = ListPaymentsRequest(
        typeFilter = listOf(PaymentType.RECEIVE),
        statusFilter = listOf(PaymentStatus.PENDING),
        paymentDetailsFilter = listOf(
            PaymentDetailsFilter.Spark(
                htlcStatus = listOf(SparkHtlcStatus.WAITING_FOR_PREIMAGE),
                conversionRefundNeeded = null
            ),
            PaymentDetailsFilter.Lightning(
                htlcStatus = listOf(SparkHtlcStatus.WAITING_FOR_PREIMAGE)
            )
        )
    )

    val response = sdk.listPayments(request)
    val payments = response.payments

    for (payment in payments) {
        val details = payment.details
        when (details) {
            is PaymentDetails.Spark -> {
                val htlc = details.htlcDetails
                if (htlc != null) {
                    // Log.v("Breez", "Spark HTLC expiry time: ${htlc.expiryTime}")
                }
            }
            is PaymentDetails.Lightning -> {
                val htlc = details.htlcDetails
                // Log.v("Breez", "Lightning HTLC expiry time: ${htlc.expiryTime}")
            }
            else -> {}
        }
    }
} catch (e: Exception) {
    // handle error
    throw e
}
```



## Claiming conditional payments

API docs: https://breez.github.io/spark-sdk/breez_sdk_spark/struct.BreezSdk.html#method.claim_htlc_payment

To claim an HTLC payment, provide the preimage that matches the payment hash. This works for both Spark HTLC payments and HODL invoices.

```kotlin
try {
    val preimage = "<preimage hex>"
    val request = ClaimHtlcPaymentRequest(preimage = preimage)
    val response = sdk.claimHtlcPayment(request)
    val payment = response.payment
} catch (e: Exception) {
    // handle error
    throw e
}
```
