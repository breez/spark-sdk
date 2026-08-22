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

```go
paymentRequest := "<bolt11 invoice>"
// Optionally set the amount you wish to pay the receiver
optionalAmountSats := new(big.Int).SetInt64(5_000)

request := breez_sdk_spark.PrepareSendPaymentRequest{
	PaymentRequest:    breez_sdk_spark.PaymentRequestInput{Input: paymentRequest},
	Amount:            &optionalAmountSats,
	TokenIdentifier:   nil,
	ConversionOptions: nil,
	FeePolicy:         nil,
}
response, err := sdk.PrepareSendPayment(request)

if err != nil {
	var sdkErr *breez_sdk_spark.SdkError
	if errors.As(err, &sdkErr) {
		// Handle SdkError - can inspect specific variants if needed
		// e.g., switch on sdkErr variant for InsufficientFunds, NetworkError, etc.
	}
	return nil, err
}

// If the fees are acceptable, continue to create the Send Payment
switch paymentMethod := response.PaymentMethod.(type) {
case breez_sdk_spark.SendPaymentMethodBolt11Invoice:
	// Fees to pay via Lightning
	lightningFeeSats := paymentMethod.LightningFeeSats
	// Or fees to pay (if available) via a Spark transfer
	sparkTransferFeeSats := paymentMethod.SparkTransferFeeSats
	log.Printf("Lightning Fees: %v sats", lightningFeeSats)
	log.Printf("Spark Transfer Fees: %v sats", sparkTransferFeeSats)
}
```



### Bitcoin

For Bitcoin addresses, the amount must be set in the request. The prepare response includes fee quotes for three payment speeds: Slow, Medium, and Fast.

```go
paymentRequest := "<bitcoin address>"
// Set the amount you wish to pay the receiver
amountSats := new(big.Int).SetInt64(50_000)

request := breez_sdk_spark.PrepareSendPaymentRequest{
	PaymentRequest:    breez_sdk_spark.PaymentRequestInput{Input: paymentRequest},
	Amount:            &amountSats,
	TokenIdentifier:   nil,
	ConversionOptions: nil,
	FeePolicy:         nil,
}
response, err := sdk.PrepareSendPayment(request)

if err != nil {
	var sdkErr *breez_sdk_spark.SdkError
	if errors.As(err, &sdkErr) {
		// Handle SdkError - can inspect specific variants if needed
		// e.g., switch on sdkErr variant for InsufficientFunds, NetworkError, etc.
	}
	return nil, err
}

// Review the fee quote for each confirmation speed
switch paymentMethod := response.PaymentMethod.(type) {
case breez_sdk_spark.SendPaymentMethodBitcoinAddress:
	feeQuote := paymentMethod.FeeQuote
	slowFeeSats := feeQuote.SpeedSlow.UserFeeSat + feeQuote.SpeedSlow.L1BroadcastFeeSat
	mediumFeeSats := feeQuote.SpeedMedium.UserFeeSat + feeQuote.SpeedMedium.L1BroadcastFeeSat
	fastFeeSats := feeQuote.SpeedFast.UserFeeSat + feeQuote.SpeedFast.L1BroadcastFeeSat
	log.Printf("Slow fee: %v sats", slowFeeSats)
	log.Printf("Medium fee: %v sats", mediumFeeSats)
	log.Printf("Fast fee: %v sats", fastFeeSats)
}
```



### Spark

#### Spark address

For Spark addresses, the amount must be set in the request. Sending to a Spark address uses a direct Spark transfer.

