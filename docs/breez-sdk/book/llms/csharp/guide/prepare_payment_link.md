# Prepare Payment Link

`PreparePaymentLink` lets a user send USDC or USDT to a recipient on an external chain (Ethereum-family, Solana, or Tron). The user pays through Cash App (over Lightning) and a cross-chain provider delivers the stablecoin to the recipient.

The SDK acts as orchestrator only. No wallet funds move, and nothing is written to the payment history: Cash App funds the purchase externally and the provider delivers autonomously. This is distinct from the wallet-funded, tracked flow on the [Send USDC/USDT](./cross_chain.md) page.

## How it works

1. Parse the recipient's external-chain address and select a destination route.
2. Call `PreparePaymentLink` with the route and the USD amount. The SDK gets a quote from the cross-chain provider and returns a Cash App deep link plus the quote details.
3. Open the returned `Url`. The user completes payment in Cash App, and the provider delivers the stablecoin to the recipient.

Because the SDK never sees the funds, `PreparePaymentLink` returns and forgets: there is no `Payment` row, no status tracking, and no event. Surface the quote fields in the response so the app can show the user what to expect before they pay.

## Selecting a route

Parse the recipient address, then list the stablecoin destinations you can send to with `GetCrossChainRoutes` filtered by `CrossChainRouteFilter.PaymentLink`. These routes are the ones fundable by an external rail (Cash App over Lightning), a different set from the wallet-funded send routes. Pick the `CrossChainRoutePair` whose `CrossChainRoutePair.Asset` and `CrossChainRoutePair.Chain` match the destination you want, and pass it to `PreparePaymentLink`.

Calling `PreparePaymentLink` with a route that can't be funded over Lightning fails fast, before any funds move.

## Cash App

Cash App funds the purchase over **Lightning**. The SDK builds a `cash.app/launch/lightning/<bolt11>` deep link from the provider's invoice and returns it as the response `Url`.

```csharp
// Parse the recipient's external-chain address (EVM/Solana/Tron).
var parsed = await sdk.Parse(input: "<recipient address>");
if (parsed is not InputType.CrossChainAddress crossChain)
{
    throw new InvalidOperationException("Not a cross-chain address");
}
var addressDetails = crossChain.v1;

// List the stablecoin destinations you can send to and pick one,
// e.g. USDC on Base. Restrict to Lightning-fundable routes,
// since Cash App funds over Lightning.
var filter = new CrossChainRouteFilter.PaymentLink(
    addressDetails: addressDetails
);
var routes = await sdk.GetCrossChainRoutes(filter: filter);
var route = routes.First(r => r.asset == "USDC" && r.chain == "base");

// Send $10 of USDC, funded by Cash App over Lightning. The amount
// is in the route asset's base units (USDC, 6 decimals), so
// 10_000_000 = 10 USDC, about $10.
var request = new PreparePaymentLinkRequest(
    address: addressDetails.address,
    route: route,
    amount: new BigInteger(10_000_000),
    feePolicy: null,
    maxSlippageBps: null
);
var response = await sdk.PreparePaymentLink(request: request);
Console.WriteLine($"Open this URL in Cash App: {response.url}");
Console.WriteLine($"Recipient receives ~{response.estimatedOut} {response.asset}");
```



On devices with Cash App installed the URL opens the app directly; otherwise it falls back to the Cash App website. The same UX guidance as the [Buying Bitcoin](./buy_bitcoin.md#recommended-ux) Cash App flow applies (mobile redirect, desktop QR code, pre-opening a tab on web to avoid popup blockers).

**Developer note**

Cash App is available in the US and UK only (excluding New York State for Bitcoin/Lightning features). Cash App handles region restrictions on their end, so no client-side gating is needed.

## Amounts and fees

The `Amount` on `PreparePaymentLinkRequest` is in the destination asset's base units, per the route's `CrossChainRoutePair.Decimals`. These routes deliver USD-pegged stablecoins, so at parity it is the USD value: `1_000_000` is 1 USDC (6 decimals), about $1.

`FeePolicy` controls who absorbs the provider fee, reusing the same `FeePolicy` as the send flow:

- `FeePolicy.FeesExcluded` (default) delivers the target amount to the recipient and adds the fee on top of what the user pays.
- `FeePolicy.FeesIncluded` takes the fee out of the amount, so the user pays exactly the amount and the recipient receives less.

`MaxSlippageBps` bounds the price movement tolerated between quote and delivery, in basis points (1 bps = 0.01%). Left unset, the provider default applies.

## Response fields

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

## Limitations

- **Mainnet only.** Cash App and the cross-chain providers operate against live networks. There is no testnet equivalent.
- **Not tracked.** The purchase is funded outside the wallet, so it produces no `Payment` row and no event. If you need delivery confirmation, observe the recipient chain directly.
- **Orchestra routes only.** Payment links quote against Orchestra, whose orders deliver without the SDK online. A Boltz route would need the wallet to stay online to claim the swap before the payer's payment settles, so `GetCrossChainRoutes` with `CrossChainRouteFilter.PaymentLink` returns only Orchestra routes and `PreparePaymentLink` rejects a Boltz route.
