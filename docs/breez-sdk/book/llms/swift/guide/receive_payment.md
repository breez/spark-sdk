# Receiving payments

API docs: https://breez.github.io/spark-sdk/breez_sdk_spark/struct.BreezSdk.html#method.receive_payment

Once the SDK is initialized, you can directly begin receiving payments. The SDK currently supports three methods of receiving: Lightning, Bitcoin and Spark.

## Lightning

#### BOLT11 invoice

When receiving via Lightning, we can generate a BOLT11 invoice to be paid. Setting the invoice amount fixes the amount the sender should pay.

**Note:** the payment may fallback to a direct Spark payment (if the payer's client supports this).

```swift
let description = "<invoice description>"
// Optionally set the invoice amount you wish the payer to send
let optionalAmountSats: UInt64 = 5_000
// Optionally set the expiry duration in seconds
let optionalExpirySecs: UInt32 = 3600
let response =
    try await sdk
    .receivePayment(
        request: ReceivePaymentRequest(
            paymentMethod: ReceivePaymentMethod.bolt11Invoice(
                description: description,
                amountSats: optionalAmountSats,
                expirySecs: optionalExpirySecs,
                paymentHash: nil
            )
        ))

let paymentRequest = response.paymentRequest
print("Payment Request: {}", paymentRequest)
let receiveFeeSats = response.fee
print("Fees: {} sats", receiveFeeSats)
```



#### LNURL-Pay & Lightning address

To receive via LNURL-Pay and/or a Lightning address, follow [these instructions](/llms/swift/guide/receive_lnurl_pay.md).

> Note: Lightning payments work in Spark even if the receiver is offline. To understand how it works under the hood, read [this](https://docs.spark.money/learn/lightning).

## Bitcoin

For on-chain payments you can generate a Bitcoin deposit address to receive payments. By default the existing address is returned; you can optionally request a new address to rotate to a fresh one for improved privacy. All previously generated addresses remain monitored.

On-chain deposits go through the following lifecycle:

1. **Detected** — The SDK detects the deposit and emits a `SdkEvent.newDeposits` event. The deposit may or may not have sufficient confirmations to be claimed yet.
2. **Sufficient confirmations** — After **3 on-chain confirmations**, the deposit has sufficient confirmations and the SDK automatically attempts to claim it.
3. **Claimed or unclaimed** — If claiming succeeds, the funds are added to your balance. If it fails (e.g. fees too high), the deposit remains unclaimed and can be [manually claimed or refunded](/llms/swift/guide/onchain_claims.md).

```swift
let newAddress: Bool? = nil // Set to true to get a new address
let response =
    try await sdk
    .receivePayment(
        request: ReceivePaymentRequest(
            paymentMethod: ReceivePaymentMethod.bitcoinAddress(
                newAddress: newAddress)
        ))

let paymentRequest = response.paymentRequest
print("Payment Request: {}", paymentRequest)
let receiveFeeSats = response.fee
print("Fees: {} sats", receiveFeeSats)
```



To track pending deposits, use `listUnclaimedDeposits` and filter by the `isMature` field:

```swift
let request = ListUnclaimedDepositsRequest()
let response = try await sdk.listUnclaimedDeposits(request: request)

let pendingDeposits = response.deposits.filter { !$0.isMature }

for deposit in pendingDeposits {
    print("Pending deposit: \(deposit.txid):\(deposit.vout)")
    print("Amount: \(deposit.amountSats) sats")
}
```



## Spark

For payments between Spark users, you can use a Spark address or generate a Spark invoice to receive payments.

#### Spark address

Spark addresses are static.

```swift
let response =
    try await sdk
    .receivePayment(
        request: ReceivePaymentRequest(
            paymentMethod: ReceivePaymentMethod.sparkAddress
        ))

let paymentRequest = response.paymentRequest
print("Payment Request: {}", paymentRequest)
let receiveFeeSats = response.fee
print("Fees: {} sats", receiveFeeSats)
```



#### Spark invoice

Spark invoices are single-use and may impose restrictions on the payment, such as amount, expiry, and who is able to pay it.

```swift
let optionalDescription = "<invoice description>"
let optionalAmountSats = BInt(5_000)
// Optionally set the expiry UNIX timestamp in seconds
let optionalExpiryTimeSeconds: UInt64 = 1_716_691_200
let optionalSenderPublicKey = "<sender public key>"

let response =
    try await sdk
    .receivePayment(
        request: ReceivePaymentRequest(
            paymentMethod: ReceivePaymentMethod.sparkInvoice(
                amount: optionalAmountSats,
                tokenIdentifier: nil,
                expiryTime: optionalExpiryTimeSeconds,
                description: optionalDescription,
                senderPublicKey: optionalSenderPublicKey
            )
        ))

let paymentRequest = response.paymentRequest
print("Payment Request: {}", paymentRequest)
let receiveFeeSats = response.fee
print("Fees: {} sats", receiveFeeSats)
```



## Event Flows

Once a receive payment is initiated, you can follow and react to the different payment events using the guide below for each payment method. See [listening to events](/llms/swift/guide/events.md) for how to subscribe to events. 

The `SdkEvent.synced` event is also emitted as the SDK syncs in the background. See [fetching the balance](/llms/swift/guide/get_info.md) for the recommended pattern for refreshing the balance and payments list.

#### Lightning

| Event                | Description                                                       | UX Suggestion                                    |
| -------------------- | ----------------------------------------------------------------- | ------------------------------------------------ |
| **PaymentPending**   | The Spark transfer was detected and the claim process will start. | Show payment as pending.                         |
| **PaymentSucceeded** | The Spark transfer is claimed and the payment is complete.        | Show the payment as complete and call `getInfo` to read the updated balance. The SDK refreshes the cached balance before emitting this event. See [fetching the balance](/llms/swift/guide/get_info.md). |

#### Bitcoin

The following events are emitted in order during the deposit lifecycle. See [Listening to events](/llms/swift/guide/events.md) for how to subscribe.

| Event                 | Description                                                                                                                              | UX Suggestion                                                                                               |
| --------------------- | ---------------------------------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------- |
| **NewDeposits**       | New deposits were detected. Each deposit includes a `isMature` field indicating whether it has enough confirmations to be claimed. | Show the deposit to the user. If it does not yet have sufficient confirmations, show it as pending.          |
| **ClaimedDeposits**   | The SDK successfully claimed confirmed deposits.                                                                                         |                                                                                                             |
| **UnclaimedDeposits** | Claiming failed (e.g. fee exceeded the configured maximum or the UTXO could not be found).                                               | Allow the user to manually claim or refund. See [Claiming on-chain deposits](/llms/swift/guide/onchain_claims.md). |
| **PaymentPending**    | The Spark transfer was detected and the claim process will start.                                                                        | Show payment as pending.                                                                                    |
| **PaymentSucceeded**  | The Spark transfer is claimed and the payment is complete.                                                                               | Show the payment as complete and call `getInfo` to read the updated balance. The SDK refreshes the cached balance before emitting this event. See [fetching the balance](/llms/swift/guide/get_info.md).                                                            |

#### Spark

| Event                | Description                                                                                                                                                                                          | UX Suggestion                                    |
| -------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------ |
| **PaymentPending**   | The Spark transfer was detected and the claim process will start. For Spark HTLC payments, the claim will only start once the HTLC is claimed. For more details see [Spark HTLC payments](htlcs.md). | Show payment as pending.                         |
| **PaymentSucceeded** | The Spark transfer is claimed and the payment is complete.                                                                                                                                           | Show the payment as complete and call `getInfo` to read the updated balance. The SDK refreshes the cached balance before emitting this event. See [fetching the balance](/llms/swift/guide/get_info.md). |
