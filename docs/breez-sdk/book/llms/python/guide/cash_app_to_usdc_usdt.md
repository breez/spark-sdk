# Cash App to USDC/USDT

`bridge_from_cash_app` lets a user send USDC or USDT to a recipient on an external chain. The user pays through Cash App over Lightning, and the cross-chain provider delivers the stablecoin to the recipient. See [Cash App and USDC/USDT](./cash_app.md) for what the two Cash App bridges share.

## How it works

1. Get a [bridging client](./bridging.md#getting-a-bridging-client): build one from your API key with `new_bridging_client`, or call `get_bridging_client` on a connected SDK.
2. [Select a destination route](./cash_app.md#selecting-a-route) for the recipient's external-chain address.
3. Call `bridge_from_cash_app` with the recipient, the route and the USD amount. The SDK creates an order with the cross-chain provider and returns a Cash App deep link plus the order details.
4. Open the returned `payment_request`, and store the `bridge_id` and `status_token` to [track the bridge](#tracking-the-bridge). The user completes payment in Cash App, and the provider delivers the stablecoin to the recipient.

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



## Slippage and refunds

The provider allows these payments up to 10% of price movement between the quote and the delivery, whatever `max_slippage_bps` says. With fees excluded, `max_slippage_bps` still bounds how far the quoted delivery may fall short of the `amount`: a bridge quoted below it fails before the user sees a link. With fees included, the quoted delivery isn't checked.

If the payment fails before the swap, the provider refunds it to the [refund address](./bridging.md#refund-address): a Lightning address, Cash App `$cashtag` or amountless BOLT11 invoice. An invoice must not expire before the bridge, and must still be valid when the refund is paid. Without a refund address, the payment can't be refunded automatically.

## Response fields

`bridge_from_cash_app` returns a `CreateBridgeResponse`, described in [Bridging USDC/USDT](./bridging.md#response-fields). Its `payment_request` is the Cash App deep link to open.

## Opening the payment link

On devices with Cash App installed the URL opens the app directly. Otherwise it falls back to the Cash App website. The same UX guidance as the [Buying Bitcoin](./buy_bitcoin.md#recommended-ux) Cash App flow applies (mobile redirect, desktop QR code, pre-opening a tab on web to avoid popup blockers).

## Tracking the bridge

Pass the `bridge_id` and `status_token` from the response to `get_bridge_status`. The bridge reads:

- `BridgeStatus.CREATED` until the user pays in Cash App.
- `BridgeStatus.EXPIRED` if nobody pays before its `expires_at`. The link can't be paid after that, so create a new bridge.
- `BridgeStatus.PENDING` once the payment arrives, while the provider swaps and delivers.
- `BridgeStatus.COMPLETED` once the recipient has the stablecoin. `delivery_tx_hash` is the delivery transaction on the recipient's chain.
- `BridgeStatus.REFUNDING`, then `BridgeStatus.REFUNDED`, if the payment failed before the swap and is returned to the [refund address](#slippage-and-refunds).

```python
try:
    response = await client.get_bridge_status(
        request=GetBridgeStatusRequest(
            bridge_id=bridge_id, status_token=status_token
        )
    )
except Exception as error:
    logging.error(error)
    raise
status = response.status
if status == BridgeStatus.COMPLETED:
    logging.debug(f"Delivered {response.amount_out}")
elif status in (BridgeStatus.REFUNDED, BridgeStatus.FAILED, BridgeStatus.EXPIRED):
    logging.debug(f"Not delivered: {status}")
else:
    logging.debug(f"Not final yet: {status}")
```



See [Tracking a bridge](./bridging.md#tracking-a-bridge) for every status.
