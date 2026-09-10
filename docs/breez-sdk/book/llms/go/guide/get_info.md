# Getting the SDK info

API docs: https://breez.github.io/spark-sdk/breez_sdk_spark/struct.BreezSdk.html#method.get_info

Once connected, you can retrieve the current state of the SDK at any time using `GetInfo`. This returns:

- **Spark identity public key** - The wallet's unique identity on the Spark network as a hex string
- **Bitcoin balance** - The balance in satoshis
- **Token balances** - Balances of any tokens held in the wallet

```go
ensureSynced := false
info, err := sdk.GetInfo(breez_sdk_spark.GetInfoRequest{
	// EnsureSynced: true will ensure the SDK is synced with the Spark network
	// before returning the balance
	EnsureSynced: &ensureSynced,
})

if err != nil {
	var sdkErr *breez_sdk_spark.SdkError
	if errors.As(err, &sdkErr) {
		// Handle SdkError - can inspect specific variants if needed
		// e.g., switch on sdkErr variant for InsufficientFunds, NetworkError, etc.
	}
	return err
}

identityPubkey := info.IdentityPubkey
balanceSats := info.BalanceSats
log.Printf("Identity pubkey: %v, Balance: %v sats", identityPubkey, balanceSats)
```



## Fetching the balance

The SDK keeps a **cached balance** in local storage and `GetInfo` reads from this cache for a low-latency response. The cache is refreshed automatically by the SDK's background sync.

The recommended pattern is:

1. Call `GetInfo` with `EnsureSynced` = **false** whenever you need to render the balance.
2. Subscribe to events and call `GetInfo` again on each `SdkEventSynced` event to fetch the latest balance. See [Listening to events](/llms/go/guide/events.md).

| Event | Description | UX Suggestion |
| ----- | ----------- | ------------- |
| `SdkEventSynced` | The SDK has synced with the network in the background. | Call `GetInfo` to refresh the displayed balance, and refresh the payments list. See [listing payments](/llms/go/guide/list_payments.md). |

**Developer note**

`EnsureSynced` = **true** blocks until the SDK's **initial** sync after `Connect` completes. This is useful for short-lived scripts that connect, read the balance once, and disconnect. It is **not** a "force a fresh sync now" call. In long-running applications, prefer `EnsureSynced` = **false** combined with the `SdkEventSynced` event listener pattern above.

## Server mode

When the SDK is built with [Server mode](server_mode.md), `GetInfo` reads the balance live from the spark wallet's local tree store rather than from the background-maintained cache. As a result:

- `EnsureSynced` = **true** is rejected with an invalid-input error. The SDK has no initial-sync watcher to await; call `SyncWallet` explicitly if you need to refresh state first.
- The returned balance reflects whatever is currently in the local tree store. If you need the freshest possible balance after an external state change (an incoming Spark transfer claimed elsewhere, an on-chain deposit confirmed, etc.), call `SyncWallet` first.
