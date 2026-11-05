# Cash App to USDC/USDT

`bridge_from_cash_app` lets a user send USDC or USDT to a recipient on an external chain. The user pays through Cash App over Lightning, and the cross-chain provider delivers the stablecoin to the recipient. See [Cash App and USDC/USDT](./cash_app.md) for what the two Cash App bridges share.

## How it works

1. [Select a destination route](./cash_app.md#selecting-a-route) for the recipient's external-chain address.
2. Call `bridge_from_cash_app` with the recipient, the route and the USD amount. The SDK gets a quote from the cross-chain provider and returns a Cash App deep link plus the quote details.
3. Open the returned `payment_request`. The user completes payment in Cash App, and the provider delivers the stablecoin to the recipient.

The SDK builds a `cash.app/launch/lightning/<bolt11>` deep link from the provider's invoice and returns it as the response `payment_request`. The invoice itself is `info.deposit_address`.

```python
try:
    # List the routes to the recipient's external-chain address
    # (EVM/Solana/Tron) and pick a stablecoin Cash App can fund over
    # Lightning, e.g. USDC on Base.
    recipient = "<recipient address>"
    routes_response = await client.get_routes(
        request=GetBridgeRoutesRequest(recipient=recipient)
    )
    route = next(
        (
            r
            for r in routes_response.routes
            if r.source.chain == "lightning"
            and r.destination.asset == "USDC"
            and r.destination.chain == "base"
        ),
        None,
    )
    if route is None:
        raise ValueError("No USDC route on Base")

    # Send $10 of USDC, funded by Cash App over Lightning. The amount is in
    # USDC base units (6 decimals), so 10_000_000 = 10 USDC, about $10.
    request = CreateBridgeRequest(
        recipient=recipient,
        route=route,
        amount=10_000_000,
        refund_address=None,
        fee_policy=None,
        max_slippage_bps=None,
    )
    response = await client.bridge_from_cash_app(request=request)

    # Open this Cash App link to pay. The recipient then receives the
    # stablecoin.
    info = response.info
    logging.debug(f"Open this URL in Cash App: {response.payment_request}")
    logging.debug(
        f"Recipient receives ~{info.expected_received_amount} "
        f"{info.destination_asset}"
    )
except Exception as error:
    logging.error(error)
    raise
```



## Response fields

`bridge_from_cash_app` returns a `CreateBridgeResponse`, described in [Bridging USDC/USDT](./bridging.md#response-fields). Its `payment_request` is the Cash App deep link to open.

## Opening the payment link

On devices with Cash App installed the URL opens the app directly. Otherwise it falls back to the Cash App website. The same UX guidance as the [Buying Bitcoin](./buy_bitcoin.md#recommended-ux) Cash App flow applies (mobile redirect, desktop QR code, pre-opening a tab on web to avoid popup blockers).
