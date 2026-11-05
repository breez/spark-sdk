# Cash App to USDC/USDT

`bridge_from_cash_app` lets a user send USDC or USDT to a recipient on an external chain. The user pays through Cash App over Lightning, and the cross-chain provider delivers the stablecoin to the recipient. See [Cash App and USDC/USDT](./cash_app.md) for what the two Cash App bridges share.

## How it works

1. [Select a destination route](./cash_app.md#selecting-a-route) for the recipient's external-chain address.
2. Call `bridge_from_cash_app` with the recipient, the route and the USD amount. The SDK gets a quote from the cross-chain provider and returns a Cash App deep link plus the quote details.
3. Open the returned `payment_request`. The user completes payment in Cash App, and the provider delivers the stablecoin to the recipient.

The SDK builds a `cash.app/launch/lightning/<bolt11>` deep link from the provider's invoice and returns it as the response `payment_request`. The invoice itself is `info.deposit_address`.

```rust
// List the routes to the recipient's external-chain address
// (EVM/Solana/Tron) and pick a stablecoin Cash App can fund over
// Lightning, e.g. USDC on Base.
let recipient = "<recipient address>".to_string();
let routes = client
    .get_routes(GetBridgeRoutesRequest {
        recipient: recipient.clone(),
    })
    .await?
    .routes;
let route = routes
    .into_iter()
    .find(|r| {
        r.source.chain == "lightning"
            && r.destination.asset == "USDC"
            && r.destination.chain == "base"
    })
    .ok_or_else(|| anyhow::anyhow!("No USDC route on Base"))?;

// Send $10 of USDC, funded by Cash App over Lightning. The amount is in
// USDC base units (6 decimals), so 10_000_000 = 10 USDC, about $10.
let response = client
    .bridge_from_cash_app(CreateBridgeRequest {
        recipient,
        route,
        amount: 10_000_000,
        refund_address: None,
        fee_policy: None,
        max_slippage_bps: None,
    })
    .await?;

// Open this Cash App link to pay. The recipient then receives the stablecoin.
let info = response.info;
info!("Open this URL in Cash App: {}", response.payment_request);
info!(
    "Recipient receives ~{} {}",
    info.expected_received_amount, info.destination_asset
);
```



## Response fields

`bridge_from_cash_app` returns a `CreateBridgeResponse`, described in [Bridging USDC/USDT](./bridging.md#response-fields). Its `payment_request` is the Cash App deep link to open.

## Opening the payment link

On devices with Cash App installed the URL opens the app directly. Otherwise it falls back to the Cash App website. The same UX guidance as the [Buying Bitcoin](./buy_bitcoin.md#recommended-ux) Cash App flow applies (mobile redirect, desktop QR code, pre-opening a tab on web to avoid popup blockers).
