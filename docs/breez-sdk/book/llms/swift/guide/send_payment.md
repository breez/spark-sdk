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

For BOLT11 invoices the amount can be optionally set. It is only required if the invoice doesn't specify an amount. If the invoice specifies an amount, providing a different amount is not supported.

If the invoice also contains a Spark address, the payment can be sent directly via a Spark transfer instead. When this is the case, the prepare response includes the Spark transfer fee. Note that only one fee is paid: either the Lightning fee or the Spark transfer fee, depending on which payment method is ultimately used. See [Lightning](send_payment.md#lightning-1) for how to select the payment method.

```swift
let paymentRequest = "<bolt11 invoice>"
// Optionally set the amount you wish to pay the receiver
let optionalAmountSats: BInt? = BInt(5_000)

let prepareResponse = try await sdk.prepareSendPayment(
    request: PrepareSendPaymentRequest(
        paymentRequest: .input(input: paymentRequest),
        amount: optionalAmountSats,
        tokenIdentifier: nil,
        conversionOptions: nil,
        feePolicy: nil
    ))

if case let .bolt11Invoice(_, sparkTransferFeeSats, lightningFeeSats) = prepareResponse
    .paymentMethod
{
    // Fees to pay via Lightning
    print("Lightning Fees: \(lightningFeeSats) sats")
    // Or fees to pay (if available) via a Spark transfer
    if let sparkTransferFeeSats = sparkTransferFeeSats {
        print("Spark Transfer Fees: \(sparkTransferFeeSats) sats")
    }
}
```



### Bitcoin

For Bitcoin addresses, the amount must be set in the request. The prepare response includes fee quotes for three payment speeds: Slow, Medium, and Fast.

```swift
let paymentRequest = "<bitcoin address>"
// Set the amount you wish to pay the receiver
let amountSats = BInt(50_000)

let prepareResponse = try await sdk.prepareSendPayment(
    request: PrepareSendPaymentRequest(
        paymentRequest: .input(input: paymentRequest),
        amount: amountSats,
        tokenIdentifier: nil,
        conversionOptions: nil,
        feePolicy: nil
    ))

// Review the fee quote for each confirmation speed
if case let .bitcoinAddress(address: _, feeQuote: feeQuote) = prepareResponse.paymentMethod {
    let slowFeeSats = feeQuote.speedSlow.userFeeSat + feeQuote.speedSlow.l1BroadcastFeeSat
    let mediumFeeSats = feeQuote.speedMedium.userFeeSat + feeQuote.speedMedium.l1BroadcastFeeSat
    let fastFeeSats = feeQuote.speedFast.userFeeSat + feeQuote.speedFast.l1BroadcastFeeSat
    print("Slow fee: \(slowFeeSats) sats")
    print("Medium fee: \(mediumFeeSats) sats")
    print("Fast fee: \(fastFeeSats) sats")
}
```



### Spark

#### Spark address

For Spark addresses, the amount must be set in the request. Sending to a Spark address uses a direct Spark transfer.

```swift
let paymentRequest = "<spark address>"
// Set the amount you wish to pay the receiver
let amountSats = BInt(50_000)

let prepareResponse = try await sdk.prepareSendPayment(
    request: PrepareSendPaymentRequest(
        paymentRequest: .input(input: paymentRequest),
        amount: amountSats,
        tokenIdentifier: nil,
        conversionOptions: nil,
        feePolicy: nil
    ))

if case let .sparkAddress(_, feeSats, _) = prepareResponse.paymentMethod {
    print("Fees: \(feeSats) sats")
}
```



#### Spark invoice

For Spark invoices, the amount can be optionally set. It is only required if the invoice doesn't specify an amount. If the invoice specifies an amount, providing a different amount is not supported.

**Developer note**

Spark invoices may require a token (non-Bitcoin) as the payment asset. To determine the requirements of a Spark invoice and any restrictions it may impose, see the <a href="./parse.md">Parsing inputs</a> page. To learn more about tokens, see the <a href="./tokens.md">Handling tokens</a> page.

```swift
let paymentRequest = "<spark invoice>"
// Optionally set the amount you wish to pay the receiver
let optionalAmountSats: BInt? = BInt(50_000)

let prepareResponse = try await sdk.prepareSendPayment(
    request: PrepareSendPaymentRequest(
        paymentRequest: .input(input: paymentRequest),
        amount: optionalAmountSats,
        tokenIdentifier: nil,
        conversionOptions: nil,
        feePolicy: nil
    ))

if case let .sparkInvoice(_, feeSats, _) = prepareResponse.paymentMethod {
    print("Fees: \(feeSats) sats")
}
```



### USDC/USDT

Send USDC or USDT from a Spark wallet to a recipient on one of several supported chains: Ethereum-family chains (Arbitrum, Base, and similar EVM networks), Solana, and Tron. The source on the Spark side is BTC sats or USDB. This feature must be enabled in [the SDK configuration](./config.md#send-usdc-usdt) before using. See [Send USDC/USDT](./cross_chain.md) for provider details and the status lifecycle.

After [parsing](./parse.md) the recipient address into `InputType.crossChainAddress`, call `getCrossChainRoutes` with `CrossChainRouteFilter.send` carrying the parsed `CrossChainAddressDetails`. The returned `CrossChainRoutePair`s name the provider, destination chain and asset, decimals, optional token contract address, and which source assets (BTC sats or USDB) each route accepts.

```swift
let input = "<recipient address>"
let parsed = try await sdk.parse(input: input)
guard case let .crossChainAddress(v1: addressDetails) = parsed else {
    throw NSError(domain: "CrossChain", code: 1)
}

let routes = try await sdk.getCrossChainRoutes(
    filter: .send(addressDetails: addressDetails))

for route in routes {
    print("Route via \(route.provider): \(route.chain)/\(route.asset)")
}
```



Build `PaymentRequest.crossChain` with the recipient address, the chosen route, and an optional `maxSlippageBps` (10 to 500 basis points). The amount on the prepare request is denominated in the source asset's base units: sats for a BTC source, USDB base units for a USDB source.

The prepare response carries a quote `expiresAt` timestamp. Re-prepare and pick a fresh route if it lapses before send.

```swift
// Optionally set the maximum slippage in basis points (10 to 500)
let optionalMaxSlippageBps: UInt32? = 100

let prepareResponse = try await sdk.prepareSendPayment(
    request: PrepareSendPaymentRequest(
        paymentRequest: .crossChain(
            address: addressDetails.address,
            route: route,
            maxSlippageBps: optionalMaxSlippageBps,
            targetOverpayBps: nil
        ),
        amount: BInt(50_000),
        tokenIdentifier: nil,
        conversionOptions: nil,
        feePolicy: nil
    ))

if case let .crossChainAddress(
    _, _, amountIn, _, estimatedOut, feeAmount, _, _, _, _, expiresAt, _
) = prepareResponse.paymentMethod {
    print("Amount in: \(amountIn)")
    print("Estimated out: \(estimatedOut)")
    print("Provider fee: \(feeAmount)")
    print("Quote expires at: \(expiresAt)")
}
```



## Fee Policy

By default, fees are added on top of the amount (`FeePolicy.feesExcluded`). Use `FeePolicy.feesIncluded` to deduct fees from the amount instead—the receiver gets the amount minus fees.

This is particularly useful when you want to spend your entire balance in a single payment—simply provide your full balance as the amount. Note: `FeePolicy.feesIncluded` is not compatible with payment requests that specify an amount (e.g., BOLT11 invoices and Spark invoices with amount).

```swift
// By default (.feesExcluded), fees are added on top of the amount.
// Use .feesIncluded to deduct fees from the amount instead.
// The receiver gets amount minus fees.
let paymentRequest = "<payment request>"
let amountSats: BInt? = BInt(50_000)

let prepareResponse = try await sdk.prepareSendPayment(
    request: PrepareSendPaymentRequest(
        paymentRequest: .input(input: paymentRequest),
        amount: amountSats,
        tokenIdentifier: nil,
        conversionOptions: nil,
        feePolicy: .feesIncluded
    ))

// The response shows the fee policy used
print("Fee policy: \(String(describing: prepareResponse.feePolicy))")
print("Amount: \(String(describing: prepareResponse.amount))")
// The receiver gets amount - fees (fees are available in prepareResponse.paymentMethod)
```



When [stable balance](./stable_balance.md) is active, you can send your entire wallet balance — both the token balance and any remaining sats — by combining `FeePolicy.feesIncluded` with `ConversionType.toBitcoin` conversion options. See [Sending entire balance](./stable_balance.md#sending-entire-balance) for details.

```swift
let paymentRequest = "<payment request>"
let tokenIdentifier = "<token identifier>"

let info = try await sdk.getInfo(
    request: GetInfoRequest(ensureSynced: false))

guard let tokenBalance = info.tokenBalances[tokenIdentifier] else {
    throw SdkError.InvalidInput("Token balance not found")
}

let conversionOptions = ConversionOptions(
    conversionType: ConversionType.toBitcoin(
        fromTokenIdentifier: tokenIdentifier
    ),
    maxSlippageBps: nil,
    completionTimeoutSecs: nil
)

let prepareResponse = try await sdk.prepareSendPayment(
    request: PrepareSendPaymentRequest(
        paymentRequest: .input(input: paymentRequest),
        amount: tokenBalance.balance,
        tokenIdentifier: tokenIdentifier,
        conversionOptions: conversionOptions,
        feePolicy: .feesIncluded
    ))

// The response amount is the estimated total sats available
// (converted sats + existing sat balance)
print("Total sats available: \(prepareResponse.amount)")

if let conversionEstimate = prepareResponse.conversionEstimate {
    print(
        "Converting \(conversionEstimate.amountIn) token units "
            + "→ ~\(conversionEstimate.amountOut) sats")
    print("Conversion fee: \(conversionEstimate.fee) token units")
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

```swift
let options = SendPaymentOptions.bolt11Invoice(preferSpark: false, completionTimeoutSecs: 10)
let optionalIdempotencyKey = "<idempotency key uuid>"
let sendResponse = try await sdk.sendPayment(
    request: SendPaymentRequest(
        prepareResponse: prepareResponse,
        options: options,
        idempotencyKey: optionalIdempotencyKey
    ))
let payment = sendResponse.payment
```



### Bitcoin

In the optional send payment options for Bitcoin addresses, you can set:

- **Confirmation Speed** - The priority that the Bitcoin transaction confirms, that also effects the fee paid. By default, it is set to Fast.

```swift
// Select the confirmation speed for the on-chain transaction
let options = SendPaymentOptions.bitcoinAddress(
    confirmationSpeed: OnchainConfirmationSpeed.medium
)
let optionalIdempotencyKey = "<idempotency key uuid>"
let sendResponse = try await sdk.sendPayment(
    request: SendPaymentRequest(
        prepareResponse: prepareResponse,
        options: options,
        idempotencyKey: optionalIdempotencyKey
    ))
let payment = sendResponse.payment
```



### Spark

In the optional send payment options for Spark addresses, you can set:

- **HTLC Options** - Enables Spark HTLC payments, which are an advanced feature that allows for conditional payments. See the [Spark HTLC Payments](htlcs.md) page for more details and example usage.

```swift
let optionalIdempotencyKey = "<idempotency key uuid>"
let sendResponse = try await sdk.sendPayment(
    request: SendPaymentRequest(
        prepareResponse: prepareResponse,
        idempotencyKey: optionalIdempotencyKey
    ))
let payment = sendResponse.payment
```



### USDC/USDT

Send USDC/USDT has no additional send payment options.

```swift
// Only valid for sends with no token leg (see Retry safety).
let optionalIdempotencyKey = "<idempotency key uuid>"
let sendResponse = try await sdk.sendPayment(
    request: SendPaymentRequest(
        prepareResponse: prepareResponse,
        options: nil,
        idempotencyKey: optionalIdempotencyKey
    ))
let payment = sendResponse.payment
print(payment)
```



## Event Flows

Once a send payment is initiated, you can follow and react to the different payment events using the guide below for each payment method. See [listening to events](/guide/events.html) for how to subscribe to events. 

The `SdkEvent.synced` event is also emitted as the SDK syncs in the background. See [fetching the balance](/llms/swift/guide/get_info.md) for the recommended pattern for refreshing the balance and payments list.

#### Lightning

| Event                | Description                                                                       | UX Suggestion                                    |
| -------------------- | --------------------------------------------------------------------------------- | ------------------------------------------------ |
| **PaymentPending**   | The Spark transfer has been started. Awaiting Lightning payment completion.       | Show payment as pending.                         |
| **PaymentSucceeded** | The Lightning invoice has been paid either over Lightning or via a Spark transfer | Show the payment as complete and call `getInfo` to read the updated balance. The SDK refreshes the cached balance before emitting this event. See [fetching the balance](/llms/swift/guide/get_info.md). |
| **PaymentFailed**    | The attempt to pay the Lightning invoice failed.                                  |                                                  |

#### Bitcoin

| Event                | Description                                                                   | UX Suggestion                                    |
| -------------------- | ----------------------------------------------------------------------------- | ------------------------------------------------ |
| **PaymentPending**   | The Spark transfer has been started. Awaiting on-chain withdrawal completion. | Show payment as pending.                         |
| **PaymentSucceeded** | The payment amount was successfully withdrawn on-chain.                       | Show the payment as complete and call `getInfo` to read the updated balance. The SDK refreshes the cached balance before emitting this event. See [fetching the balance](/llms/swift/guide/get_info.md). |

#### Spark

| Event                | Description                     | UX Suggestion                                    |
| -------------------- | ------------------------------- | ------------------------------------------------ |
| **PaymentSucceeded** | The Spark transfer is complete. | Show the payment as complete and call `getInfo` to read the updated balance. The SDK refreshes the cached balance before emitting this event. See [fetching the balance](/llms/swift/guide/get_info.md). |

#### USDC/USDT

| Event                | Description                                                                                              | UX Suggestion                                    |
| -------------------- | -------------------------------------------------------------------------------------------------------- | ------------------------------------------------ |
| **PaymentPending**   | The deposit transfer has been submitted to the provider. The cross-chain leg is awaiting settlement.     | Show payment as pending; the bridge leg may take several minutes depending on the provider and destination chain. |
| **PaymentSucceeded** | The provider reports the cross-chain order terminal. The amount actually delivered to the recipient is carried on the conversion info. | Show the payment as complete and call `getInfo` to read the updated balance. The SDK refreshes the cached balance before emitting this event. See [fetching the balance](/llms/swift/guide/get_info.md). |
