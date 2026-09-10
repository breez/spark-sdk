# Receiving payments

API docs: https://breez.github.io/spark-sdk/breez_sdk_spark/struct.BreezSdk.html#method.receive_payment

Once the SDK is initialized, you can directly begin receiving payments. The SDK supports receiving via Lightning, Bitcoin, Spark, and USDC/USDT into Spark from a supported external chain.

## Lightning

#### BOLT11 invoice

When receiving via Lightning, we can generate a BOLT11 invoice to be paid. Setting the invoice amount fixes the amount the sender should pay.

To create an invoice for another Spark wallet, set `receiverIdentityPublicKey` to that wallet's identity public key. Creating the invoice requires only the receiver's public key, not their private keys.

**Note:** the payment may fallback to a direct Spark payment (if the payer's client supports this).

```typescript
const description = '<invoice description>'
// Optionally set the invoice amount you wish the payer to send
const optionalAmountSats = BigInt(5_000)
// Optionally set the expiry duration in seconds
const optionalExpirySecs = 3600
// Set this to create an invoice for another Spark identity
const optionalReceiverIdentityPublicKey = undefined

const response = await sdk.receivePayment({
  paymentMethod: new ReceivePaymentMethod.Bolt11Invoice({
    description,
    amountSats: optionalAmountSats,
    expirySecs: optionalExpirySecs,
    paymentHash: undefined,
    receiverIdentityPublicKey: optionalReceiverIdentityPublicKey
  })
})

const paymentRequest = response.paymentRequest
console.log(`Payment Request: ${paymentRequest}`)
const receiveFeeSats = response.fee
console.log(`Fees: ${receiveFeeSats} sats`)
```



#### LNURL-Pay & Lightning address

To receive via LNURL-Pay and/or a Lightning address, follow [these instructions](/llms/react-native/guide/receive_lnurl_pay.md).

> Note: Lightning payments work in Spark even if the receiver is offline. To understand how it works under the hood, read [this](https://docs.spark.money/learn/lightning).

## Bitcoin

For on-chain payments you can generate a Bitcoin deposit address to receive payments. By default the existing address is returned; you can optionally request a new address to rotate to a fresh one for improved privacy. All previously generated addresses remain monitored.

On-chain deposits go through the following lifecycle:

1. **Detected** — The SDK detects the deposit and emits a `SdkEvent.NewDeposits` event. The deposit may or may not have sufficient confirmations to be claimed yet.
2. **Sufficient confirmations** — After **3 on-chain confirmations**, the deposit has sufficient confirmations and the SDK automatically attempts to claim it.
3. **Claimed or unclaimed** — If claiming succeeds, the funds are added to your balance. If it fails (e.g. fees too high), the deposit remains unclaimed and can be [manually claimed or refunded](/llms/react-native/guide/onchain_claims.md).

```typescript
const newAddress = undefined // Set to true to get a new address
const response = await sdk.receivePayment({
  paymentMethod: new ReceivePaymentMethod.BitcoinAddress({
    newAddress
  })
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
  paymentMethod: new ReceivePaymentMethod.SparkAddress()
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
const optionalAmountSats = BigInt(5_000)
// Optionally set the expiry UNIX timestamp in seconds
const optionalExpiryTimeSeconds = BigInt(1716691200)
const optionalSenderPublicKey = '<sender public key>'

const response = await sdk.receivePayment({
  paymentMethod: new ReceivePaymentMethod.SparkInvoice({
    description: optionalDescription,
    amount: optionalAmountSats,
    expiryTime: optionalExpiryTimeSeconds,
    senderPublicKey: optionalSenderPublicKey,
    tokenIdentifier: undefined
  })
})

const paymentRequest = response.paymentRequest
console.log(`Payment Request: ${paymentRequest}`)
const receiveFeeSats = response.fee
console.log(`Fees: ${receiveFeeSats} sats`)
```



## USDC/USDT

Cross-chain receive is supported only via the Orchestra provider. Receive USDC or USDT from a sender on one of several supported chains: Ethereum-family chains (Arbitrum, Base, and similar EVM networks), Solana, and Tron. The receiver lands either BTC sats or [USDB](./stable_balance.md) (a 6-decimal USD-pegged token on Spark) on the Spark side. This feature must be enabled in [the SDK configuration](./config.md#usdc-usdt) before using. See [USDC/USDT](./cross_chain.md) for provider details and the status lifecycle.

Call `getCrossChainRoutes` with `CrossChainRouteFilter.Receive` to discover supported source assets. Each `CrossChainRoutePair` names the provider, source chain and asset, decimals, optional token contract address, and the Spark-side destinations the route lands (`CrossChainRoutePair.acceptedAssets`).

```typescript
const routes = await sdk.getCrossChainRoutes(
  new CrossChainRouteFilter.Receive({ contractAddress: undefined })
)

for (const route of routes) {
  console.debug(
    `Route via ${route.provider}: ${route.chain}/${route.asset} -> Spark`
  )
}
```



Build `ReceivePaymentMethod.CrossChain` with the chosen route and an `amount`.

The `amount` on {{#name ReceivePaymentMethod::CrossChain}} is in the source asset's base units, per the route's `CrossChainRoutePair.decimals`. USD-stable sources sit at USD parity, so `1_000_000` is 1 USDC (6 decimals), about $1.

`feeMode` controls what the amount means:

- `CrossChainFeeMode.FeesExcluded` (default): `amount` is the receiver's target on Spark. The SDK pads the sender's deposit to cover provider fees plus an overpay buffer.
- `CrossChainFeeMode.FeesIncluded`: `amount` is the deposit the sender pays. The receiver lands `amount - fees`.

`destination` picks which Spark-side asset the receiver wants delivered. Left unset, the SDK auto-picks the wallet's active stable-balance token if the route supports it, otherwise BTC (converted from the USD amount via the live BTC/USD rate).

`maxSlippageBps` (10 to 500) bounds the price movement tolerated between quote and delivery. `targetOverpayBps` (0 to 500) sets the FeesExcluded overpay buffer. Left unset, the SDK defaults apply.

The `paymentRequest` field carries an EIP-681 URI for EVM routes and the bare deposit address for Solana and Tron. The `crossChainInfo` block surfaces the bare deposit address, deposit amount, expected receive amount, destination denomination, and quote `expiresAt`. The receiver pays no fee; the sender's deposit covers it.

```typescript
// amount is in the route's source-asset base units (USD-stable parity:
// 1_000_000 = $1 on 6-decimal routes). See the guide for feeMode,
// destination, and the slippage/overpay overrides.
const amount = BigInt(1_000_000)
const optionalDestination: SparkAsset | undefined = undefined
const optionalMaxSlippageBps = 100
const optionalTargetOverpayBps = undefined
const optionalFeeMode = undefined

const response = await sdk.receivePayment({
  paymentMethod: new ReceivePaymentMethod.CrossChain({
    route,
    amount,
    destination: optionalDestination,
    feeMode: optionalFeeMode,
    maxSlippageBps: optionalMaxSlippageBps,
    targetOverpayBps: optionalTargetOverpayBps
  })
})

console.debug(`Payment request: ${response.paymentRequest}`)
if (response.crossChainInfo !== undefined) {
  const {
    depositAddress,
    depositAmount,
    expectedReceivedAmount,
    destinationAsset,
    expiresAt
  } = response.crossChainInfo
  console.debug(`Deposit address: ${depositAddress}`)
  console.debug(`Deposit amount: ${depositAmount}`)
  console.debug(
    `Expected received: ${expectedReceivedAmount} ${destinationAsset}`
  )
  console.debug(`Expires at: ${expiresAt}`)
}
```



## Event Flows

Once a receive payment is initiated, you can follow and react to the different payment events using the guide below for each payment method. See [listening to events](/llms/react-native/guide/events.md) for how to subscribe to events. 

The `SdkEvent.Synced` event is also emitted as the SDK syncs in the background. See [fetching the balance](/llms/react-native/guide/get_info.md) for the recommended pattern for refreshing the balance and payments list.

#### Lightning

| Event                | Description                                                       | UX Suggestion                                    |
| -------------------- | ----------------------------------------------------------------- | ------------------------------------------------ |
| **PaymentPending**   | The Spark transfer was detected and the claim process will start. | Show payment as pending.                         |
| **PaymentSucceeded** | The Spark transfer is claimed and the payment is complete.        | Show the payment as complete and call `getInfo` to read the updated balance. The SDK refreshes the cached balance before emitting this event. See [fetching the balance](/llms/react-native/guide/get_info.md). |

#### Bitcoin

The following events are emitted in order during the deposit lifecycle. See [Listening to events](/llms/react-native/guide/events.md) for how to subscribe.

| Event                 | Description                                                                                                                              | UX Suggestion                                                                                               |
| --------------------- | ---------------------------------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------- |
| **NewDeposits**       | New deposits were detected. Each deposit includes a `isMature` field indicating whether it has enough confirmations to be claimed. | Show the deposit to the user. If it does not yet have sufficient confirmations, show it as pending.          |
| **ClaimedDeposits**   | The SDK successfully claimed confirmed deposits.                                                                                         |                                                                                                             |
| **UnclaimedDeposits** | Claiming failed (e.g. fee exceeded the configured maximum or the UTXO could not be found).                                               | Allow the user to manually claim or refund. See [Claiming on-chain deposits](/llms/react-native/guide/onchain_claims.md). |
| **PaymentPending**    | The Spark transfer was detected and the claim process will start.                                                                        | Show payment as pending.                                                                                    |
| **PaymentSucceeded**  | The Spark transfer is claimed and the payment is complete.                                                                               | Show the payment as complete and call `getInfo` to read the updated balance. The SDK refreshes the cached balance before emitting this event. See [fetching the balance](/llms/react-native/guide/get_info.md).                                                            |

#### Spark

| Event                | Description                                                                                                                                                                                          | UX Suggestion                                    |
| -------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------ |
| **PaymentPending**   | The Spark transfer was detected and the claim process will start. For Spark HTLC payments, the claim will only start once the HTLC is claimed. For more details see [Spark HTLC payments](htlcs.md). | Show payment as pending.                         |
| **PaymentSucceeded** | The Spark transfer is claimed and the payment is complete.                                                                                                                                           | Show the payment as complete and call `getInfo` to read the updated balance. The SDK refreshes the cached balance before emitting this event. See [fetching the balance](/llms/react-native/guide/get_info.md). |

#### USDC/USDT

| Event                | Description                                                                                          | UX Suggestion                                    |
| -------------------- | ---------------------------------------------------------------------------------------------------- | ------------------------------------------------ |
| **PaymentPending**   | The sender's deposit was detected and the inbound Spark transfer claim is in progress.               | Show payment as pending.                         |
| **PaymentSucceeded** | The inbound Spark transfer is claimed and the payment is complete.                                   | Show the payment as complete and call `getInfo` to read the updated balance. The SDK refreshes the cached balance before emitting this event. See [fetching the balance](/llms/react-native/guide/get_info.md). |
