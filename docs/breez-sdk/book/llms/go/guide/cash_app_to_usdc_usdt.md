# Cash App to USDC/USDT

`BridgeFromCashApp` lets a user send USDC or USDT to a recipient on an external chain. The user pays through Cash App over Lightning, and the cross-chain provider delivers the stablecoin to the recipient. See [Cash App and USDC/USDT](./cash_app.md) for what the two Cash App bridges share.

## How it works

1. Get a [bridging client](./bridging.md#getting-a-bridging-client): build one from your API key with `NewBridgingClient`, or call `GetBridgingClient` on a connected SDK.
2. [Select a destination route](./cash_app.md#selecting-a-route) for the recipient's external-chain address.
3. Call `BridgeFromCashApp` with the recipient, the route and the USD amount. The SDK creates an order with the cross-chain provider and returns a Cash App deep link plus the order details.
4. Open the returned `PaymentRequest`, and store the `BridgeId` and `StatusToken` to [track the bridge](#tracking-the-bridge). The user completes payment in Cash App, and the provider delivers the stablecoin to the recipient.

The SDK builds a `cash.app/launch/lightning/<bolt11>` deep link from the provider's invoice and returns it as the response `PaymentRequest`. The invoice itself is `Info.DepositAddress`.

```go
// List the routes to the recipient's external-chain address
// (EVM/Solana/Tron) and pick a stablecoin Cash App can fund over
// Lightning, e.g. USDC on Base.
recipient := "<recipient address>"
routesResponse, err := client.GetRoutes(breez_sdk_spark.GetBridgeRoutesRequest{
	Recipient: recipient,
})
if err != nil {
	return nil, err
}
routes := routesResponse.Routes
var route *breez_sdk_spark.BridgeRoutePair
for i := range routes {
	if routes[i].Source.Chain == "lightning" &&
		routes[i].Destination.Asset == "USDC" &&
		routes[i].Destination.Chain == "base" {
		route = &routes[i]
		break
	}
}
if route == nil {
	return nil, errors.New("no USDC route on Base")
}

// Send $10 of USDC, funded by Cash App over Lightning. The amount is in
// USDC base units (6 decimals), so 10_000_000 = 10 USDC, about $10.
response, err := client.BridgeFromCashApp(breez_sdk_spark.CreateBridgeRequest{
	Recipient:      recipient,
	Route:          *route,
	Amount:         new(big.Int).SetInt64(10_000_000),
	RefundAddress:  nil,
	FeePolicy:      nil,
	MaxSlippageBps: nil,
})
if err != nil {
	return nil, err
}

// Open this Cash App link to pay. The recipient then receives the stablecoin.
info := response.Info
log.Printf("Open this URL in Cash App: %s", response.PaymentRequest)
log.Printf("Recipient receives ~%v %s", info.ExpectedReceivedAmount, info.DestinationAsset)
```



## Slippage and refunds

The provider allows these payments up to 10% of price movement between the quote and the delivery, whatever `MaxSlippageBps` says. With fees excluded, `MaxSlippageBps` still bounds how far the quoted delivery may fall short of the `Amount`: a bridge quoted below it fails before the user sees a link. With fees included, the quoted delivery isn't checked.

If the payment fails before the swap, the provider refunds it to the [refund address](./bridging.md#refund-address): a Lightning address, Cash App `$cashtag` or amountless BOLT11 invoice. An invoice must not expire before the bridge, and must still be valid when the refund is paid. Without a refund address, the payment can't be refunded automatically.

## Response fields

`BridgeFromCashApp` returns a `CreateBridgeResponse`, described in [Bridging USDC/USDT](./bridging.md#response-fields). Its `PaymentRequest` is the Cash App deep link to open.

## Opening the payment link

On devices with Cash App installed the URL opens the app directly. Otherwise it falls back to the Cash App website. The same UX guidance as the [Buying Bitcoin](./buy_bitcoin.md#recommended-ux) Cash App flow applies (mobile redirect, desktop QR code, pre-opening a tab on web to avoid popup blockers).

## Tracking the bridge

Pass the `BridgeId` and `StatusToken` from the response to `GetBridgeStatus`. The bridge reads:

- `BridgeStatusCreated` until the user pays in Cash App.
- `BridgeStatusExpired` if nobody pays before its `ExpiresAt`. The link can't be paid after that, so create a new bridge.
- `BridgeStatusPending` once the payment arrives, while the provider swaps and delivers.
- `BridgeStatusCompleted` once the recipient has the stablecoin. `DeliveryTxHash` is the delivery transaction on the recipient's chain.
- `BridgeStatusRefunding`, then `BridgeStatusRefunded`, if the payment failed before the swap and is returned to the [refund address](#slippage-and-refunds).

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



See [Tracking a bridge](./bridging.md#tracking-a-bridge) for every status.
