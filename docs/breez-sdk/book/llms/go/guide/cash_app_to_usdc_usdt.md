# Cash App to USDC/USDT

`BridgeFromCashApp` lets a user send USDC or USDT to a recipient on an external chain. The user pays through Cash App over Lightning, and the cross-chain provider delivers the stablecoin to the recipient. See [Cash App and USDC/USDT](./cash_app.md) for what the two Cash App bridges share.

## How it works

1. [Select a destination route](./cash_app.md#selecting-a-route) for the recipient's external-chain address.
2. Call `BridgeFromCashApp` with the recipient, the route and the USD amount. The SDK gets a quote from the cross-chain provider and returns a Cash App deep link plus the quote details.
3. Open the returned `PaymentRequest`. The user completes payment in Cash App, and the provider delivers the stablecoin to the recipient.

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



## Response fields

`BridgeFromCashApp` returns a `CreateBridgeResponse`, described in [Bridging USDC/USDT](./bridging.md#response-fields). Its `PaymentRequest` is the Cash App deep link to open.

## Opening the payment link

On devices with Cash App installed the URL opens the app directly. Otherwise it falls back to the Cash App website. The same UX guidance as the [Buying Bitcoin](./buy_bitcoin.md#recommended-ux) Cash App flow applies (mobile redirect, desktop QR code, pre-opening a tab on web to avoid popup blockers).
