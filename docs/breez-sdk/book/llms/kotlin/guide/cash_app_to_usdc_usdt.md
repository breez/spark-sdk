# Cash App to USDC/USDT

`bridgeFromCashApp` lets a user send USDC or USDT to a recipient on an external chain. The user pays through Cash App over Lightning, and the cross-chain provider delivers the stablecoin to the recipient. See [Cash App and USDC/USDT](./cash_app.md) for what the two Cash App bridges share.

## How it works

1. Parse the recipient's external-chain address and [select a destination route](./cash_app.md#selecting-a-route) with `CrossChainRouteFilter.Send`.
2. Call `bridgeFromCashApp` with the route and the USD amount. The SDK gets a quote from the cross-chain provider and returns a Cash App deep link plus the quote details.
3. Open the returned `url`. The user completes payment in Cash App, and the provider delivers the stablecoin to the recipient.

The SDK builds a `cash.app/launch/lightning/<bolt11>` deep link from the provider's invoice and returns it as the response `url`.

```kotlin
// Parse the recipient's external-chain address (EVM/Solana/Tron).
val input = "<recipient address>"
val parsed = sdk.parse(input)
if (parsed !is InputType.CrossChainAddress) {
    throw IllegalArgumentException("Not a cross-chain address")
}
val addressDetails = parsed.v1

// List the stablecoin destinations Cash App can fund over Lightning and
// pick one, e.g. USDC on Base.
val routes = sdk.getCrossChainRoutes(
    CrossChainRouteFilter.Send(
        addressDetails = addressDetails,
        deliveryMethod = DeliveryMethod.LIGHTNING,
    )
)
val route = routes.find { it.asset == "USDC" && it.chain == "base" }
    ?: throw IllegalArgumentException("No USDC route on Base")

// Send $10 of USDC, funded by Cash App over Lightning. The amount is in
// the route asset's base units (USDC, 6 decimals), so 10_000_000 =
// 10 USDC, about $10.
val response = sdk.bridgeFromCashApp(
    BridgeFromCashAppRequest(
        address = addressDetails.address,
        route = route,
        amount = BigInteger.fromLong(10_000_000L),
        feePolicy = null,
        maxSlippageBps = null,
    )
)

// Open this Cash App URL to pay. The recipient then receives the stablecoin.
// Log.v("Breez", "Open this URL in Cash App: ${response.url}")
// Log.v("Breez", "Recipient receives ~${response.estimatedOut} ${response.asset}")
```



## Response fields

`BridgeFromCashAppResponse` carries the payment URL and the quote:

| Field | Meaning |
| ----- | ------- |
| `url` | The Cash App deep link to open. |
| `amountSats` | Bitcoin amount the user deposits through Cash App. |
| `estimatedOut` | Expected amount delivered to the recipient, in `asset` units. |
| `asset` | The delivered stablecoin (e.g. `USDC`). |
| `serviceFeeAmount` | Provider fee for the conversion, including your [partner fee](./cross_chain_partner_fees.md) if you set one. |
| `serviceFeeAsset` | Denomination of the fee. Absent means the fee is in sats. |
| `serviceFeeAssetDecimals` | Decimals of `serviceFeeAsset`, for formatting the fee. Absent when the fee is in sats or the provider did not report them. |
| `expiresAt` | Quote expiry, in seconds since the Unix epoch. Call `bridgeFromCashApp` again for a fresh quote if it lapses before the user starts paying. |

## Opening the payment link

On devices with Cash App installed the URL opens the app directly. Otherwise it falls back to the Cash App website. The same UX guidance as the [Buying Bitcoin](./buy_bitcoin.md#recommended-ux) Cash App flow applies (mobile redirect, desktop QR code, pre-opening a tab on web to avoid popup blockers).
