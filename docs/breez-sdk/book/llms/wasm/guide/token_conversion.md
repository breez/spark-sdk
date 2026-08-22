# Converting tokens

Token conversion enables payments to be made without holding the required asset by converting on-the-fly between Bitcoin and tokens using the Flashnet protocol.

## Fetching conversion limits

API docs: https://breez.github.io/spark-sdk/breez_sdk_spark/struct.BreezSdk.html#method.fetch_conversion_limits

Before performing a conversion, you can fetch the minimum amounts required for the conversion. The limits depend on the conversion direction:

- **Bitcoin to token**: Minimum Bitcoin amount (in satoshis) and minimum token amount to receive (in token base units)
- **Token to Bitcoin**: Minimum token amount (in token base units) and minimum Bitcoin amount to receive (in satoshis)

```typescript
// Fetch limits for converting Bitcoin to a token
const fromBitcoinResponse = await sdk.fetchConversionLimits({
  conversionType: { type: 'fromBitcoin' },
  tokenIdentifier: '<token identifier>'
})

if (fromBitcoinResponse.minFromAmount !== undefined) {
  console.log(`Minimum BTC to convert: ${fromBitcoinResponse.minFromAmount} sats`)
}
if (fromBitcoinResponse.minToAmount !== undefined) {
  console.log(`Minimum tokens to receive: ${fromBitcoinResponse.minToAmount} base units`)
}

// Fetch limits for converting a token to Bitcoin
const toBitcoinResponse = await sdk.fetchConversionLimits({
  conversionType: {
    type: 'toBitcoin',
    fromTokenIdentifier: '<token identifier>'
  },
  tokenIdentifier: undefined
})

if (toBitcoinResponse.minFromAmount !== undefined) {
  console.log(`Minimum tokens to convert: ${toBitcoinResponse.minFromAmount} base units`)
}
if (toBitcoinResponse.minToAmount !== undefined) {
  console.log(`Minimum BTC to receive: ${toBitcoinResponse.minToAmount} sats`)
}
```



**Developer note**

Amounts are denominated in satoshis for Bitcoin (1 BTC = 100,000,000 sats) and in token base units for tokens. Token base units depend on the token's decimal specification.

## Converting Bitcoin to tokens

API docs: https://breez.github.io/spark-sdk/breez_sdk_spark/struct.BreezSdk.html#method.prepare_send_payment

Token conversion enables payments of tokens like <a href="https://sparkscan.io/token/3206c93b24a4d18ea19d0a9a213204af2c7e74a6d16c7535cc5d33eca4ad1eca?network=mainnet" target="_blank">USDB</a> to be made without holding the token, but instead using Bitcoin.

To do so, when preparing to send a payment, set the conversion options. The conversion will first calculate the Bitcoin amount needed to be converted into the token, convert Bitcoin into that token amount, and then finally complete the payment.

```typescript
const paymentRequest = '<spark address or invoice>'
// Token identifier must match the invoice in case it specifies one.
const tokenIdentifier = '<token identifier>'
// Set the amount of tokens you wish to send (in token base units).
const amount = BigInt(1_000)
// Set to use Bitcoin funds to pay via conversion
const optionalMaxSlippageBps = 50
const optionalCompletionTimeoutSecs = 30
const conversionOptions: ConversionOptions = {
  conversionType: {
    type: 'fromBitcoin'
  },
  maxSlippageBps: optionalMaxSlippageBps,
  completionTimeoutSecs: optionalCompletionTimeoutSecs
}

const prepareResponse = await sdk.prepareSendPayment({
  paymentRequest: { type: 'input', input: paymentRequest },
  amount,
  tokenIdentifier,
  conversionOptions,
  feePolicy: undefined
})

// If the fees are acceptable, continue to send the token payment
if (prepareResponse.conversionEstimate !== undefined) {
  const conversionEstimate = prepareResponse.conversionEstimate
  console.log(
    `Estimated conversion: ${conversionEstimate.amountIn} token units → ` +
    `${conversionEstimate.amountOut} sats`
  )
  console.log(`Estimated conversion fee: ${conversionEstimate.fee} token units`)
}
```



**Developer note**

When a conversion fails due to exceeding the maximum slippage, the conversion will be refunded automatically.

**Developer note**

The conversion may result in some token balance remaining in the wallet after the payment is sent. This remaining balance is to account for slippage in the conversion.

## Converting tokens to Bitcoin

API docs: https://breez.github.io/spark-sdk/breez_sdk_spark/struct.BreezSdk.html#method.prepare_send_payment

Token conversion also enables Bitcoin payments to be made without holding the required Bitcoin, but instead using a supported token asset like <a href="https://sparkscan.io/token/3206c93b24a4d18ea19d0a9a213204af2c7e74a6d16c7535cc5d33eca4ad1eca?network=mainnet" target="_blank">USDB</a>.

To do so, when preparing to send a payment, set the conversion options. The conversion will first calculate the amount needed to be converted into Bitcoin, convert the token into that Bitcoin amount, and then finally complete the payment.

```typescript
const paymentRequest = '<payment request>'
// Set to use token funds to pay via conversion
const optionalMaxSlippageBps = 50
const optionalCompletionTimeoutSecs = 30
const conversionOptions: ConversionOptions = {
  conversionType: {
    type: 'toBitcoin',
    fromTokenIdentifier: '<token identifier>'
  },
  maxSlippageBps: optionalMaxSlippageBps,
  completionTimeoutSecs: optionalCompletionTimeoutSecs
}

const prepareResponse = await sdk.prepareSendPayment({
  paymentRequest: { type: 'input', input: paymentRequest },
  amount: undefined,
  tokenIdentifier: undefined,
  conversionOptions,
  feePolicy: undefined
})

// If the fees are acceptable, continue to create the Send Payment
if (prepareResponse.conversionEstimate !== undefined) {
  const conversionEstimate = prepareResponse.conversionEstimate
  console.debug(
    `Estimated conversion: ${conversionEstimate.amountIn} token units → ` +
    `${conversionEstimate.amountOut} sats`
  )
  console.debug(`Estimated conversion fee: ${conversionEstimate.fee} token units`)
}
```



**Developer note**

When a conversion fails due to exceeding the maximum slippage, the conversion will be refunded automatically.

**Developer note**

The conversion may result in some Bitcoin remaining in the wallet after the payment is sent. This remaining Bitcoin is to account for slippage in the conversion.
