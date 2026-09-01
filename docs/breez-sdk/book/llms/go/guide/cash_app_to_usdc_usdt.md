# Cash App to USDC/USDT payments

`PreparePaymentLink` lets a user send USDC or USDT to a recipient on an external chain (Ethereum-family, Solana, or Tron). The user pays through Cash App (over Lightning) and a cross-chain provider delivers the stablecoin to the recipient.

The SDK acts as orchestrator only. No wallet funds move, and nothing is written to the payment history: Cash App funds the purchase externally and the provider delivers autonomously. This is distinct from the wallet-funded, tracked flow on the [Send USDC/USDT](./cross_chain.md) page.

**Developer note**

Cash App is available in the US and UK only (excluding New York State for Bitcoin/Lightning features). Cash App handles region restrictions on their end, so no client-side gating is needed.

## How it works

1. Parse the recipient's external-chain address and select a destination route.
2. Call `PreparePaymentLink` with the route and the USD amount. The SDK gets a quote from the cross-chain provider and returns a Cash App deep link plus the quote details.
3. Open the returned `Url`. The user completes payment in Cash App, and the provider delivers the stablecoin to the recipient.

Because the SDK never sees the funds, `PreparePaymentLink` returns and forgets: there is no `Payment` row, no status tracking, and no event. Surface the quote fields in the response so the app can show the user what to expect before they pay.

## Selecting a route

Parse the recipient address, then list the stablecoin destinations you can send to with `GetCrossChainRoutes` filtered by `CrossChainRouteFilterPaymentLink`. These routes are the ones fundable by an external rail (Cash App over Lightning), a different set from the wallet-funded send routes. Pick the `CrossChainRoutePair` whose `CrossChainRoutePair.Asset` and `CrossChainRoutePair.Chain` match the destination you want, and pass it to `PreparePaymentLink`.

Calling `PreparePaymentLink` with a route that can't be funded over Lightning fails fast, before any funds move.

## Preparing the payment link

Cash App funds the purchase over **Lightning**. The SDK builds a `cash.app/launch/lightning/<bolt11>` deep link from the provider's invoice and returns it as the response `Url`.

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

// List the stablecoin destinations you can send to and pick one, e.g. USDC
// on Base. These routes are funded by an external rail, not the Spark wallet.
// Restrict to Lightning-fundable routes, since Cash App funds over Lightning.
filter := breez_sdk_spark.CrossChainRouteFilterPaymentLink{
	AddressDetails: addressDetails,
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
request := breez_sdk_spark.PreparePaymentLinkRequest{
	Address:        addressDetails.Address,
	Route:          *route,
	Amount:         new(big.Int).SetInt64(10_000_000),
	FeePolicy:      nil,
	MaxSlippageBps: nil,
}
response, err := sdk.PreparePaymentLink(request)
if err != nil {
	return nil, err
}

// Open this Cash App URL to pay; the recipient then receives the stablecoin.
log.Printf("Open this URL in Cash App: %v", response.Url)
log.Printf("Recipient receives ~%v %s", response.EstimatedOut, response.Asset)
```



### Amounts and fees

The `Amount` on `PreparePaymentLinkRequest` is in the destination asset's base units, per the route's `CrossChainRoutePair.Decimals`. These routes deliver USD-pegged stablecoins, so at parity it is the USD value: `1_000_000` is 1 USDC (6 decimals), about $1.

`FeePolicy` controls who absorbs the provider fee, reusing the same `FeePolicy` as the send flow:

- `FeePolicyFeesExcluded` (default) delivers the target amount to the recipient and adds the fee on top of what the user pays.
- `FeePolicyFeesIncluded` takes the fee out of the amount, so the user pays exactly the amount and the recipient receives less.

`MaxSlippageBps` bounds the price movement tolerated between quote and delivery, in basis points (1 bps = 0.01%). Left unset, the provider default applies.

### Response fields

`PreparePaymentLinkResponse` carries the payment URL and the quote:

| Field | Meaning |
| ----- | ------- |
| `Url` | The Cash App deep link to open. |
| `AmountSats` | Bitcoin amount the user deposits through Cash App. |
| `EstimatedOut` | Expected amount delivered to the recipient, in `Asset` units. |
| `Asset` | The delivered stablecoin (e.g. `USDC`). |
| `ServiceFeeAmount` | Provider fee for the conversion. |
| `ServiceFeeAsset` | Denomination of the fee. Absent means the fee is in sats. |
| `ExpiresAt` | Quote expiry. Re-call `PreparePaymentLink` for a fresh quote if it lapses before the user starts paying. |

## Opening the payment link

On devices with Cash App installed the URL opens the app directly; otherwise it falls back to the Cash App website. The same UX guidance as the [Buying Bitcoin](./buy_bitcoin.md#recommended-ux) Cash App flow applies (mobile redirect, desktop QR code, pre-opening a tab on web to avoid popup blockers).

## Limitations

- **Mainnet only.** Cash App and the cross-chain providers operate against live networks. There is no testnet equivalent.
- **Not tracked.** The purchase is funded outside the wallet, so it produces no `Payment` row and no event. If you need delivery confirmation, observe the recipient chain directly.
- **Orchestra routes only.** Payment links quote against Orchestra, whose orders deliver without the SDK online. A Boltz route would need the wallet to stay online to claim the swap before the payer's payment settles, so `GetCrossChainRoutes` with `CrossChainRouteFilterPaymentLink` returns only Orchestra routes and `PreparePaymentLink` rejects a Boltz route.
