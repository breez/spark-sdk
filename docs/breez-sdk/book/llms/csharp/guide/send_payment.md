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

```csharp
var paymentRequest = "<bolt11 invoice>";
// Optionally set the amount you wish to pay the receiver
ulong? optionalAmountSats = 5_000UL;

var request = new PrepareSendPaymentRequest(
    paymentRequest: new PaymentRequest.Input(input: paymentRequest),
    amount: optionalAmountSats,
    tokenIdentifier: null,
    conversionOptions: null,
    feePolicy: null
);
var prepareResponse = await sdk.PrepareSendPayment(request: request);

// If the fees are acceptable, continue to create the Send Payment
if (prepareResponse.paymentMethod is SendPaymentMethod.Bolt11Invoice bolt11Method)
{
    // Fees to pay via Lightning
    var lightningFeeSats = bolt11Method.lightningFeeSats;
    // Or fees to pay (if available) via a Spark transfer
    var sparkTransferFeeSats = bolt11Method.sparkTransferFeeSats;
    Console.WriteLine($"Lightning Fees: {lightningFeeSats} sats");
    Console.WriteLine($"Spark Transfer Fees: {sparkTransferFeeSats} sats");
}
```



### Bitcoin

For Bitcoin addresses, the amount must be set in the request. The prepare response includes fee quotes for three payment speeds: Slow, Medium, and Fast.

```csharp
var paymentRequest = "<bitcoin address>";
// Set the amount you wish to pay the receiver
ulong? amountSats = 50_000UL;

var request = new PrepareSendPaymentRequest(
    paymentRequest: new PaymentRequest.Input(input: paymentRequest),
    amount: amountSats,
    tokenIdentifier: null,
    conversionOptions: null,
    feePolicy: null
);
var prepareResponse = await sdk.PrepareSendPayment(request: request);

// Review the fee quote for each confirmation speed
if (prepareResponse.paymentMethod is SendPaymentMethod.BitcoinAddress bitcoinMethod)
{
    var feeQuote = bitcoinMethod.feeQuote;
    var slowFeeSats = feeQuote.speedSlow.userFeeSat + feeQuote.speedSlow.l1BroadcastFeeSat;
    var mediumFeeSats = feeQuote.speedMedium.userFeeSat + feeQuote.speedMedium.l1BroadcastFeeSat;
    var fastFeeSats = feeQuote.speedFast.userFeeSat + feeQuote.speedFast.l1BroadcastFeeSat;
    Console.WriteLine($"Slow fee: {slowFeeSats} sats");
    Console.WriteLine($"Medium fee: {mediumFeeSats} sats");
    Console.WriteLine($"Fast fee: {fastFeeSats} sats");
}
```



### Spark

#### Spark address

For Spark addresses, the amount must be set in the request. Sending to a Spark address uses a direct Spark transfer.

```csharp
var paymentRequest = "<spark address>";
// Set the amount you wish to pay the receiver
ulong? amountSats = 50_000UL;

var request = new PrepareSendPaymentRequest(
    paymentRequest: new PaymentRequest.Input(input: paymentRequest),
    amount: amountSats,
    tokenIdentifier: null,
    conversionOptions: null,
    feePolicy: null
);
var prepareResponse = await sdk.PrepareSendPayment(request: request);

// If the fees are acceptable, continue to create the Send Payment
if (prepareResponse.paymentMethod is SendPaymentMethod.SparkAddress sparkMethod)
{
    var fee = sparkMethod.fee;
    Console.WriteLine($"Fees: {fee} sats");
}
```



#### Spark invoice

For Spark invoices, the amount can be optionally set. It is only required if the invoice doesn't specify an amount. If the invoice specifies an amount, providing a different amount is not supported.

**Developer note**

Spark invoices may require a token (non-Bitcoin) as the payment asset. To determine the requirements of a Spark invoice and any restrictions it may impose, see the <a href="./parse.md">Parsing inputs</a> page. To learn more about tokens, see the <a href="./tokens.md">Handling tokens</a> page.

