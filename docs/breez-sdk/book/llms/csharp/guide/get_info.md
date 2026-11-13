# Getting the SDK info

Once connected, you can retrieve the current state of the SDK at any time using `GetInfo`. This returns:

- **Spark identity public key** - The wallet's unique identity on the Spark network as a hex string
- **Bitcoin balance** - The balance in satoshis
- **Token balances** - Balances of any tokens held in the wallet
- **Recoverable funds** - Funds that left the balance and wait to be recovered on-chain, see [Recovering funds](recover_funds.md)
- **Last sync error** - Why the latest sync couldn't refresh the balances, if it failed
- **Last synced at** - When a sync last refreshed the balances

```csharp
// ensureSynced: true will ensure the SDK is synced with the Spark network
// before returning the balance
var info = await sdk.GetInfo(request: new GetInfoRequest(ensureSynced: false));
var identityPubkey = info.identityPubkey;
var balanceSats = info.balanceSats;
```



## Fetching the balance

The SDK keeps a **cached balance** in local storage and `GetInfo` reads from this cache for a low-latency response. The cache is refreshed automatically by the SDK's background sync.

The recommended pattern is:

1. Call `GetInfo` with `EnsureSynced` = **false** whenever you need to render the balance.
2. Subscribe to events and call `GetInfo` again on each `SdkEvent.Synced` event to fetch the latest balance. See [Listening to events](/llms/csharp/guide/events.md).

| Event | Description | UX Suggestion |
| ----- | ----------- | ------------- |
| `SdkEvent.Synced` | The SDK has synced with the network in the background. | Call `GetInfo` to refresh the displayed balance, and refresh the payments list. See [listing payments](/llms/csharp/guide/list_payments.md). |

When the latest sync fails, the cached balance may be out of date and `LastSyncError` is set. `SyncError.Network` means the network couldn't be reached, so you can ask the user to check their connection. `SyncError.Generic` covers any other failure. Both carry a message meant for logs, not for display. While either is set, show the balance as unconfirmed. `LastSyncedAt` tells you how old it is, for example to show when it was last updated.

**Developer note**

`EnsureSynced` = **true** blocks until the SDK's **initial** sync after `Connect` completes. This is useful for short-lived scripts that connect, read the balance once, and disconnect. If that sync fails, the call still returns, with `LastSyncError` set. It is **not** a "force a fresh sync now" call. In long-running applications, prefer `EnsureSynced` = **false** combined with the `SdkEvent.Synced` event listener pattern above.

## Without background tasks

When the SDK is built with the [multi-user configuration](server_mode.md), `GetInfo` reads the balance live from the spark wallet's local tree store rather than from the background-maintained cache. As a result:

- `EnsureSynced` = **true** is rejected with an invalid-input error. The SDK has no initial-sync watcher to await; call `SyncWallet` explicitly if you need to refresh state first.
- The returned balance reflects whatever is currently in the local tree store. If you need the freshest possible balance after an external state change (an incoming Spark transfer claimed elsewhere, an on-chain deposit confirmed, etc.), call `SyncWallet` first.
- `SyncWallet` returns `LastSyncError` for its own sync, and `LastSyncedAt` for the last sync that succeeded.
