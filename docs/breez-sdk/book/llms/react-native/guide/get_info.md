# Getting the SDK info

API docs: https://breez.github.io/spark-sdk/breez_sdk_spark/struct.BreezSdk.html#method.get_info

Once connected, you can retrieve the current state of the SDK at any time using `getInfo`. This returns:

- **Spark identity public key** - The wallet's unique identity on the Spark network as a hex string
- **Bitcoin balance** - The balance in satoshis
- **Token balances** - Balances of any tokens held in the wallet

```typescript
// ensureSynced: true will ensure the SDK is synced with the Spark network
// before returning the balance
const info = await sdk.getInfo({
  ensureSynced: false
})
const identityPubkey = info.identityPubkey
const balanceSats = info.balanceSats
```



## Fetching the balance

The SDK keeps a **cached balance** in local storage and `getInfo` reads from this cache for a low-latency response. The cache is refreshed automatically by the SDK's background sync.

The recommended pattern is:

1. Call `getInfo` with `ensureSynced` = **false** whenever you need to render the balance.
2. Subscribe to events and call `getInfo` again on each `SdkEvent.Synced` event to fetch the latest balance. See [Listening to events](/llms/react-native/guide/events.md).

| Event | Description | UX Suggestion |
| ----- | ----------- | ------------- |
| `SdkEvent.Synced` | The SDK has synced with the network in the background. | Call `getInfo` to refresh the displayed balance, and refresh the payments list. See [listing payments](/llms/react-native/guide/list_payments.md). |

**Developer note**

`ensureSynced` = **true** blocks until the SDK's **initial** sync after `connect` completes. This is useful for short-lived scripts that connect, read the balance once, and disconnect. It is **not** a "force a fresh sync now" call. In long-running applications, prefer `ensureSynced` = **false** combined with the `SdkEvent.Synced` event listener pattern above.

## Server mode

When the SDK is built with [Server mode](server_mode.md), `getInfo` reads the balance live from the spark wallet's local tree store rather than from the background-maintained cache. As a result:

- `ensureSynced` = **true** is rejected with an invalid-input error. The SDK has no initial-sync watcher to await; call `syncWallet` explicitly if you need to refresh state first.
- The returned balance reflects whatever is currently in the local tree store. If you need the freshest possible balance after an external state change (an incoming Spark transfer claimed elsewhere, an on-chain deposit confirmed, etc.), call `syncWallet` first.