```csharp
var paymentRequest = "<spark invoice>";
// Optionally set the amount you wish to pay the receiver
ulong? optionalAmountSats = 50_000UL;

var request = new PrepareSendPaymentRequest(
    paymentRequest: new PaymentRequest.Input(input: paymentRequest),
    amount: optionalAmountSats,
    tokenIdentifier: null,
    conversionOptions: null,
    feePolicy: null
);
var prepareResponse = await sdk.PrepareSendPayment(request: request);

// If the fees are acceptable, continue to create the Send Payment
if (prepareResponse.paymentMethod is SendPaymentMethod.SparkInvoice sparkInvoiceMethod)
{
    var fee = sparkInvoiceMethod.fee;
    Console.WriteLine($"Fees: {fee} sats");
}
```



### USDC/USDT

Send USDC or USDT from a Spark wallet to a recipient on one of several supported chains: Ethereum-family chains (Arbitrum, Base, and similar EVM networks), Solana, and Tron. The source on the Spark side is BTC sats or USDB. This feature must be enabled in [the SDK configuration](./config.md#send-usdc-usdt) before using. See [Send USDC/USDT](./cross_chain.md) for provider details and the status lifecycle.

After [parsing](./parse.md) the recipient address into `InputType.CrossChainAddress`, call `GetCrossChainRoutes` with `CrossChainRouteFilter.Send` carrying the parsed `CrossChainAddressDetails`. The returned `CrossChainRoutePair`s name the provider, destination chain and asset, decimals, optional token contract address, and which source assets (BTC sats or USDB) each route accepts.

```csharp
var inputStr = "<recipient address>";
var parsed = await sdk.Parse(input: inputStr);
if (parsed is not InputType.CrossChainAddress crossChain)
{
    throw new InvalidOperationException("Not a cross-chain address");
}
var addressDetails = crossChain.v1;

var filter = new CrossChainRouteFilter.Send(addressDetails: addressDetails);
var routes = await sdk.GetCrossChainRoutes(filter: filter);

foreach (var route in routes)
{
    Console.WriteLine($"Route via {route.provider}: {route.chain}/{route.asset}");
}
```



Build `PaymentRequest.CrossChain` with the recipient address, the chosen route, and an optional `MaxSlippageBps` (10 to 500 basis points). The amount on the prepare request is denominated in the source asset's base units: sats for a BTC source, USDB base units for a USDB source.

The prepare response carries a quote `ExpiresAt` timestamp. Re-prepare and pick a fresh route if it lapses before send.

```csharp
// Optionally set the maximum slippage in basis points (10 to 500)
uint? optionalMaxSlippageBps = 100;

var request = new PrepareSendPaymentRequest(
    paymentRequest: new PaymentRequest.CrossChain(
        address: addressDetails.address,
        route: route,
        maxSlippageBps: optionalMaxSlippageBps,
        targetOverpayBps: null
    ),
    amount: 50_000UL,
    tokenIdentifier: null,
    conversionOptions: null,
    feePolicy: null
);
var prepareResponse = await sdk.PrepareSendPayment(request: request);

if (prepareResponse.paymentMethod is SendPaymentMethod.CrossChainAddress method)
{
    Console.WriteLine($"Amount in: {method.amountIn}");
    Console.WriteLine($"Estimated out: {method.estimatedOut}");
    Console.WriteLine($"Provider fee: {method.feeAmount}");
    Console.WriteLine($"Quote expires at: {method.expiresAt}");
}
```



## Fee Policy

By default, fees are added on top of the amount (`FeePolicy.FeesExcluded`). Use `FeePolicy.FeesIncluded` to deduct fees from the amount instead—the receiver gets the amount minus fees.

This is particularly useful when you want to spend your entire balance in a single payment—simply provide your full balance as the amount. Note: `FeePolicy.FeesIncluded` is not compatible with payment requests that specify an amount (e.g., BOLT11 invoices and Spark invoices with amount).

```csharp
// By default (FeePolicy.FeesExcluded), fees are added on top of the amount.
// Use FeePolicy.FeesIncluded to deduct fees from the amount instead.
// The receiver gets amount minus fees.
var paymentRequest = "<payment request>";
ulong? amountSats = 50_000UL;

var request = new PrepareSendPaymentRequest(
    paymentRequest: new PaymentRequest.Input(input: paymentRequest),
    amount: amountSats,
    tokenIdentifier: null,
    conversionOptions: null,
    feePolicy: FeePolicy.FeesIncluded
);
var prepareResponse = await sdk.PrepareSendPayment(request: request);

// The response shows the fee policy used
Console.WriteLine($"Fee policy: {prepareResponse.feePolicy}");
Console.WriteLine($"Amount: {prepareResponse.amount}");
// The receiver gets amount - fees (fees are available in prepareResponse.paymentMethod)
```



When [stable balance](./stable_balance.md) is active, you can send your entire wallet balance — both the token balance and any remaining sats — by combining `FeePolicy.FeesIncluded` with `ConversionType.ToBitcoin` conversion options. See [Sending entire balance](./stable_balance.md#sending-entire-balance) for details.

```csharp
var paymentRequest = "<payment request>";
var tokenIdentifier = "<token identifier>";

var info = await sdk.GetInfo(request: new GetInfoRequest(ensureSynced: false));
if (!info.tokenBalances.TryGetValue(tokenIdentifier, out var tokenBalance))
{
    throw new Exception("Token balance not found");
}

var conversionOptions = new ConversionOptions(
    conversionType: new ConversionType.ToBitcoin(
        fromTokenIdentifier: tokenIdentifier
    ),
    maxSlippageBps: null,
    completionTimeoutSecs: null
);

var request = new PrepareSendPaymentRequest(
    paymentRequest: new PaymentRequest.Input(input: paymentRequest),
    amount: tokenBalance.balance,
    tokenIdentifier: tokenIdentifier,
    conversionOptions: conversionOptions,
    feePolicy: FeePolicy.FeesIncluded
);
var prepareResponse = await sdk.PrepareSendPayment(request: request);

// The response amount is the estimated total sats available
// (converted sats + existing sat balance)
Console.WriteLine($"Total sats available: {prepareResponse.amount}");

if (prepareResponse.conversionEstimate != null)
{
    Console.WriteLine("Converting " +
        $"{prepareResponse.conversionEstimate.amountIn} token units " +
        $"→ ~{prepareResponse.conversionEstimate.amountOut} sats");
    Console.WriteLine("Conversion fee: " +
        $"{prepareResponse.conversionEstimate.fee} token units");
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

```csharp
var options = new SendPaymentOptions.Bolt11Invoice(
    preferSpark: false,
    completionTimeoutSecs: 10
);
var optionalIdempotencyKey = "<idempotency key uuid>";
var request = new SendPaymentRequest(
    prepareResponse: prepareResponse,
    options: options,
    idempotencyKey: optionalIdempotencyKey
);
var sendResponse = await sdk.SendPayment(request: request);
var payment = sendResponse.payment;
```



### Bitcoin

In the optional send payment options for Bitcoin addresses, you can set:

- **Confirmation Speed** - The priority that the Bitcoin transaction confirms, that also effects the fee paid. By default, it is set to Fast.

```csharp
// Select the confirmation speed for the on-chain transaction
var options = new SendPaymentOptions.BitcoinAddress(
    confirmationSpeed: OnchainConfirmationSpeed.Medium
);
var optionalIdempotencyKey = "<idempotency key uuid>";
var request = new SendPaymentRequest(
    prepareResponse: prepareResponse,
    options: options,
    idempotencyKey: optionalIdempotencyKey
);
var sendResponse = await sdk.SendPayment(request: request);
var payment = sendResponse.payment;
```



### Spark

In the optional send payment options for Spark addresses, you can set:

- **HTLC Options** - Enables Spark HTLC payments, which are an advanced feature that allows for conditional payments. See the [Spark HTLC Payments](htlcs.md) page for more details and example usage.

```csharp
var optionalIdempotencyKey = "<idempotency key uuid>";
var request = new SendPaymentRequest(
    prepareResponse: prepareResponse,
    idempotencyKey: optionalIdempotencyKey
);
var sendResponse = await sdk.SendPayment(request: request);
var payment = sendResponse.payment;
```



### USDC/USDT

Send USDC/USDT has no additional send payment options.

```csharp
// Only valid for sends with no token leg (see Retry safety).
var optionalIdempotencyKey = "<idempotency key uuid>";
var request = new SendPaymentRequest(
    prepareResponse: prepareResponse,
    options: null,
    idempotencyKey: optionalIdempotencyKey
);
var sendResponse = await sdk.SendPayment(request: request);
Console.WriteLine($"Payment: {sendResponse.payment}");
```



## Event Flows

Once a send payment is initiated, you can follow and react to the different payment events using the guide below for each payment method. See [listening to events](/guide/events.html) for how to subscribe to events. 

The `SdkEvent.Synced` event is also emitted as the SDK syncs in the background. See [fetching the balance](/llms/csharp/guide/get_info.md) for the recommended pattern for refreshing the balance and payments list.

#### Lightning

| Event                | Description                                                                       | UX Suggestion                                    |
| -------------------- | --------------------------------------------------------------------------------- | ------------------------------------------------ |
| **PaymentPending**   | The Spark transfer has been started. Awaiting Lightning payment completion.       | Show payment as pending.                         |
| **PaymentSucceeded** | The Lightning invoice has been paid either over Lightning or via a Spark transfer | Show the payment as complete and call `GetInfo` to read the updated balance. The SDK refreshes the cached balance before emitting this event. See [fetching the balance](/llms/csharp/guide/get_info.md). |
| **PaymentFailed**    | The attempt to pay the Lightning invoice failed.                                  |                                                  |

#### Bitcoin

| Event                | Description                                                                   | UX Suggestion                                    |
| -------------------- | ----------------------------------------------------------------------------- | ------------------------------------------------ |
| **PaymentPending**   | The Spark transfer has been started. Awaiting on-chain withdrawal completion. | Show payment as pending.                         |
| **PaymentSucceeded** | The payment amount was successfully withdrawn on-chain.                       | Show the payment as complete and call `GetInfo` to read the updated balance. The SDK refreshes the cached balance before emitting this event. See [fetching the balance](/llms/csharp/guide/get_info.md). |

#### Spark

| Event                | Description                     | UX Suggestion                                    |
| -------------------- | ------------------------------- | ------------------------------------------------ |
| **PaymentSucceeded** | The Spark transfer is complete. | Show the payment as complete and call `GetInfo` to read the updated balance. The SDK refreshes the cached balance before emitting this event. See [fetching the balance](/llms/csharp/guide/get_info.md). |

#### USDC/USDT

| Event                | Description                                                                                              | UX Suggestion                                    |
| -------------------- | -------------------------------------------------------------------------------------------------------- | ------------------------------------------------ |
| **PaymentPending**   | The deposit transfer has been submitted to the provider. The cross-chain leg is awaiting settlement.     | Show payment as pending; the bridge leg may take several minutes depending on the provider and destination chain. |
| **PaymentSucceeded** | The provider reports the cross-chain order terminal. The amount actually delivered to the recipient is carried on the conversion info. | Show the payment as complete and call `GetInfo` to read the updated balance. The SDK refreshes the cached balance before emitting this event. See [fetching the balance](/llms/csharp/guide/get_info.md). |
