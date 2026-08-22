# Receiving payments

API docs: https://breez.github.io/spark-sdk/breez_sdk_spark/struct.BreezSdk.html#method.receive_payment

Once the SDK is initialized, you can directly begin receiving payments. The SDK currently supports three methods of receiving: Lightning, Bitcoin and Spark.

## Lightning

#### BOLT11 invoice

When receiving via Lightning, we can generate a BOLT11 invoice to be paid. Setting the invoice amount fixes the amount the sender should pay.

**Note:** the payment may fallback to a direct Spark payment (if the payer's client supports this).

```typescript
const description = '<invoice description>'
// Optionally set the invoice amount you wish the payer to send
const optionalAmountSats = 5_000
// Optionally set the expiry duration in seconds
const optionalExpirySecs = 3600

const response = await sdk.receivePayment({
  paymentMethod: {
    type: 'bolt11Invoice',
    description,
    amountSats: optionalAmountSats,
    expirySecs: optionalExpirySecs,
    paymentHash: undefined
  }
})

const paymentRequest = response.paymentRequest
console.log(`Payment Request: ${paymentRequest}`)
const receiveFeeSats = response.fee
console.log(`Fees: ${receiveFeeSats} sats`)
```



#### LNURL-Pay & Lightning address

To receive via LNURL-Pay and/or a Lightning address, follow [these instructions](/llms/wasm/guide/receive_lnurl_pay.md).

> Note: Lightning payments work in Spark even if the receiver is offline. To understand how it works under the hood, read [this](https://docs.spark.money/learn/lightning).

## Bitcoin

For on-chain payments you can generate a Bitcoin deposit address to receive payments. By default the existing address is returned; you can optionally request a new address to rotate to a fresh one for improved privacy. All previously generated addresses remain monitored.

On-chain deposits go through the following lifecycle:

1. **Detected** — The SDK detects the deposit and emits a `SdkEvent.NewDeposits` event. The deposit may or may not have sufficient confirmations to be claimed yet.
2. **Sufficient confirmations** — After **3 on-chain confirmations**, the deposit has sufficient confirmations and the SDK automatically attempts to claim it.
3. **Claimed or unclaimed** — If claiming succeeds, the funds are added to your balance. If it fails (e.g. fees too high), the deposit remains unclaimed and can be [manually claimed or refunded](/llms/wasm/guide/onchain_claims.md).

```typescript
const newAddress = undefined // Set to true to get a new address
const response = await sdk.receivePayment({
  paymentMethod: { type: 'bitcoinAddress', newAddress }
})

const paymentRequest = response.paymentRequest
console.log(`Payment Request: ${paymentRequest}`)
const receiveFeeSats = response.fee
console.log(`Fees: ${receiveFeeSats} sats`)
```



To track pending deposits, use `listUnclaimedDeposits` and filter by the `isMature` field:

```typescript
const request: ListUnclaimedDepositsRequest = {}
const response = await sdk.listUnclaimedDeposits(request)

const pendingDeposits = response.deposits.filter((d) => !d.isMature)

for (const deposit of pendingDeposits) {
  console.log(`Pending deposit: ${deposit.txid}:${deposit.vout}`)
  console.log(`Amount: ${deposit.amountSats} sats`)
}
```



## Spark

For payments between Spark users, you can use a Spark address or generate a Spark invoice to receive payments.

#### Spark address

Spark addresses are static.

```typescript
const response = await sdk.receivePayment({
  paymentMethod: { type: 'sparkAddress' }
})

const paymentRequest = response.paymentRequest
console.log(`Payment Request: ${paymentRequest}`)
const receiveFeeSats = response.fee
console.log(`Fees: ${receiveFeeSats} sats`)
```



#### Spark invoice

Spark invoices are single-use and may impose restrictions on the payment, such as amount, expiry, and who is able to pay it.

```typescript
const optionalDescription = '<invoice description>'
const optionalAmountSats = '5000'
// Optionally set the expiry UNIX timestamp in seconds
const optionalExpiryTimeSeconds = 1716691200
const optionalSenderPublicKey = '<sender public key>'

const response = await sdk.receivePayment({
  paymentMethod: {
    type: 'sparkInvoice',
    description: optionalDescription,
    amount: optionalAmountSats,
    expiryTime: optionalExpiryTimeSeconds,
    senderPublicKey: optionalSenderPublicKey
  }
})

const paymentRequest = response.paymentRequest
console.log(`Payment Request: ${paymentRequest}`)
const receiveFeeSats = response.fee
console.log(`Fees: ${receiveFeeSats} sats`)
```



## Event Flows

Once a receive payment is initiated, you can follow and react to the different payment events using the guide below for each payment method. See [listening to events](/llms/wasm/guide/events.md) for how to subscribe to events. 

The `SdkEvent.Synced` event is also emitted as the SDK syncs in the background. See [fetching the balance](/llms/wasm/guide/get_info.md) for the recommended pattern for refreshing the balance and payments list.

#### Lightning

| Event                | Description                                                       | UX Suggestion                                    |
| -------------------- | ----------------------------------------------------------------- | ------------------------------------------------ |
| **PaymentPending**   | The Spark transfer was detected and the claim process will start. | Show payment as pending.                         |
| **PaymentSucceeded** | The Spark transfer is claimed and the payment is complete.        | Show the payment as complete and call `getInfo` to read the updated balance. The SDK refreshes the cached balance before emitting this event. See [fetching the balance](/llms/wasm/guide/get_info.md). |

#### Bitcoin

The following events are emitted in order during the deposit lifecycle. See [Listening to events](/llms/wasm/guide/events.md) for how to subscribe.

| Event                 | Description                                                                                                                              | UX Suggestion                                                                                               |
| --------------------- | ---------------------------------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------- |
| **NewDeposits**       | New deposits were detected. Each deposit includes a `isMature` field indicating whether it has enough confirmations to be claimed. | Show the deposit to the user. If it does not yet have sufficient confirmations, show it as pending.          |
| **ClaimedDeposits**   | The SDK successfully claimed confirmed deposits.                                                                                         |                                                                                                             |
| **UnclaimedDeposits** | Claiming failed (e.g. fee exceeded the configured maximum or the UTXO could not be found).                                               | Allow the user to manually claim or refund. See [Claiming on-chain deposits](/llms/wasm/guide/onchain_claims.md). |
| **PaymentPending**    | The Spark transfer was detected and the claim process will start.                                                                        | Show payment as pending.                                                                                    |
| **PaymentSucceeded**  | The Spark transfer is claimed and the payment is complete.                                                                               | Show the payment as complete and call `getInfo` to read the updated balance. The SDK refreshes the cached balance before emitting this event. See [fetching the balance](/llms/wasm/guide/get_info.md).                                                            |

#### Spark

| Event                | Description                                                                                                                                                                                          | UX Suggestion                                    |
| -------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------ |
| **PaymentPending**   | The Spark transfer was detected and the claim process will start. For Spark HTLC payments, the claim will only start once the HTLC is claimed. For more details see [Spark HTLC payments](htlcs.md). | Show payment as pending.                         |
| **PaymentSucceeded** | The Spark transfer is claimed and the payment is complete.                                                                                                                                           | Show the payment as complete and call `getInfo` to read the updated balance. The SDK refreshes the cached balance before emitting this event. See [fetching the balance](/llms/wasm/guide/get_info.md). |
