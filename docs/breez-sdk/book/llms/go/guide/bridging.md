# Bridging USDC/USDT

A bridge is an order that pays a recipient from another chain or over Lightning, with no funds moving through the SDK. The payer pays the cross-chain provider, and the provider delivers to the recipient: a Spark address, a Lightning address, or a USDC/USDT address on another chain.

Bridging suits a server that facilitates payments: it needs only your API key, no seed and no storage.

## Getting a bridging client

Build a `BridgingClient` from your API key with `NewBridgingClient`. Keep it for the life of your process: it caches the provider's routes and prices between calls.

```go
config := breez_sdk_spark.DefaultBridgingConfig("<your api key>")
client, err := breez_sdk_spark.NewBridgingClient(config)
if err != nil {
	return nil, err
}
```



An app that already runs the SDK, including in [server mode](./server_mode.md), can get a client from it instead. The client shares the SDK's API key, and its [cross-chain settings](./config.md#usdc-usdt) when they are set. Bridging doesn't need cross-chain payments enabled.

```go
client, err := sdk.GetBridgingClient()
if err != nil {
	return nil, err
}
```



## Creating a bridge

1. List the routes that can pay the `Recipient` with `GetRoutes`. Each `BridgeRoutePair` has a `Source`, what the payer pays, and a `Destination`, what the recipient receives. Pick one by the `Asset` and `Chain` of its sides.
2. Call `CreateBridge` with the same recipient, the route, the amount and, when needed, the payer's [refund address](#refund-address).
3. Show the payer the returned `PaymentRequest`, and store the `BridgeId` and `StatusToken` to look up the status later. The [response](#response-fields) also carries the deposit and the expected delivery.

```go
// Who to pay: a Spark address, a Lightning address or an EVM/Solana/Tron
// address.
recipient := "<recipient>"

// List the routes that pay them and pick what the payer pays in, e.g.
// USDC on Base.
routesResponse, err := client.GetRoutes(breez_sdk_spark.GetBridgeRoutesRequest{
	Recipient: recipient,
})
if err != nil {
	return err
}
routes := routesResponse.Routes
var route *breez_sdk_spark.BridgeRoutePair
for i := range routes {
	if routes[i].Source.Asset == "USDC" && routes[i].Source.Chain == "base" {
		route = &routes[i]
		break
	}
}
if route == nil {
	return errors.New("no route from USDC on Base")
}

// Ask for $10. The amount is in the recipient's stablecoin when they are
// paid in one, the payer's otherwise (6 decimals for USDC). The deposit is
// refunded to the payer's address if the recipient can't be paid.
refundAddress := "<payer address on Base>"
response, err := client.CreateBridge(breez_sdk_spark.CreateBridgeRequest{
	Recipient: recipient,
	Route:     *route,
	Amount:    new(big.Int).SetInt64(10_000_000),
	// Required to pay a Lightning address.
	RefundAddress:  &refundAddress,
	FeePolicy:      nil,
	MaxSlippageBps: nil,
})
if err != nil {
	return err
}

// Show the payer what to pay, and store the bridge id and status token to
// look up the status later.
log.Printf("Pay with: %s", response.PaymentRequest)
log.Printf("Bridge id: %s", response.BridgeId)
```



When a Cash App user pays, use [`BridgeFromCashApp`](#from-cash-app) instead.

### Recipients

| Recipient | Payer pays from | Recipient receives |
| --------- | --------------- | ------------------ |
| Spark address | USDC/USDT on another chain | Bitcoin, or a stablecoin token such as USDB |
| Lightning address, including a Cash App `$cashtag` | USDC/USDT on another chain | Bitcoin over Lightning |
| USDC/USDT address on another chain | Lightning, or USDC/USDT on any chain | USDC/USDT on that chain |

Pass the recipient as you have it: a Spark address, a Lightning address or `$cashtag`, or an address or EIP-681 URI on another chain. A recipient that can't be bridged fails with `SdkErrorInvalidInput`.

Routes move only USD stablecoins and Bitcoin. For a recipient on another chain, the address decides which routes apply: an EVM address is paid on any EVM chain, unless it names a chain or token (as an EIP-681 URI does). That can be hundreds of routes, so filter them by the sources you accept.

### Amount and fees

The `Amount` is in the base units of the stablecoin the recipient receives, per its side's `BridgeRoutePairSide.Decimals`. When the recipient receives Bitcoin, it's in the stablecoin the payer pays instead. These routes carry USD-pegged stablecoins, so at parity it is the USD value: `1_000_000` is 1 USDC (6 decimals), about $1. USDC and USDT on BNB Chain have 18 decimals.

`FeePolicy` decides who absorbs the provider fee. `FeePolicyFeesExcluded` (the default) adds it on top of what the payer pays, and `FeePolicyFeesIncluded` takes it out of the amount. The fee includes your [partner fee](./cross_chain_partner_fees.md) if you set one.

### Refund address

When the payer pays from another chain, `RefundAddress` is their address on that chain. If the recipient can't be paid, the provider refunds the payment there. It must belong to the `Source` chain's family, e.g. an EVM `0x...` address for USDC on Base.

When the payer pays over Lightning, it's a Lightning address, Cash App `$cashtag` or amountless BOLT11 invoice, refunded if the payment fails before the swap. An invoice must not expire before the bridge's `ExpiresAt`, and must still be valid when the refund is paid. Anything else is rejected with `SdkErrorInvalidInput`.

It's required to pay a Lightning address, and recommended otherwise: without one, the provider has no address to refund to.

### Response fields

`CreateBridgeResponse` carries what the payer pays, the quote, and what [tracks the bridge](#tracking-a-bridge):

| Field | Meaning |
| ----- | ------- |
| `BridgeId` | Identifies the bridge. Store it to look its status up. |
| `StatusToken` | Pass it to `GetBridgeStatus` with the `BridgeId`. |
| `PaymentRequest` | What the payer pays: an EIP-681 URI on EVM chains, a BOLT11 invoice over Lightning ([a Cash App link](#from-cash-app) from `BridgeFromCashApp`), otherwise the bare deposit address. |
| `Info.DepositAddress` | The deposit address on the source chain, or the BOLT11 invoice over Lightning. |
| `Info.DepositAmount` | The amount the payer pays, in the source asset's base units (sats over Lightning). |
| `Info.ExpectedReceivedAmount` | The amount the recipient is expected to receive, in `Info.DestinationAsset` base units (sats for Bitcoin). |
| `Info.DestinationAsset` | The asset the recipient receives, e.g. `USDC`, `USDB` or `BTC`. |
| `Info.TokenIdentifier` | The Spark token identifier when the recipient receives a Spark token such as USDB. |
| `Info.ServiceFeeAmount` | Provider fee, including your [partner fee](./cross_chain_partner_fees.md) if you set one, in `Info.ServiceFeeAsset` units. |
| `Info.ServiceFeeAsset` | Denomination of the fee. Absent means the fee is in sats. |
| `Info.ServiceFeeAssetDecimals` | Decimals of `Info.ServiceFeeAsset`, for formatting the fee. Absent when the fee is in sats or the provider did not report them. |
| `Info.ExpiresAt` | Quote expiry, in seconds since the Unix epoch. Create the bridge again for a fresh quote if it lapses before the payer pays. |

A Solana transfer built in code must also create the deposit address's token account, which wallet apps do on their own. See [Paying from Solana](./cross_chain.md#paying-from-solana).

## Cash App

Two functions create a bridge with a Cash App user on one side. They take the same `CreateBridgeRequest` as `CreateBridge`, return a `CreateBridgeResponse`, and are [tracked](#tracking-a-bridge) the same way.

### From Cash App

When a Cash App user pays a USDC/USDT recipient, call `BridgeFromCashApp` with a route whose `Source` is on `lightning`. `CreateBridge` doesn't do this for you: on the same route it returns a plain BOLT11 invoice. `BridgeFromCashApp` differs in three ways:

- Its `PaymentRequest` is a `cash.app` link that opens Cash App on the invoice.
- The provider's order exists from the start. The bridge can be paid until its `ExpiresAt`, then becomes `BridgeStatusExpired`.
- The provider allows up to 10% of price movement, whatever `MaxSlippageBps` says. See [Slippage and refunds](./cash_app_to_usdc_usdt.md#slippage-and-refunds).

See [Cash App to USDC/USDT](./cash_app_to_usdc_usdt.md) for a walkthrough.

### To Cash App

To pay a Cash App user from USDC/USDT on another chain, call `BridgeToCashApp`. It creates the same bridge as `CreateBridge` with the user's `$cashtag`, and fails with `SdkErrorInvalidInput` for a recipient who isn't a Cash App user. As for any Lightning address, a [refund address](#refund-address) is required.

See [USDC/USDT to Cash App](./usdc_usdt_to_cash_app.md) for a walkthrough.

## Tracking a bridge

Read a bridge's status with `GetBridgeStatus`, passing the `BridgeId` and `StatusToken` that the bridge was created with.

```go
response, err := client.GetBridgeStatus(breez_sdk_spark.GetBridgeStatusRequest{
	BridgeId:    bridgeId,
	StatusToken: statusToken,
})
if err != nil {
	return err
}
switch response.Status {
case breez_sdk_spark.BridgeStatusCompleted:
	if response.AmountOut != nil {
		log.Printf("Delivered %v", *response.AmountOut)
	}
case breez_sdk_spark.BridgeStatusRefunded,
	breez_sdk_spark.BridgeStatusFailed,
	breez_sdk_spark.BridgeStatusExpired:
	log.Printf("Not delivered: %v", response.Status)
default:
	log.Printf("Not final yet: %v", response.Status)
}
```



| Status | Meaning |
| ------ | ------- |
| `BridgeStatusCreated` | The bridge was created, and the payer hasn't paid yet. A bridge from Cash App becomes `BridgeStatusExpired` once it can no longer be paid. Other unpaid bridges stay `BridgeStatusCreated` after their `ExpiresAt`, and a late deposit on another chain may still be accepted at a new price. Stop polling when you no longer expect a payment. |
| `BridgeStatusPending` | The provider is processing the payment, from confirming the deposit to paying the recipient. `ProviderStatus` gives the stage. |
| `BridgeStatusRefunding` | The recipient can't be paid, and the payment is being refunded. |
| `BridgeStatusCompleted` | The recipient was paid. `AmountOut` and `DeliveryTxHash` record the delivery. |
| `BridgeStatusRefunded` | The payment was refunded to the `RefundAddress`. |
| `BridgeStatusFailed` | The bridge failed. `ErrorCode` gives the reason. |
| `BridgeStatusExpired` | The bridge can no longer be paid, and nobody paid it. |

## Limitations

- **Mainnet only.** The cross-chain provider operates against live networks.
- **Polling only.** Bridges send no events. Look the status up when you need it.
