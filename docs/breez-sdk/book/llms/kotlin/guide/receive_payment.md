# Receiving payments

API docs: https://breez.github.io/spark-sdk/breez_sdk_spark/struct.BreezSdk.html#method.receive_payment

Once the SDK is initialized, you can directly begin receiving payments. The SDK currently supports three methods of receiving: Lightning, Bitcoin and Spark.

## Lightning

#### BOLT11 invoice

When receiving via Lightning, we can generate a BOLT11 invoice to be paid. Setting the invoice amount fixes the amount the sender should pay.

To create an invoice for another Spark wallet, set `receiverIdentityPublicKey` to that wallet's identity public key. Creating the invoice requires only the receiver's public key, not their private keys.

**Note:** the payment may fallback to a direct Spark payment (if the payer's client supports this).

```kotlin
try {
    val description = "<invoice description>"
    // Optionally set the invoice amount you wish the payer to send
    val optionalAmountSats = 5_000.toULong()
    // Optionally set the expiry duration in seconds
    val optionalExpirySecs = 3600.toUInt()
    val optionalReceiverIdentityPublicKey: String? = null

    val request = ReceivePaymentRequest(
        ReceivePaymentMethod.Bolt11Invoice(
            description,
            optionalAmountSats,
            optionalExpirySecs,
            null,
            optionalReceiverIdentityPublicKey
        )
    )
    val response = sdk.receivePayment(request)

    val paymentRequest = response.paymentRequest
    // Log.v("Breez", "Payment Request: ${paymentRequest}")
    val receiveFeeSats = response.fee
    // Log.v("Breez", "Fees: ${receiveFeeSats} sats")
} catch (e: Exception) {
    // handle error
}
```



#### LNURL-Pay & Lightning address

To receive via LNURL-Pay and/or a Lightning address, follow [these instructions](/llms/kotlin/guide/receive_lnurl_pay.md).

> Note: Lightning payments work in Spark even if the receiver is offline. To understand how it works under the hood, read [this](https://docs.spark.money/learn/lightning).

## Bitcoin

For on-chain payments you can generate a Bitcoin deposit address to receive payments. By default the existing address is returned; you can optionally request a new address to rotate to a fresh one for improved privacy. All previously generated addresses remain monitored.

On-chain deposits go through the following lifecycle:

1. **Detected** — The SDK detects the deposit and emits a `SdkEvent.NewDeposits` event. The deposit may or may not have sufficient confirmations to be claimed yet.
2. **Sufficient confirmations** — After **3 on-chain confirmations**, the deposit has sufficient confirmations and the SDK automatically attempts to claim it.
3. **Claimed or unclaimed** — If claiming succeeds, the funds are added to your balance. If it fails (e.g. fees too high), the deposit remains unclaimed and can be [manually claimed or refunded](/llms/kotlin/guide/onchain_claims.md).

```kotlin
try {
    val newAddress: Boolean? = null // Set to true to get a new address
    val request = ReceivePaymentRequest(
        ReceivePaymentMethod.BitcoinAddress(newAddress = newAddress)
    )
    val response = sdk.receivePayment(request)

    val paymentRequest = response.paymentRequest
    // Log.v("Breez", "Payment Request: ${paymentRequest}")
    val receiveFeeSats = response.fee
    // Log.v("Breez", "Fees: ${receiveFeeSats} sats")
} catch (e: Exception) {
    // handle error
}
```



To track pending deposits, use `listUnclaimedDeposits` and filter by the `isMature` field:

```kotlin
try {
    val request = ListUnclaimedDepositsRequest
    val response = sdk.listUnclaimedDeposits(request)

    val pendingDeposits = response.deposits.filter { !it.isMature }

    for (deposit in pendingDeposits) {
        // Log.v("Breez", "Pending deposit: ${deposit.txid}:${deposit.vout}")
        // Log.v("Breez", "Amount: ${deposit.amountSats} sats")
    }
} catch (e: Exception) {
    // handle error
}
```



## Spark

For payments between Spark users, you can use a Spark address or generate a Spark invoice to receive payments.

#### Spark address

Spark addresses are static.

```kotlin
try {
    val request = ReceivePaymentRequest(ReceivePaymentMethod.SparkAddress)
    val response = sdk.receivePayment(request)

    val paymentRequest = response.paymentRequest
    // Log.v("Breez", "Payment Request: ${paymentRequest}")
    val receiveFeeSats = response.fee
    // Log.v("Breez", "Fees: ${receiveFeeSats} sats")
} catch (e: Exception) {
    // handle error
}
```



#### Spark invoice

Spark invoices are single-use and may impose restrictions on the payment, such as amount, expiry, and who is able to pay it.

```kotlin
try {
    val optionalDescription = "<invoice description>"
    // Kotlin MPP (BigInteger from com.ionspin.kotlin.bignum.integer, which is included in
    // package)
    val optionalAmountSats = BigInteger.fromLong(5_000L)
    // Android (BigInteger from java.math)
    // val optionalAmountSats = BigInteger.valueOf(5_000L)
    // Optionally set the expiry UNIX timestamp in seconds
    val optionalExpiryTimeSeconds = 1716691200.toULong()
    val optionalSenderPublicKey = "<sender public key>"

    val request = ReceivePaymentRequest(
        ReceivePaymentMethod.SparkInvoice(
            tokenIdentifier = null,
            description = optionalDescription,
            amount = optionalAmountSats,
            expiryTime = optionalExpiryTimeSeconds,
            senderPublicKey = optionalSenderPublicKey
        )
    )
    val response = sdk.receivePayment(request)

    val paymentRequest = response.paymentRequest
    // Log.v("Breez", "Payment Request: ${paymentRequest}")
    val receiveFeeSats = response.fee
    // Log.v("Breez", "Fees: ${receiveFeeSats} sats")
} catch (e: Exception) {
    // handle error
}
```



## Event Flows

Once a receive payment is initiated, you can follow and react to the different payment events using the guide below for each payment method. See [listening to events](/llms/kotlin/guide/events.md) for how to subscribe to events. 

The `SdkEvent.Synced` event is also emitted as the SDK syncs in the background. See [fetching the balance](/llms/kotlin/guide/get_info.md) for the recommended pattern for refreshing the balance and payments list.

#### Lightning

| Event                | Description                                                       | UX Suggestion                                    |
| -------------------- | ----------------------------------------------------------------- | ------------------------------------------------ |
| **PaymentPending**   | The Spark transfer was detected and the claim process will start. | Show payment as pending.                         |
| **PaymentSucceeded** | The Spark transfer is claimed and the payment is complete.        | Show the payment as complete and call `getInfo` to read the updated balance. The SDK refreshes the cached balance before emitting this event. See [fetching the balance](/llms/kotlin/guide/get_info.md). |

#### Bitcoin

The following events are emitted in order during the deposit lifecycle. See [Listening to events](/llms/kotlin/guide/events.md) for how to subscribe.

| Event                 | Description                                                                                                                              | UX Suggestion                                                                                               |
| --------------------- | ---------------------------------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------- |
| **NewDeposits**       | New deposits were detected. Each deposit includes a `isMature` field indicating whether it has enough confirmations to be claimed. | Show the deposit to the user. If it does not yet have sufficient confirmations, show it as pending.          |
| **ClaimedDeposits**   | The SDK successfully claimed confirmed deposits.                                                                                         |                                                                                                             |
| **UnclaimedDeposits** | Claiming failed (e.g. fee exceeded the configured maximum or the UTXO could not be found).                                               | Allow the user to manually claim or refund. See [Claiming on-chain deposits](/llms/kotlin/guide/onchain_claims.md). |
| **PaymentPending**    | The Spark transfer was detected and the claim process will start.                                                                        | Show payment as pending.                                                                                    |
| **PaymentSucceeded**  | The Spark transfer is claimed and the payment is complete.                                                                               | Show the payment as complete and call `getInfo` to read the updated balance. The SDK refreshes the cached balance before emitting this event. See [fetching the balance](/llms/kotlin/guide/get_info.md).                                                            |

#### Spark

| Event                | Description                                                                                                                                                                                          | UX Suggestion                                    |
| -------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------ |
| **PaymentPending**   | The Spark transfer was detected and the claim process will start. For Spark HTLC payments, the claim will only start once the HTLC is claimed. For more details see [Spark HTLC payments](htlcs.md). | Show payment as pending.                         |
| **PaymentSucceeded** | The Spark transfer is claimed and the payment is complete.                                                                                                                                           | Show the payment as complete and call `getInfo` to read the updated balance. The SDK refreshes the cached balance before emitting this event. See [fetching the balance](/llms/kotlin/guide/get_info.md). |
