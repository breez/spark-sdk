# Cash App to USDC/USDT

`bridge_from_cash_app` lets a user send USDC or USDT to a recipient on an external chain. The user pays through Cash App over Lightning, and the cross-chain provider delivers the stablecoin to the recipient. See [Cash App and USDC/USDT](./cash_app.md) for what the two Cash App bridges share.

## How it works

1. Parse the recipient's external-chain address and [select a destination route](./cash_app.md#selecting-a-route) with `CrossChainRouteFilter.SEND`.
2. Call `bridge_from_cash_app` with the route and the USD amount. The SDK gets a quote from the cross-chain provider and returns a Cash App deep link plus the quote details.
3. Open the returned `url`. The user completes payment in Cash App, and the provider delivers the stablecoin to the recipient.

The SDK builds a `cash.app/launch/lightning/<bolt11>` deep link from the provider's invoice and returns it as the response `url`.

```python
# Parse the recipient's external-chain address (EVM/Solana/Tron).
input_str = "<recipient address>"
try:
    parsed = await sdk.parse(input=input_str)
    if not isinstance(parsed, InputType.CROSS_CHAIN_ADDRESS):
        raise ValueError("Not a cross-chain address")
    address_details = parsed[0]

    # List the stablecoin destinations Cash App can fund over Lightning and
    # pick one, e.g. USDC on Base.
    routes = await sdk.get_cross_chain_routes(
        filter=CrossChainRouteFilter.SEND(
            address_details=address_details,
            delivery_method=DeliveryMethod.LIGHTNING,
        )
    )
    route = next(
        (r for r in routes if r.asset == "USDC" and r.chain == "base"),
        None,
    )
    if route is None:
        raise ValueError("No USDC route on Base")

    # Send $10 of USDC, funded by Cash App over Lightning. The amount is in
    # the route asset's base units (USDC, 6 decimals), so 10_000_000 =
    # 10 USDC, about $10.
    request = BridgeFromCashAppRequest(
        address=address_details.address,
        route=route,
        amount=10_000_000,
        fee_policy=None,
        max_slippage_bps=None,
    )
    response = await sdk.bridge_from_cash_app(request=request)

    # Open this Cash App URL to pay. The recipient then receives the
    # stablecoin.
    logging.debug(f"Open this URL in Cash App: {response.url}")
    logging.debug(
        f"Recipient receives ~{response.estimated_out} {response.asset}"
    )
except Exception as error:
    logging.error(error)
    raise
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
