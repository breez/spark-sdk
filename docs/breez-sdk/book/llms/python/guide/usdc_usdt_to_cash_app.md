# USDC/USDT to Cash App

`bridge_to_cash_app` lets someone holding USDC or USDT on an external chain pay a Cash App user. The payer deposits the stablecoin, and the cross-chain provider pays the recipient in Bitcoin over Lightning. See [Cash App and USDC/USDT](./cash_app.md) for what the two Cash App bridges share.

## How it works

1. Get a [bridging client](./bridging.md#getting-a-bridging-client): build one from your API key with `new_bridging_client`, or call `get_bridging_client` on a connected SDK.
2. [Select a source route](./cash_app.md#selecting-a-route) for the Cash App user.
3. Call `bridge_to_cash_app` with the recipient, the route, the USD amount and the payer's refund address. The SDK checks the recipient and gets a quote from the cross-chain provider.
4. Show the payer the returned `payment_request`, and store the `bridge_id` and `status_token` to [track the bridge](#tracking-the-bridge). Once their deposit lands, the provider pays the recipient.

```python
try:
    # List the routes to the Cash App user and pick what the payer pays in,
    # e.g. USDC on Base.
    recipient = "$alice"
    routes_response = await client.get_routes(
        request=GetBridgeRoutesRequest(recipient=recipient)
    )
    route = next(
        (
            r
            for r in routes_response.routes
            if r.source.asset == "USDC" and r.source.chain == "base"
        ),
        None,
    )
    if route is None:
        raise ValueError("No route from USDC on Base")

    # Pay $10 of USDC to the Cash App user $alice. The amount is in USDC base
    # units (6 decimals), so 10_000_000 = 10 USDC, about $10. The deposit is
    # refunded to the payer's address if delivery fails.
    request = CreateBridgeRequest(
        recipient=recipient,
        route=route,
        amount=10_000_000,
        refund_address="<payer address>",
        fee_policy=None,
        max_slippage_bps=None,
    )
    response = await client.bridge_to_cash_app(request=request)

    # Show the payer what to pay. The recipient then receives Bitcoin.
    info = response.info
    logging.debug(f"Pay with: {response.payment_request}")
    logging.debug(
        f"Deposit {info.deposit_amount} to {info.deposit_address}, "
        f"recipient receives ~{info.expected_received_amount} sats"
    )
except Exception as error:
    logging.error(error)
    raise
```



## Recipient

The `recipient` is a Cash App username, given as `$alice`, or the user's Cash App Lightning address, `alice@cash.app`. Both reach the same user, who is paid at that Lightning address. `bridge_to_cash_app` also takes the bare `alice`, but `get_routes` needs the `$`.

`get_routes` looks the user up, so an unknown username fails before anything is quoted. `bridge_to_cash_app` fails with `SdkError.INVALID_INPUT` for a recipient who isn't a Cash App user, or an amount outside what the recipient accepts.

## Refund address

The `refund_address` is the payer's own address on the source chain. If the recipient can't be paid, the provider refunds the deposit there. It must belong to the `source` chain's family (e.g. an EVM `0x...` address for USDC on Base).

## Response fields

`bridge_to_cash_app` returns a `CreateBridgeResponse`, described in [Bridging USDC/USDT](./bridging.md#response-fields). Its `payment_request` is what the payer pays: an EIP-681 URI on EVM chains, otherwise the bare deposit address. A Solana transfer built in code must also create the deposit address's token account, which wallet apps do on their own (see [Paying from Solana](./cross_chain.md#paying-from-solana)).

## Tracking the bridge

Pass the `bridge_id` and `status_token` from the response to `get_bridge_status`. The bridge reads:

- `BridgeStatus.CREATED` until the provider sees the deposit. It stays `BridgeStatus.CREATED` after its `expires_at`, because a late deposit may still be accepted at a new price. Stop polling when you no longer expect one.
- `BridgeStatus.PENDING` from the deposit to the Lightning payment. `deposit_tx_hash` is the payer's transaction.
- `BridgeStatus.COMPLETED` once the Lightning payment reached the recipient. `amount_out` is in sats. `delivery_tx_hash` is the provider's reference for the payment, not a Lightning payment hash.
- `BridgeStatus.REFUNDING`, then `BridgeStatus.REFUNDED`, if the recipient couldn't be paid (see [Limitations](#limitations)) and the deposit is returned to the `refund_address`.

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

## Limitations

- **Delivery can still fail after funding.** The provider looks the recipient up again when it pays. If the account was closed, its limit lowered, or slippage moved the amount across a limit since the quote, the payment fails and the deposit is refunded to `refund_address`.
- **Lightning delivery only.** A completed bridge means the Lightning payment was delivered. Whether Cash App credits it as Bitcoin or converts it to dollars depends on the recipient's Cash App settings.
