# Cash App to USDC/USDT

`BridgeFromCashApp` lets a user send USDC or USDT to a recipient on an external chain. The user pays through Cash App over Lightning, and the cross-chain provider delivers the stablecoin to the recipient. See [Cash App and USDC/USDT](./cash_app.md) for what the two Cash App bridges share.

## How it works

1. Parse the recipient's external-chain address and [select a destination route](./cash_app.md#selecting-a-route) with `CrossChainRouteFilter.Send`.
2. Call `BridgeFromCashApp` with the route and the USD amount. The SDK gets a quote from the cross-chain provider and returns a Cash App deep link plus the quote details.
3. Open the returned `Url`. The user completes payment in Cash App, and the provider delivers the stablecoin to the recipient.

The SDK builds a `cash.app/launch/lightning/<bolt11>` deep link from the provider's invoice and returns it as the response `Url`.

```csharp
// Parse the recipient's external-chain address (EVM/Solana/Tron).
var parsed = await sdk.Parse(input: "<recipient address>");
if (parsed is not InputType.CrossChainAddress crossChain)
{
    throw new InvalidOperationException("Not a cross-chain address");
}
var addressDetails = crossChain.v1;

// List the stablecoin destinations Cash App can fund over Lightning
// and pick one, e.g. USDC on Base.
var filter = new CrossChainRouteFilter.Send(
    addressDetails: addressDetails,
    deliveryMethod: DeliveryMethod.Lightning
);
var routes = await sdk.GetCrossChainRoutes(filter: filter);
var route = routes.First(r => r.asset == "USDC" && r.chain == "base");

// Send $10 of USDC, funded by Cash App over Lightning. The amount
// is in the route asset's base units (USDC, 6 decimals), so
// 10_000_000 = 10 USDC, about $10.
var request = new BridgeFromCashAppRequest(
    address: addressDetails.address,
    route: route,
    amount: new BigInteger(10_000_000),
    feePolicy: null,
    maxSlippageBps: null
);
var response = await sdk.BridgeFromCashApp(request: request);

// Open this Cash App URL to pay. The recipient then receives the
// stablecoin.
Console.WriteLine($"Open this URL in Cash App: {response.url}");
Console.WriteLine($"Recipient receives ~{response.estimatedOut} {response.asset}");
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
