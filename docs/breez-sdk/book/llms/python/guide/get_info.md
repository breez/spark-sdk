# Getting the SDK info

API docs: https://breez.github.io/spark-sdk/breez_sdk_spark/struct.BreezSdk.html#method.get_info

Once connected, you can retrieve the current state of the SDK at any time using `get_info`. This returns:

- **Spark identity public key** - The wallet's unique identity on the Spark network as a hex string
- **Bitcoin balance** - The balance in satoshis
- **Token balances** - Balances of any tokens held in the wallet

```python
try:
    # ensure_synced: True will ensure the SDK is synced with the Spark network
    # before returning the balance
    info = await sdk.get_info(request=GetInfoRequest(ensure_synced=False))
    identity_pubkey = info.identity_pubkey
    balance_sats = info.balance_sats
except Exception as error:
    logging.error(error)
    raise
```



## Fetching the balance

The SDK keeps a **cached balance** in local storage and `get_info` reads from this cache for a low-latency response. The cache is refreshed automatically by the SDK's background sync.

The recommended pattern is:

1. Call `get_info` with `ensure_synced` = **false** whenever you need to render the balance.
2. Subscribe to events and call `get_info` again on each `SdkEvent.SYNCED` event to fetch the latest balance. See [Listening to events](/llms/python/guide/events.md).

| Event | Description | UX Suggestion |
| ----- | ----------- | ------------- |
| `SdkEvent.SYNCED` | The SDK has synced with the network in the background. | Call `get_info` to refresh the displayed balance, and refresh the payments list. See [listing payments](/llms/python/guide/list_payments.md). |

**Developer note**

`ensure_synced` = **true** blocks until the SDK's **initial** sync after `connect` completes. This is useful for short-lived scripts that connect, read the balance once, and disconnect. It is **not** a "force a fresh sync now" call. In long-running applications, prefer `ensure_synced` = **false** combined with the `SdkEvent.SYNCED` event listener pattern above.

## Server mode

When the SDK is built with [Server mode](server_mode.md), `get_info` reads the balance live from the spark wallet's local tree store rather than from the background-maintained cache. As a result:

- `ensure_synced` = **true** is rejected with an invalid-input error. The SDK has no initial-sync watcher to await; call `sync_wallet` explicitly if you need to refresh state first.
- The returned balance reflects whatever is currently in the local tree store. If you need the freshest possible balance after an external state change (an incoming Spark transfer claimed elsewhere, an on-chain deposit confirmed, etc.), call `sync_wallet` first.