```go
paymentRequest := "<spark address>"
// Set the amount you wish to pay the receiver
amountSats := new(big.Int).SetInt64(50_000)

request := breez_sdk_spark.PrepareSendPaymentRequest{
	PaymentRequest:    breez_sdk_spark.PaymentRequestInput{Input: paymentRequest},
	Amount:            &amountSats,
	TokenIdentifier:   nil,
	ConversionOptions: nil,
	FeePolicy:         nil,
}
response, err := sdk.PrepareSendPayment(request)

if err != nil {
	var sdkErr *breez_sdk_spark.SdkError
	if errors.As(err, &sdkErr) {
		// Handle SdkError - can inspect specific variants if needed
		// e.g., switch on sdkErr variant for InsufficientFunds, NetworkError, etc.
	}
	return nil, err
}

// If the fees are acceptable, continue to create the Send Payment
switch paymentMethod := response.PaymentMethod.(type) {
case breez_sdk_spark.SendPaymentMethodSparkAddress:
	feeSats := paymentMethod.Fee
	log.Printf("Fees: %v sats", feeSats)
}
```



#### Spark invoice

For Spark invoices, the amount can be optionally set. It is only required if the invoice doesn't specify an amount. If the invoice specifies an amount, providing a different amount is not supported.

**Developer note**

Spark invoices may require a token (non-Bitcoin) as the payment asset. To determine the requirements of a Spark invoice and any restrictions it may impose, see the <a href="./parse.md">Parsing inputs</a> page. To learn more about tokens, see the <a href="./tokens.md">Handling tokens</a> page.

```go
paymentRequest := "<spark invoice>"
// Optionally set the amount you wish to pay the receiver
optionalAmountSats := new(big.Int).SetInt64(50_000)

request := breez_sdk_spark.PrepareSendPaymentRequest{
	PaymentRequest:    breez_sdk_spark.PaymentRequestInput{Input: paymentRequest},
	Amount:            &optionalAmountSats,
	TokenIdentifier:   nil,
	ConversionOptions: nil,
	FeePolicy:         nil,
}
response, err := sdk.PrepareSendPayment(request)

if err != nil {
	var sdkErr *breez_sdk_spark.SdkError
	if errors.As(err, &sdkErr) {
		// Handle SdkError - can inspect specific variants if needed
		// e.g., switch on sdkErr variant for InsufficientFunds, NetworkError, etc.
	}
	return nil, err
}

// If the fees are acceptable, continue to create the Send Payment
switch paymentMethod := response.PaymentMethod.(type) {
case breez_sdk_spark.SendPaymentMethodSparkInvoice:
	feeSats := paymentMethod.Fee
	log.Printf("Fees: %v sats", feeSats)
}
```



### USDC/USDT

