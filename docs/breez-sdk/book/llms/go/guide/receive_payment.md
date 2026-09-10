# Receiving payments

API docs: https://breez.github.io/spark-sdk/breez_sdk_spark/struct.BreezSdk.html#method.receive_payment

Once the SDK is initialized, you can directly begin receiving payments. The SDK supports receiving via Lightning, Bitcoin, Spark, and USDC/USDT into Spark from a supported external chain.

## Lightning

#### BOLT11 invoice

When receiving via Lightning, we can generate a BOLT11 invoice to be paid. Setting the invoice amount fixes the amount the sender should pay.

To create an invoice for another Spark wallet, set `ReceiverIdentityPublicKey` to that wallet's identity public key. Creating the invoice requires only the receiver's public key, not their private keys.

**Note:** the payment may fallback to a direct Spark payment (if the payer's client supports this).

```go
description := "<invoice description>"
// Optionally set the invoice amount you wish the payer to send
optionalAmountSats := uint64(5_000)
// Optionally set the expiry duration in seconds
optionalExpirySecs := uint32(3600)
// Set this to create an invoice for another Spark identity
var optionalReceiverIdentityPublicKey *string

request := breez_sdk_spark.ReceivePaymentRequest{
	PaymentMethod: breez_sdk_spark.ReceivePaymentMethodBolt11Invoice{
		Description: description,
		AmountSats:  &optionalAmountSats,
		ExpirySecs:  &optionalExpirySecs,
		PaymentHash: nil,
		ReceiverIdentityPublicKey: optionalReceiverIdentityPublicKey,
	},
}

response, err := sdk.ReceivePayment(request)

if err != nil {
	var sdkErr *breez_sdk_spark.SdkError
	if errors.As(err, &sdkErr) {
		// Handle SdkError - can inspect specific variants if needed
		// e.g., switch on sdkErr variant for InsufficientFunds, NetworkError, etc.
	}
	return nil, err
}

paymentRequest := response.PaymentRequest
log.Printf("Payment Request: %v", paymentRequest)
receiveFeesSat := response.Fee
log.Printf("Fees: %v sats", receiveFeesSat)
```



#### LNURL-Pay & Lightning address

To receive via LNURL-Pay and/or a Lightning address, follow [these instructions](/llms/go/guide/receive_lnurl_pay.md).

> Note: Lightning payments work in Spark even if the receiver is offline. To understand how it works under the hood, read [this](https://docs.spark.money/learn/lightning).

## Bitcoin

For on-chain payments you can generate a Bitcoin deposit address to receive payments. By default the existing address is returned; you can optionally request a new address to rotate to a fresh one for improved privacy. All previously generated addresses remain monitored.

On-chain deposits go through the following lifecycle:

1. **Detected** — The SDK detects the deposit and emits a `SdkEventNewDeposits` event. The deposit may or may not have sufficient confirmations to be claimed yet.
2. **Sufficient confirmations** — After **3 on-chain confirmations**, the deposit has sufficient confirmations and the SDK automatically attempts to claim it.
3. **Claimed or unclaimed** — If claiming succeeds, the funds are added to your balance. If it fails (e.g. fees too high), the deposit remains unclaimed and can be [manually claimed or refunded](/llms/go/guide/onchain_claims.md).

```go
var newAddress *bool // To get a new address: t := true; newAddress = &t
request := breez_sdk_spark.ReceivePaymentRequest{
	PaymentMethod: breez_sdk_spark.ReceivePaymentMethodBitcoinAddress{
		NewAddress: newAddress,
	},
}

response, err := sdk.ReceivePayment(request)

if err != nil {
	var sdkErr *breez_sdk_spark.SdkError
	if errors.As(err, &sdkErr) {
		// Handle SdkError - can inspect specific variants if needed
		// e.g., switch on sdkErr variant for InsufficientFunds, NetworkError, etc.
	}
	return nil, err
}

paymentRequest := response.PaymentRequest
log.Printf("Payment Request: %v", paymentRequest)
receiveFeesSat := response.Fee
log.Printf("Fees: %v sats", receiveFeesSat)
```



To track pending deposits, use `ListUnclaimedDeposits` and filter by the `IsMature` field:

```go
request := breez_sdk_spark.ListUnclaimedDepositsRequest{}
response, err := sdk.ListUnclaimedDeposits(request)
if err != nil {
	return err
}

var pendingDeposits []breez_sdk_spark.DepositInfo
for _, deposit := range response.Deposits {
	if !deposit.IsMature {
		pendingDeposits = append(pendingDeposits, deposit)
	}
}

for _, deposit := range pendingDeposits {
	log.Printf("Pending deposit: %v:%v", deposit.Txid, deposit.Vout)
	log.Printf("Amount: %v sats", deposit.AmountSats)
}
```



## Spark

For payments between Spark users, you can use a Spark address or generate a Spark invoice to receive payments.

#### Spark address

Spark addresses are static.

```go
request := breez_sdk_spark.ReceivePaymentRequest{
	PaymentMethod: breez_sdk_spark.ReceivePaymentMethodSparkAddress{},
}

response, err := sdk.ReceivePayment(request)

if err != nil {
	var sdkErr *breez_sdk_spark.SdkError
	if errors.As(err, &sdkErr) {
		// Handle SdkError - can inspect specific variants if needed
		// e.g., switch on sdkErr variant for InsufficientFunds, NetworkError, etc.
	}
	return nil, err
}

paymentRequest := response.PaymentRequest
log.Printf("Payment Request: %v", paymentRequest)
receiveFeesSat := response.Fee
log.Printf("Fees: %v sats", receiveFeesSat)
```



#### Spark invoice

Spark invoices are single-use and may impose restrictions on the payment, such as amount, expiry, and who is able to pay it.

```go
optionalDescription := "<invoice description>"
optionalAmountSats := new(big.Int).SetInt64(5_000)
// Optionally set the expiry UNIX timestamp in seconds
optionalExpiryTimeSeconds := uint64(1716691200)
optionalSenderPublicKey := "<sender public key>"

request := breez_sdk_spark.ReceivePaymentRequest{
	PaymentMethod: breez_sdk_spark.ReceivePaymentMethodSparkInvoice{
		Description:     &optionalDescription,
		Amount:          &optionalAmountSats,
		ExpiryTime:      &optionalExpiryTimeSeconds,
		SenderPublicKey: &optionalSenderPublicKey,
	},
}

response, err := sdk.ReceivePayment(request)

if err != nil {
	var sdkErr *breez_sdk_spark.SdkError
	if errors.As(err, &sdkErr) {
		// Handle SdkError - can inspect specific variants if needed
		// e.g., switch on sdkErr variant for InsufficientFunds, NetworkError, etc.
	}
	return nil, err
}

paymentRequest := response.PaymentRequest
log.Printf("Payment Request: %v", paymentRequest)
receiveFeesSat := response.Fee
log.Printf("Fees: %v sats", receiveFeesSat)
```



## USDC/USDT

Cross-chain receive is supported only via the Orchestra provider. Receive USDC or USDT from a sender on one of several supported chains: Ethereum-family chains (Arbitrum, Base, and similar EVM networks), Solana, and Tron. The receiver lands either BTC sats or [USDB](./stable_balance.md) (a 6-decimal USD-pegged token on Spark) on the Spark side. This feature must be enabled in [the SDK configuration](./config.md#usdc-usdt) before using. See [USDC/USDT](./cross_chain.md) for provider details and the status lifecycle.

Call `GetCrossChainRoutes` with `CrossChainRouteFilterReceive` to discover supported source assets. Each `CrossChainRoutePair` names the provider, source chain and asset, decimals, optional token contract address, and the Spark-side destinations the route lands (`CrossChainRoutePair.AcceptedAssets`).

```go
filter := breez_sdk_spark.CrossChainRouteFilterReceive{ContractAddress: nil}
routes, err := sdk.GetCrossChainRoutes(filter)
if err != nil {
	return nil, err
}

for _, route := range routes {
	log.Printf(
		"Route via %v: %s/%s -> Spark",
		route.Provider, route.Chain, route.Asset,
	)
}
```



Build `ReceivePaymentMethodCrossChain` with the chosen route and an `amount`.

The `Amount` on {{#name ReceivePaymentMethod::CrossChain}} is in the source asset's base units, per the route's `CrossChainRoutePair.Decimals`. USD-stable sources sit at USD parity, so `1_000_000` is 1 USDC (6 decimals), about $1.

`FeeMode` controls what the amount means:

- `CrossChainFeeModeFeesExcluded` (default): `amount` is the receiver's target on Spark. The SDK pads the sender's deposit to cover provider fees plus an overpay buffer.
- `CrossChainFeeModeFeesIncluded`: `amount` is the deposit the sender pays. The receiver lands `amount - fees`.

`Destination` picks which Spark-side asset the receiver wants delivered. Left unset, the SDK auto-picks the wallet's active stable-balance token if the route supports it, otherwise BTC (converted from the USD amount via the live BTC/USD rate).

`MaxSlippageBps` (10 to 500) bounds the price movement tolerated between quote and delivery. `TargetOverpayBps` (0 to 500) sets the FeesExcluded overpay buffer. Left unset, the SDK defaults apply.

The `PaymentRequest` field carries an EIP-681 URI for EVM routes and the bare deposit address for Solana and Tron. The `CrossChainInfo` block surfaces the bare deposit address, deposit amount, expected receive amount, destination denomination, and quote `ExpiresAt`. The receiver pays no fee; the sender's deposit covers it.

```go
// amount is in the route's source-asset base units (USD-stable parity:
// 1_000_000 = $1 on 6-decimal routes). See the guide for FeeMode,
// destination, and the slippage/overpay overrides.
amount := new(big.Int).SetInt64(1_000_000)
var optionalDestination *breez_sdk_spark.SparkAsset = nil
optionalMaxSlippageBps := uint32(100)
var optionalTargetOverpayBps *uint32 = nil
var optionalFeeMode *breez_sdk_spark.CrossChainFeeMode = nil

request := breez_sdk_spark.ReceivePaymentRequest{
	PaymentMethod: breez_sdk_spark.ReceivePaymentMethodCrossChain{
		Route:             route,
		Amount:            amount,
		Destination:       optionalDestination,
		FeeMode:           optionalFeeMode,
		MaxSlippageBps:    &optionalMaxSlippageBps,
		TargetOverpayBps:  optionalTargetOverpayBps,
	},
}
response, err := sdk.ReceivePayment(request)
if err != nil {
	return nil, err
}

log.Printf("Payment request: %s", response.PaymentRequest)
if info := response.CrossChainInfo; info != nil {
	log.Printf("Deposit address: %s", info.DepositAddress)
	log.Printf("Deposit amount: %v", info.DepositAmount)
	log.Printf(
		"Expected received: %v %s",
		info.ExpectedReceivedAmount, info.DestinationAsset,
	)
	log.Printf("Expires at: %d", info.ExpiresAt)
}
```



## Event Flows

Once a receive payment is initiated, you can follow and react to the different payment events using the guide below for each payment method. See [listening to events](/llms/go/guide/events.md) for how to subscribe to events. 

The `SdkEventSynced` event is also emitted as the SDK syncs in the background. See [fetching the balance](/llms/go/guide/get_info.md) for the recommended pattern for refreshing the balance and payments list.

#### Lightning

| Event                | Description                                                       | UX Suggestion                                    |
| -------------------- | ----------------------------------------------------------------- | ------------------------------------------------ |
| **PaymentPending**   | The Spark transfer was detected and the claim process will start. | Show payment as pending.                         |
| **PaymentSucceeded** | The Spark transfer is claimed and the payment is complete.        | Show the payment as complete and call `GetInfo` to read the updated balance. The SDK refreshes the cached balance before emitting this event. See [fetching the balance](/llms/go/guide/get_info.md). |

#### Bitcoin

The following events are emitted in order during the deposit lifecycle. See [Listening to events](/llms/go/guide/events.md) for how to subscribe.

| Event                 | Description                                                                                                                              | UX Suggestion                                                                                               |
| --------------------- | ---------------------------------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------- |
| **NewDeposits**       | New deposits were detected. Each deposit includes a `IsMature` field indicating whether it has enough confirmations to be claimed. | Show the deposit to the user. If it does not yet have sufficient confirmations, show it as pending.          |
| **ClaimedDeposits**   | The SDK successfully claimed confirmed deposits.                                                                                         |                                                                                                             |
| **UnclaimedDeposits** | Claiming failed (e.g. fee exceeded the configured maximum or the UTXO could not be found).                                               | Allow the user to manually claim or refund. See [Claiming on-chain deposits](/llms/go/guide/onchain_claims.md). |
| **PaymentPending**    | The Spark transfer was detected and the claim process will start.                                                                        | Show payment as pending.                                                                                    |
| **PaymentSucceeded**  | The Spark transfer is claimed and the payment is complete.                                                                               | Show the payment as complete and call `GetInfo` to read the updated balance. The SDK refreshes the cached balance before emitting this event. See [fetching the balance](/llms/go/guide/get_info.md).                                                            |

#### Spark

| Event                | Description                                                                                                                                                                                          | UX Suggestion                                    |
| -------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------ |
| **PaymentPending**   | The Spark transfer was detected and the claim process will start. For Spark HTLC payments, the claim will only start once the HTLC is claimed. For more details see [Spark HTLC payments](htlcs.md). | Show payment as pending.                         |
| **PaymentSucceeded** | The Spark transfer is claimed and the payment is complete.                                                                                                                                           | Show the payment as complete and call `GetInfo` to read the updated balance. The SDK refreshes the cached balance before emitting this event. See [fetching the balance](/llms/go/guide/get_info.md). |

#### USDC/USDT

| Event                | Description                                                                                          | UX Suggestion                                    |
| -------------------- | ---------------------------------------------------------------------------------------------------- | ------------------------------------------------ |
| **PaymentPending**   | The sender's deposit was detected and the inbound Spark transfer claim is in progress.               | Show payment as pending.                         |
| **PaymentSucceeded** | The inbound Spark transfer is claimed and the payment is complete.                                   | Show the payment as complete and call `GetInfo` to read the updated balance. The SDK refreshes the cached balance before emitting this event. See [fetching the balance](/llms/go/guide/get_info.md). |
