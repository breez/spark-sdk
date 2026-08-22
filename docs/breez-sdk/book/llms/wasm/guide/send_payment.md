# Sending payments

Once the SDK is initialized, you can directly begin sending payments. The send process takes two steps:

1. [Preparing the Payment](send_payment.md#preparing-payments)
2. [Sending the Payment](send_payment.md#sending-payments)

For sending payments via LNURL, see [LNURL-Pay](lnurl_pay.md).

## Preparing Payments

API docs: https://breez.github.io/spark-sdk/breez_sdk_spark/struct.BreezSdk.html#method.prepare_send_payment

During the prepare step, the SDK ensures that the inputs are valid with respect to the payment request type,
and also returns the fees related to the payment so they can be confirmed.

The payment request field supports Lightning invoices, Bitcoin addresses, Spark addresses and Spark invoices.

**Developer note**

Payments can be sent without holding Bitcoin by converting on-the-fly as a step before sending a payment. See <a href="./token_conversion.md">Converting tokens</a> for more information.

### Lightning

#### BOLT11 invoice

For BOLT11 invoices the amount can be optionally set. The amount set in the request is only taken into account if it's an amountless invoice.

If the invoice also contains a Spark address, the payment can be sent directly via a Spark transfer instead. When this is the case, the prepare response includes the Spark transfer fee. Note that only one fee is paid: either the Lightning fee or the Spark transfer fee, depending on which payment method is ultimately used. See [Lightning](send_payment.md#lightning-1) for how to select the payment method.

```typescript
const paymentRequest = '<bolt11 invoice>'
// Optionally set the amount you wish to pay the receiver
const optionalAmountSats = BigInt(5_000)

const prepareResponse = await sdk.prepareSendPayment({
  paymentRequest: { type: 'input', input: paymentRequest },
  amount: optionalAmountSats,
  tokenIdentifier: undefined,
  conversionOptions: undefined,
  feePolicy: undefined
})

// If the fees are acceptable, continue to create the Send Payment
if (prepareResponse.paymentMethod.type === 'bolt11Invoice') {
  // Fees to pay via Lightning
  const lightningFeeSats = prepareResponse.paymentMethod.lightningFeeSats
  // Or fees to pay (if available) via a Spark transfer
  const sparkTransferFeeSats = prepareResponse.paymentMethod.sparkTransferFeeSats
  console.debug(`Lightning Fees: ${lightningFeeSats} sats`)
  console.debug(`Spark Transfer Fees: ${sparkTransferFeeSats} sats`)
}
```



### Bitcoin

For Bitcoin addresses, the amount must be set in the request. The prepare response includes fee quotes for three payment speeds: Slow, Medium, and Fast.

```typescript
const paymentRequest = '<bitcoin address>'
// Set the amount you wish to pay the receiver
const amountSats = BigInt(50_000)

const prepareResponse = await sdk.prepareSendPayment({
  paymentRequest: { type: 'input', input: paymentRequest },
  amount: amountSats,
  tokenIdentifier: undefined,
  conversionOptions: undefined,
  feePolicy: undefined
})

// Review the fee quote for each confirmation speed
if (prepareResponse.paymentMethod.type === 'bitcoinAddress') {
  const feeQuote = prepareResponse.paymentMethod.feeQuote
  const slowFeeSats = feeQuote.speedSlow.userFeeSat + feeQuote.speedSlow.l1BroadcastFeeSat
  const mediumFeeSats = feeQuote.speedMedium.userFeeSat + feeQuote.speedMedium.l1BroadcastFeeSat
  const fastFeeSats = feeQuote.speedFast.userFeeSat + feeQuote.speedFast.l1BroadcastFeeSat
  console.debug(`Slow fee: ${slowFeeSats} sats`)
  console.debug(`Medium fee: ${mediumFeeSats} sats`)
  console.debug(`Fast fee: ${fastFeeSats} sats`)
}
```



### Spark

#### Spark address

For Spark addresses, the amount must be set in the request. Sending to a Spark address uses a direct Spark transfer.

```typescript
const paymentRequest = '<spark address>'
// Set the amount you wish to pay the receiver
const amountSats = BigInt(50_000)

const prepareResponse = await sdk.prepareSendPayment({
  paymentRequest: { type: 'input', input: paymentRequest },
  amount: amountSats,
  tokenIdentifier: undefined,
  conversionOptions: undefined,
  feePolicy: undefined
})

// If the fees are acceptable, continue to create the Send Payment
if (prepareResponse.paymentMethod.type === 'sparkAddress') {
  const feeSats = prepareResponse.paymentMethod.fee
  console.debug(`Fees: ${feeSats} sats`)
}
```



#### Spark invoice

For Spark invoices, the amount can be optionally set. It is only required if the invoice doesn't specify an amount. If the invoice specifies an amount, providing a different amount is not supported.

**Developer note**

Spark invoices may require a token (non-Bitcoin) as the payment asset. To determine the requirements of a Spark invoice and any restrictions it may impose, see the <a href="./parse.md">Parsing inputs</a> page. To learn more about tokens, see the <a href="./tokens.md">Handling tokens</a> page.

```typescript
const paymentRequest = '<spark invoice>'
// Optionally set the amount you wish to pay the receiver
const optionalAmountSats = BigInt(50_000)

const prepareResponse = await sdk.prepareSendPayment({
  paymentRequest: { type: 'input', input: paymentRequest },
  amount: optionalAmountSats,
  tokenIdentifier: undefined,
  conversionOptions: undefined,
  feePolicy: undefined
})

// If the fees are acceptable, continue to create the Send Payment
if (prepareResponse.paymentMethod.type === 'sparkInvoice') {
  const feeSats = prepareResponse.paymentMethod.fee
  console.debug(`Fees: ${feeSats} sats`)
}
```



### USDC/USDT

Send USDC or USDT from a Spark wallet to a recipient on one of several supported chains: Ethereum-family chains (Arbitrum, Base, and similar EVM networks), Solana, and Tron. The source on the Spark side is BTC sats or USDB. This feature must be enabled in [the SDK configuration](./config.md#send-usdc-usdt) before using. See [Send USDC/USDT](./cross_chain.md) for provider details and the status lifecycle.

After [parsing](./parse.md) the recipient address into `InputType.CrossChainAddress`, call `getCrossChainRoutes` with `CrossChainRouteFilter.Send` carrying the parsed `CrossChainAddressDetails`. The returned `CrossChainRoutePair`s name the provider, destination chain and asset, decimals, optional token contract address, and which source assets (BTC sats or USDB) each route accepts.

```typescript
const input = '<recipient address>'
const parsed = await sdk.parse(input)
if (parsed.type !== 'crossChainAddress') {
  throw new Error('Not a cross-chain address')
}

const routes = await sdk.getCrossChainRoutes({
  type: 'send',
  addressDetails: parsed
})

for (const route of routes) {
  console.debug(`Route via ${route.provider}: ${route.chain}/${route.asset}`)
}
```



Build `PaymentRequest.CrossChain` with the recipient address, the chosen route, and an optional `maxSlippageBps` (10 to 500 basis points). The amount on the prepare request is denominated in the source asset's base units: sats for a BTC source, USDB base units for a USDB source.

The prepare response carries a quote `expiresAt` timestamp. Re-prepare and pick a fresh route if it lapses before send.

```typescript
// Optionally set the maximum slippage in basis points (10 to 500)
const optionalMaxSlippageBps = 100

const prepareResponse = await sdk.prepareSendPayment({
  paymentRequest: {
    type: 'crossChain',
    address: addressDetails.address,
    route,
    maxSlippageBps: optionalMaxSlippageBps
  },
  amount: BigInt(50_000),
  tokenIdentifier: undefined,
  conversionOptions: undefined,
  feePolicy: undefined
})

if (prepareResponse.paymentMethod.type === 'crossChainAddress') {
  const { amountIn, estimatedOut, feeAmount, expiresAt } = prepareResponse.paymentMethod
  console.debug(`Amount in: ${amountIn}`)
  console.debug(`Estimated out: ${estimatedOut}`)
  console.debug(`Provider fee: ${feeAmount}`)
  console.debug(`Quote expires at: ${expiresAt}`)
}
```



## Fee Policy

By default, fees are added on top of the amount (`FeePolicy.FeesExcluded`). Use `FeePolicy.FeesIncluded` to deduct fees from the amount instead—the receiver gets the amount minus fees.

This is particularly useful when you want to spend your entire balance in a single payment—simply provide your full balance as the amount. Note: `FeePolicy.FeesIncluded` is not compatible with payment requests that specify an amount (e.g., BOLT11 invoices and Spark invoices with amount).

```typescript
// By default ({ type: 'feesExcluded' }), fees are added on top of the amount.
// Use { type: 'feesIncluded' } to deduct fees from the amount instead.
// The receiver gets amount minus fees.
const paymentRequest = '<payment request>'
const amountSats = BigInt(50_000)
const feePolicy: FeePolicy = 'feesIncluded'

const prepareResponse = await sdk.prepareSendPayment({
  paymentRequest: { type: 'input', input: paymentRequest },
  amount: amountSats,
  tokenIdentifier: undefined,
  conversionOptions: undefined,
  feePolicy
})

// The response shows the fee policy used
console.log(`Fee policy: ${JSON.stringify(prepareResponse.feePolicy)}`)
console.log(`Amount: ${prepareResponse.amount}`)
// The receiver gets amount - fees (fees are available in prepareResponse.paymentMethod)
```



When [stable balance](./stable_balance.md) is active, you can send your entire wallet balance — both the token balance and any remaining sats — by combining `FeePolicy.FeesIncluded` with `ConversionType.ToBitcoin` conversion options. See [Sending entire balance](./stable_balance.md#sending-entire-balance) for details.

```typescript
const paymentRequest = '<payment request>'
const tokenIdentifier = '<token identifier>'

const info = await sdk.getInfo({ ensureSynced: false })
const tokenBalance = info.tokenBalances.get(tokenIdentifier)
if (tokenBalance === undefined) {
  throw new Error('Token balance not found')
}

const conversionOptions: ConversionOptions = {
  conversionType: {
    type: 'toBitcoin',
    fromTokenIdentifier: tokenIdentifier
  }
}
const feePolicy: FeePolicy = 'feesIncluded'

const prepareResponse = await sdk.prepareSendPayment({
  paymentRequest: { type: 'input', input: paymentRequest },
  amount: tokenBalance.balance,
  tokenIdentifier,
  conversionOptions,
  feePolicy
})

// The response amount is the estimated total sats available
// (converted sats + existing sat balance)
console.log(`Total sats available: ${prepareResponse.amount}`)

if (prepareResponse.conversionEstimate !== undefined) {
  const estimate = prepareResponse.conversionEstimate
  console.log(`Converting ${estimate.amountIn} token units → ~${estimate.amountOut} sats`)
  console.log(`Conversion fee: ${estimate.fee} token units`)
}
```



## Sending Payments

API docs: https://breez.github.io/spark-sdk/breez_sdk_spark/struct.BreezSdk.html#method.send_payment

Once the payment has been prepared and the fees are accepted, the payment can be sent by passing:

- **Prepare Response** - The response from the [Preparing the Payment](send_payment.md#preparing-payments) step.
- **Options** - Any payment method specific options for the payment (see below).
- **Idempotency Key** - An optional UUID that identifies the payment. If set, providing the same idempotency key for multiple requests will ensure that only one payment is made.

### Lightning

In the optional send payment options for BOLT11 invoices, you can set:

- **Prefer Spark** - Set the preference to use Spark to transfer the payment if the invoice contains a Spark address. By default, using Spark transfers are disabled.
- **Completion Timeout** - By default, this function returns immediately. You can override this behavior by specifying a completion timeout in seconds. If the timeout is reached, a pending payment object is returned. If the payment completes within the timeout, the completed payment object is returned.

```typescript
const options: SendPaymentOptions = {
  type: 'bolt11Invoice',
  preferSpark: false,
  completionTimeoutSecs: 10
}
const optionalIdempotencyKey = '<idempotency key uuid>'
const sendResponse = await sdk.sendPayment({
  prepareResponse,
  options,
  idempotencyKey: optionalIdempotencyKey
})
const payment = sendResponse.payment
```



### Bitcoin

In the optional send payment options for Bitcoin addresses, you can set:

- **Confirmation Speed** - The priority that the Bitcoin transaction confirms, that also effects the fee paid. By default, it is set to Fast.

```typescript
// Select the confirmation speed for the on-chain transaction
const options: SendPaymentOptions = {
  type: 'bitcoinAddress',
  confirmationSpeed: 'medium'
}
const optionalIdempotencyKey = '<idempotency key uuid>'
const sendResponse = await sdk.sendPayment({
  prepareResponse,
  options,
  idempotencyKey: optionalIdempotencyKey
})
const payment = sendResponse.payment
```



### Spark

In the optional send payment options for Spark addresses, you can set:

- **HTLC Options** - Enables Spark HTLC payments, which are an advanced feature that allows for conditional payments. See the [Spark HTLC Payments](htlcs.md) page for more details and example usage.

```typescript
const optionalIdempotencyKey = '<idempotency key uuid>'
const sendResponse = await sdk.sendPayment({
  prepareResponse,
  idempotencyKey: optionalIdempotencyKey
})
const payment = sendResponse.payment
```



### USDC/USDT

Send USDC/USDT has no additional send payment options.

```typescript
// Only valid for sends with no token leg (see Retry safety).
const optionalIdempotencyKey = '<idempotency key uuid>'
const sendResponse = await sdk.sendPayment({
  prepareResponse,
  options: undefined,
  idempotencyKey: optionalIdempotencyKey
})
const payment = sendResponse.payment
console.debug('Payment:', payment)
```



## Event Flows

Once a send payment is initiated, you can follow and react to the different payment events using the guide below for each payment method. See [listening to events](/guide/events.html) for how to subscribe to events. 

The `SdkEvent.Synced` event is also emitted as the SDK syncs in the background. See [fetching the balance](/llms/wasm/guide/get_info.md) for the recommended pattern for refreshing the balance and payments list.

#### Lightning

| Event                | Description                                                                       | UX Suggestion                                    |
| -------------------- | --------------------------------------------------------------------------------- | ------------------------------------------------ |
| **PaymentPending**   | The Spark transfer has been started. Awaiting Lightning payment completion.       | Show payment as pending.                         |
| **PaymentSucceeded** | The Lightning invoice has been paid either over Lightning or via a Spark transfer | Show the payment as complete and call `getInfo` to read the updated balance. The SDK refreshes the cached balance before emitting this event. See [fetching the balance](/llms/wasm/guide/get_info.md). |
| **PaymentFailed**    | The attempt to pay the Lightning invoice failed.                                  |                                                  |

#### Bitcoin

| Event                | Description                                                                   | UX Suggestion                                    |
| -------------------- | ----------------------------------------------------------------------------- | ------------------------------------------------ |
| **PaymentPending**   | The Spark transfer has been started. Awaiting on-chain withdrawal completion. | Show payment as pending.                         |
| **PaymentSucceeded** | The payment amount was successfully withdrawn on-chain.                       | Show the payment as complete and call `getInfo` to read the updated balance. The SDK refreshes the cached balance before emitting this event. See [fetching the balance](/llms/wasm/guide/get_info.md). |

#### Spark

| Event                | Description                     | UX Suggestion                                    |
| -------------------- | ------------------------------- | ------------------------------------------------ |
| **PaymentSucceeded** | The Spark transfer is complete. | Show the payment as complete and call `getInfo` to read the updated balance. The SDK refreshes the cached balance before emitting this event. See [fetching the balance](/llms/wasm/guide/get_info.md). |

#### USDC/USDT

| Event                | Description                                                                                              | UX Suggestion                                    |
| -------------------- | -------------------------------------------------------------------------------------------------------- | ------------------------------------------------ |
| **PaymentPending**   | The deposit transfer has been submitted to the provider. The cross-chain leg is awaiting settlement.     | Show payment as pending; the bridge leg may take several minutes depending on the provider and destination chain. |
| **PaymentSucceeded** | The provider reports the cross-chain order terminal. The amount actually delivered to the recipient is carried on the conversion info. | Show the payment as complete and call `getInfo` to read the updated balance. The SDK refreshes the cached balance before emitting this event. See [fetching the balance](/llms/wasm/guide/get_info.md). |