Send USDC or USDT from a Spark wallet to a recipient on one of several supported chains: Ethereum-family chains (Arbitrum, Base, and similar EVM networks), Solana, and Tron. The source on the Spark side is BTC sats or USDB. This feature must be enabled in [the SDK configuration](./config.md#send-usdc-usdt) before using. See [Send USDC/USDT](./cross_chain.md) for provider details and the status lifecycle.

After [parsing](./parse.md) the recipient address into `InputTypeCrossChainAddress`, call `GetCrossChainRoutes` with `CrossChainRouteFilterSend` carrying the parsed `CrossChainAddressDetails`. The returned `CrossChainRoutePair`s name the provider, destination chain and asset, decimals, optional token contract address, and which source assets (BTC sats or USDB) each route accepts.

```go
inputStr := "<recipient address>"
input, err := sdk.Parse(inputStr)
if err != nil {
	var sdkErr *breez_sdk_spark.SdkError
	if errors.As(err, &sdkErr) {
		// Handle SdkError
	}
	return nil, err
}

addressInput, ok := input.(breez_sdk_spark.InputTypeCrossChainAddress)
if !ok {
	return nil, errors.New("not a cross-chain address")
}
addressDetails := addressInput.Field0

filter := breez_sdk_spark.CrossChainRouteFilterSend{AddressDetails: addressDetails}
routes, err := sdk.GetCrossChainRoutes(filter)
if err != nil {
	return nil, err
}

for _, route := range routes {
	log.Printf("Route via %v: %s/%s", route.Provider, route.Chain, route.Asset)
}
```



Build `PaymentRequestCrossChain` with the recipient address, the chosen route, and an optional `MaxSlippageBps` (10 to 500 basis points). The amount on the prepare request is denominated in the source asset's base units: sats for a BTC source, USDB base units for a USDB source.

The prepare response carries a quote `ExpiresAt` timestamp. Re-prepare and pick a fresh route if it lapses before send.

```go
// Optionally set the maximum slippage in basis points (10 to 500)
optionalMaxSlippageBps := uint32(100)
amount := new(big.Int).SetInt64(50_000)

request := breez_sdk_spark.PrepareSendPaymentRequest{
	PaymentRequest: breez_sdk_spark.PaymentRequestCrossChain{
		Address:           addressDetails.Address,
		Route:             route,
		MaxSlippageBps:    &optionalMaxSlippageBps,
		TargetOverpayBps:  nil,
	},
	Amount:            &amount,
	TokenIdentifier:   nil,
	ConversionOptions: nil,
	FeePolicy:         nil,
}
response, err := sdk.PrepareSendPayment(request)
if err != nil {
	return nil, err
}

switch paymentMethod := response.PaymentMethod.(type) {
case breez_sdk_spark.SendPaymentMethodCrossChainAddress:
	log.Printf("Amount in: %v", paymentMethod.AmountIn)
	log.Printf("Estimated out: %v", paymentMethod.EstimatedOut)
	log.Printf("Provider fee: %v", paymentMethod.FeeAmount)
	log.Printf("Quote expires at: %s", paymentMethod.ExpiresAt)
}
```



## Fee Policy

By default, fees are added on top of the amount (`FeePolicyFeesExcluded`). Use `FeePolicyFeesIncluded` to deduct fees from the amount instead—the receiver gets the amount minus fees.

This is particularly useful when you want to spend your entire balance in a single payment—simply provide your full balance as the amount. Note: `FeePolicyFeesIncluded` is not compatible with payment requests that specify an amount (e.g., BOLT11 invoices and Spark invoices with amount).

```go
// By default (FeePolicyFeesExcluded), fees are added on top of the amount.
// Use FeePolicyFeesIncluded to deduct fees from the amount instead.
// The receiver gets amount minus fees.
paymentRequest := "<payment request>"
amountSats := new(big.Int).SetInt64(50_000)
feePolicy := breez_sdk_spark.FeePolicyFeesIncluded

request := breez_sdk_spark.PrepareSendPaymentRequest{
	PaymentRequest:    breez_sdk_spark.PaymentRequestInput{Input: paymentRequest},
	Amount:            &amountSats,
	TokenIdentifier:   nil,
	ConversionOptions: nil,
	FeePolicy:         &feePolicy,
}
response, err := sdk.PrepareSendPayment(request)

if err != nil {
	var sdkErr *breez_sdk_spark.SdkError
	if errors.As(err, &sdkErr) {
		// Handle SdkError - can inspect specific variants if needed
		// e.g., switch on sdkErr variant for InsufficientFunds, NetworkError, etc.
	}
	return nil, err
}

// The response shows the fee policy used
log.Printf("Fee policy: %v", response.FeePolicy)
log.Printf("Amount: %v", response.Amount)
// The receiver gets amount - fees (fees are available in response.PaymentMethod)
```



When [stable balance](./stable_balance.md) is active, you can send your entire wallet balance — both the token balance and any remaining sats — by combining `FeePolicyFeesIncluded` with `ConversionTypeToBitcoin` conversion options. See [Sending entire balance](./stable_balance.md#sending-entire-balance) for details.

```go
paymentRequest := "<payment request>"
tokenIdentifier := "<token identifier>"

ensureSynced := false
info, err := sdk.GetInfo(breez_sdk_spark.GetInfoRequest{
	EnsureSynced: &ensureSynced,
})
if err != nil {
	return nil, err
}

tokenBalance, ok := info.TokenBalances[tokenIdentifier]
if !ok {
	return nil, errors.New("token balance not found")
}

conversionOptions := breez_sdk_spark.ConversionOptions{
	ConversionType: breez_sdk_spark.ConversionTypeToBitcoin{
		FromTokenIdentifier: tokenIdentifier,
	},
}
feePolicy := breez_sdk_spark.FeePolicyFeesIncluded

request := breez_sdk_spark.PrepareSendPaymentRequest{
	PaymentRequest:    breez_sdk_spark.PaymentRequestInput{Input: paymentRequest},
	Amount:            &tokenBalance.Balance,
	TokenIdentifier:   &tokenIdentifier,
	ConversionOptions: &conversionOptions,
	FeePolicy:         &feePolicy,
}
response, err := sdk.PrepareSendPayment(request)

if err != nil {
	var sdkErr *breez_sdk_spark.SdkError
	if errors.As(err, &sdkErr) {
		// Handle SdkError - can inspect specific variants if needed
		// e.g., switch on sdkErr variant for InsufficientFunds, NetworkError, etc.
	}
	return nil, err
}

// The response amount is the estimated total sats available
// (converted sats + existing sat balance)
log.Printf("Total sats available: %v", response.Amount)

if response.ConversionEstimate != nil {
	log.Printf(
		"Converting %v token units → ~%v sats",
		response.ConversionEstimate.AmountIn,
		response.ConversionEstimate.AmountOut,
	)
	log.Printf("Conversion fee: %v token units", response.ConversionEstimate.Fee)
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

```go
var completionTimeoutSecs uint32 = 10
var options breez_sdk_spark.SendPaymentOptions = breez_sdk_spark.SendPaymentOptionsBolt11Invoice{
	PreferSpark:           false,
	CompletionTimeoutSecs: &completionTimeoutSecs,
}

optionalIdempotencyKey := "<idempotency key uuid>"
request := breez_sdk_spark.SendPaymentRequest{
	PrepareResponse: prepareResponse,
	Options:         &options,
	IdempotencyKey:  &optionalIdempotencyKey,
}
response, err := sdk.SendPayment(request)

if err != nil {
	var sdkErr *breez_sdk_spark.SdkError
	if errors.As(err, &sdkErr) {
		// Handle SdkError - can inspect specific variants if needed
		// e.g., switch on sdkErr variant for InsufficientFunds, NetworkError, etc.
	}
	return nil, err
}

payment := response.Payment
```



### Bitcoin

In the optional send payment options for Bitcoin addresses, you can set:

- **Confirmation Speed** - The priority that the Bitcoin transaction confirms, that also effects the fee paid. By default, it is set to Fast.

```go
// Select the confirmation speed for the on-chain transaction
var options breez_sdk_spark.SendPaymentOptions = breez_sdk_spark.SendPaymentOptionsBitcoinAddress{
	ConfirmationSpeed: breez_sdk_spark.OnchainConfirmationSpeedMedium,
}
optionalIdempotencyKey := "<idempotency key uuid>"
request := breez_sdk_spark.SendPaymentRequest{
	PrepareResponse: prepareResponse,
	Options:         &options,
	IdempotencyKey:  &optionalIdempotencyKey,
}
response, err := sdk.SendPayment(request)

if err != nil {
	var sdkErr *breez_sdk_spark.SdkError
	if errors.As(err, &sdkErr) {
		// Handle SdkError - can inspect specific variants if needed
		// e.g., switch on sdkErr variant for InsufficientFunds, NetworkError, etc.
	}
	return nil, err
}

payment := response.Payment
```



### Spark

In the optional send payment options for Spark addresses, you can set:

- **HTLC Options** - Enables Spark HTLC payments, which are an advanced feature that allows for conditional payments. See the [Spark HTLC Payments](htlcs.md) page for more details and example usage.

```go
optionalIdempotencyKey := "<idempotency key uuid>"
request := breez_sdk_spark.SendPaymentRequest{
	PrepareResponse: prepareResponse,
	IdempotencyKey:  &optionalIdempotencyKey,
}
response, err := sdk.SendPayment(request)

if err != nil {
	var sdkErr *breez_sdk_spark.SdkError
	if errors.As(err, &sdkErr) {
		// Handle SdkError - can inspect specific variants if needed
		// e.g., switch on sdkErr variant for InsufficientFunds, NetworkError, etc.
	}
	return nil, err
}

payment := response.Payment
```



### USDC/USDT

Send USDC/USDT has no additional send payment options.

```go
// Only valid for sends with no token leg (see Retry safety).
optionalIdempotencyKey := "<idempotency key uuid>"
request := breez_sdk_spark.SendPaymentRequest{
	PrepareResponse: prepareResponse,
	Options:         nil,
	IdempotencyKey:  &optionalIdempotencyKey,
}
response, err := sdk.SendPayment(request)
if err != nil {
	return nil, err
}
log.Printf("Payment: %v", response.Payment)
```



## Event Flows

Once a send payment is initiated, you can follow and react to the different payment events using the guide below for each payment method. See [listening to events](/guide/events.html) for how to subscribe to events. 

The `SdkEventSynced` event is also emitted as the SDK syncs in the background. See [fetching the balance](/llms/go/guide/get_info.md) for the recommended pattern for refreshing the balance and payments list.

#### Lightning

| Event                | Description                                                                       | UX Suggestion                                    |
| -------------------- | --------------------------------------------------------------------------------- | ------------------------------------------------ |
| **PaymentPending**   | The Spark transfer has been started. Awaiting Lightning payment completion.       | Show payment as pending.                         |
| **PaymentSucceeded** | The Lightning invoice has been paid either over Lightning or via a Spark transfer | Show the payment as complete and call `GetInfo` to read the updated balance. The SDK refreshes the cached balance before emitting this event. See [fetching the balance](/llms/go/guide/get_info.md). |
| **PaymentFailed**    | The attempt to pay the Lightning invoice failed.                                  |                                                  |

#### Bitcoin

| Event                | Description                                                                   | UX Suggestion                                    |
| -------------------- | ----------------------------------------------------------------------------- | ------------------------------------------------ |
| **PaymentPending**   | The Spark transfer has been started. Awaiting on-chain withdrawal completion. | Show payment as pending.                         |
| **PaymentSucceeded** | The payment amount was successfully withdrawn on-chain.                       | Show the payment as complete and call `GetInfo` to read the updated balance. The SDK refreshes the cached balance before emitting this event. See [fetching the balance](/llms/go/guide/get_info.md). |

#### Spark

| Event                | Description                     | UX Suggestion                                    |
| -------------------- | ------------------------------- | ------------------------------------------------ |
| **PaymentSucceeded** | The Spark transfer is complete. | Show the payment as complete and call `GetInfo` to read the updated balance. The SDK refreshes the cached balance before emitting this event. See [fetching the balance](/llms/go/guide/get_info.md). |

#### USDC/USDT

| Event                | Description                                                                                              | UX Suggestion                                    |
| -------------------- | -------------------------------------------------------------------------------------------------------- | ------------------------------------------------ |
| **PaymentPending**   | The deposit transfer has been submitted to the provider. The cross-chain leg is awaiting settlement.     | Show payment as pending; the bridge leg may take several minutes depending on the provider and destination chain. |
| **PaymentSucceeded** | The provider reports the cross-chain order terminal. The amount actually delivered to the recipient is carried on the conversion info. | Show the payment as complete and call `GetInfo` to read the updated balance. The SDK refreshes the cached balance before emitting this event. See [fetching the balance](/llms/go/guide/get_info.md). |
