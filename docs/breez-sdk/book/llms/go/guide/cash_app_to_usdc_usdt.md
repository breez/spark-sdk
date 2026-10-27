# Cash App to USDC/USDT

`BridgeFromCashApp` lets a user send USDC or USDT to a recipient on an external chain. The user pays through Cash App over Lightning, and the cross-chain provider delivers the stablecoin to the recipient. See [Cash App and USDC/USDT](./cash_app.md) for what the two Cash App bridges share.

## How it works

1. Parse the recipient's external-chain address and [select a destination route](./cash_app.md#selecting-a-route) with `CrossChainRouteFilterSend`.
2. Call `BridgeFromCashApp` with the route and the USD amount. The SDK gets a quote from the cross-chain provider and returns a Cash App deep link plus the quote details.
3. Open the returned `Url`. The user completes payment in Cash App, and the provider delivers the stablecoin to the recipient.

The SDK builds a `cash.app/launch/lightning/<bolt11>` deep link from the provider's invoice and returns it as the response `Url`.

```go
// Parse the recipient's external-chain address (EVM/Solana/Tron).
inputStr := "<recipient address>"
input, err := sdk.Parse(inputStr)
if err != nil {
	return nil, err
}
addressInput, ok := input.(breez_sdk_spark.InputTypeCrossChainAddress)
if !ok {
	return nil, errors.New("not a cross-chain address")
}
addressDetails := addressInput.Field0

// List the stablecoin destinations Cash App can fund over Lightning and
// pick one, e.g. USDC on Base.
deliveryMethod := breez_sdk_spark.DeliveryMethodLightning
filter := breez_sdk_spark.CrossChainRouteFilterSend{
	AddressDetails: addressDetails,
	DeliveryMethod: &deliveryMethod,
}
routes, err := sdk.GetCrossChainRoutes(filter)
if err != nil {
	return nil, err
}
var route *breez_sdk_spark.CrossChainRoutePair
for i := range routes {
	if routes[i].Asset == "USDC" && routes[i].Chain == "base" {
		route = &routes[i]
		break
	}
}
if route == nil {
	return nil, errors.New("no USDC route on Base")
}

// Send $10 of USDC, funded by Cash App over Lightning. The amount is in the
// route asset's base units (USDC, 6 decimals), so 10_000_000 = 10 USDC,
// about $10.
request := breez_sdk_spark.BridgeFromCashAppRequest{
	Address:        addressDetails.Address,
	Route:          *route,
	Amount:         new(big.Int).SetInt64(10_000_000),
	FeePolicy:      nil,
	MaxSlippageBps: nil,
}
response, err := sdk.BridgeFromCashApp(request)
if err != nil {
	return nil, err
}

// Open this Cash App URL to pay. The recipient then receives the stablecoin.
log.Printf("Open this URL in Cash App: %v", response.Url)
log.Printf("Recipient receives ~%v %s", response.EstimatedOut, response.Asset)
```



## Response fields

`BridgeFromCashAppResponse` carries the payment URL and the quote:

| Field | Meaning |
| ----- | ------- |
| `Url` | The Cash App deep link to open. |
| `AmountSats` | Bitcoin amount the user deposits through Cash App. |
| `EstimatedOut` | Expected amount delivered to the recipient, in `Asset` units. |
| `Asset` | The delivered stablecoin (e.g. `USDC`). |
| `ServiceFeeAmount` | Provider fee for the conversion, including your [partner fee](./cross_chain_partner_fees.md) if you set one. |
| `ServiceFeeAsset` | Denomination of the fee. Absent means the fee is in sats. |
| `ServiceFeeAssetDecimals` | Decimals of `ServiceFeeAsset`, for formatting the fee. Absent when the fee is in sats or the provider did not report them. |
| `ExpiresAt` | Quote expiry, in seconds since the Unix epoch. Call `BridgeFromCashApp` again for a fresh quote if it lapses before the user starts paying. |

## Opening the payment link

On devices with Cash App installed the URL opens the app directly. Otherwise it falls back to the Cash App website. The same UX guidance as the [Buying Bitcoin](./buy_bitcoin.md#recommended-ux) Cash App flow applies (mobile redirect, desktop QR code, pre-opening a tab on web to avoid popup blockers).
