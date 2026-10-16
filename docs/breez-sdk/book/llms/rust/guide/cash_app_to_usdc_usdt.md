# Cash App to USDC/USDT

`bridge_from_cash_app` lets a user send USDC or USDT to a recipient on an external chain. The user pays through Cash App over Lightning, and the cross-chain provider delivers the stablecoin to the recipient. See [Cash App and USDC/USDT](./cash_app.md) for what the two Cash App bridges share.

## How it works

1. Parse the recipient's external-chain address and [select a destination route](./cash_app.md#selecting-a-route) with `CrossChainRouteFilter::Send`.
2. Call `bridge_from_cash_app` with the route and the USD amount. The SDK gets a quote from the cross-chain provider and returns a Cash App deep link plus the quote details.
3. Open the returned `url`. The user completes payment in Cash App, and the provider delivers the stablecoin to the recipient.

The SDK builds a `cash.app/launch/lightning/<bolt11>` deep link from the provider's invoice and returns it as the response `url`.

```rust
// Parse the recipient's external-chain address (EVM/Solana/Tron).
let input = "<recipient address>";
let InputType::CrossChainAddress(address_details) = sdk.parse(input).await? else {
    anyhow::bail!("Not a cross-chain address");
};

// List the stablecoin destinations Cash App can fund over Lightning and
// pick one, e.g. USDC on Base.
let routes = sdk
    .get_cross_chain_routes(&CrossChainRouteFilter::Send {
        address_details: address_details.clone(),
        delivery_method: Some(DeliveryMethod::Lightning),
    })
    .await?;
let route = routes
    .into_iter()
    .find(|r| r.asset == "USDC" && r.chain == "base")
    .ok_or_else(|| anyhow::anyhow!("No USDC route on Base"))?;

// Send $10 of USDC, funded by Cash App over Lightning. The amount is in the
// route asset's base units (USDC, 6 decimals), so 10_000_000 = 10 USDC,
// about $10.
let response = sdk
    .bridge_from_cash_app(BridgeFromCashAppRequest {
        address: address_details.address,
        route,
        amount: 10_000_000,
        fee_policy: None,
        max_slippage_bps: None,
    })
    .await?;

// Open this Cash App URL to pay. The recipient then receives the stablecoin.
info!("Open this URL in Cash App: {}", response.url);
info!(
    "Recipient receives ~{} {}",
    response.estimated_out, response.asset
);
```



## Response fields

`BridgeFromCashAppResponse` carries the payment URL and the quote:

| Field | Meaning |
| ----- | ------- |
| `url` | The Cash App deep link to open. |
| `amount_sats` | Bitcoin amount the user deposits through Cash App. |
| `estimated_out` | Expected amount delivered to the recipient, in `asset` units. |
| `asset` | The delivered stablecoin (e.g. `USDC`). |
| `service_fee_amount` | Provider fee for the conversion, including your [partner fee](./cross_chain_partner_fees.md) if you set one. |
| `service_fee_asset` | Denomination of the fee. Absent means the fee is in sats. |
| `service_fee_asset_decimals` | Decimals of `service_fee_asset`, for formatting the fee. Absent when the fee is in sats or the provider did not report them. |
| `expires_at` | Quote expiry, in seconds since the Unix epoch. Call `bridge_from_cash_app` again for a fresh quote if it lapses before the user starts paying. |

## Opening the payment link

On devices with Cash App installed the URL opens the app directly. Otherwise it falls back to the Cash App website. The same UX guidance as the [Buying Bitcoin](./buy_bitcoin.md#recommended-ux) Cash App flow applies (mobile redirect, desktop QR code, pre-opening a tab on web to avoid popup blockers).
