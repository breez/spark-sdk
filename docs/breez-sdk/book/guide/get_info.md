# Getting the SDK info

API docs: https://breez.github.io/spark-sdk/breez_sdk_spark/struct.BreezSdk.html#method.get_info

Once connected, you can retrieve the current state of the SDK at any time using `get_info`. This returns:

- **Spark identity public key** - The wallet's unique identity on the Spark network as a hex string
- **Bitcoin balance** - The balance in satoshis
- **Token balances** - Balances of any tokens held in the wallet

## Rust

```rust
let info = sdk
    .get_info(GetInfoRequest {
        // ensure_synced: true will ensure the SDK is synced with the Spark network
        // before returning the balance
        ensure_synced: Some(false),
    })
    .await?;
let identity_pubkey = &info.identity_pubkey;
let balance_sats = info.balance_sats;
```

## Swift

```swift
// ensureSynced: true will ensure the SDK is synced with the Spark network
// before returning the balance
let info = try await sdk.getInfo(
    request: GetInfoRequest(
        ensureSynced: false
    ))
let identityPubkey = info.identityPubkey
let balanceSats = info.balanceSats
```

## Kotlin

```kotlin
try {
    // ensureSynced: true will ensure the SDK is synced with the Spark network
    // before returning the balance
    val info = sdk.getInfo(GetInfoRequest(false))
    val identityPubkey = info.identityPubkey
    val balanceSats = info.balanceSats
} catch (e: Exception) {
    // handle error
}
```

## C#

```csharp
// ensureSynced: true will ensure the SDK is synced with the Spark network
// before returning the balance
var info = await sdk.GetInfo(request: new GetInfoRequest(ensureSynced: false));
var identityPubkey = info.identityPubkey;
var balanceSats = info.balanceSats;
```

## Javascript (Wasm)

```typescript
const info = await sdk.getInfo({
  // ensureSynced: true will ensure the SDK is synced with the Spark network
  // before returning the balance
  ensureSynced: false
})
const identityPubkey = info.identityPubkey
const balanceSats = info.balanceSats
```

## React Native

```typescript
// ensureSynced: true will ensure the SDK is synced with the Spark network
// before returning the balance
const info = await sdk.getInfo({
  ensureSynced: false
})
const identityPubkey = info.identityPubkey
const balanceSats = info.balanceSats
```

## Flutter

```dart
// ensureSynced: true will ensure the SDK is synced with the Spark network
// before returning the balance
final info = await sdk.getInfo(request: GetInfoRequest(ensureSynced: false));
final identityPubkey = info.identityPubkey;
final balanceSats = info.balanceSats;
```

## Python

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

## Go

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

The SDK keeps a **cached balance** in local storage and `get_info` reads from this cache for a low-latency response. The cache is refreshed automatically by the SDK's background sync.

The recommended pattern is:

1. Call `get_info` with `ensure_synced` = **false** whenever you need to render the balance.
2. Subscribe to events and call `get_info` again on each `SdkEvent::Synced` event to fetch the latest balance. See [Listening to events](/guide/events.md).

| Event | Description | UX Suggestion |
| ----- | ----------- | ------------- |
| `SdkEvent::Synced` | The SDK has synced with the network in the background. | Call `get_info` to refresh the displayed balance, and refresh the payments list. See [listing payments](/guide/list_payments.md). |

**Developer note**

`ensure_synced` = **true** blocks until the SDK's **initial** sync after `connect` completes. This is useful for short-lived scripts that connect, read the balance once, and disconnect. It is **not** a "force a fresh sync now" call. In long-running applications, prefer `ensure_synced` = **false** combined with the `SdkEvent::Synced` event listener pattern above.

## Server mode

When the SDK is built with [Server mode](server_mode.md), `get_info` reads the balance live from the spark wallet's local tree store rather than from the background-maintained cache. As a result:

- `ensure_synced` = **true** is rejected with an invalid-input error. The SDK has no initial-sync watcher to await; call `sync_wallet` explicitly if you need to refresh state first.
- The returned balance reflects whatever is currently in the local tree store. If you need the freshest possible balance after an external state change (an incoming Spark transfer claimed elsewhere, an on-chain deposit confirmed, etc.), call `sync_wallet` first.

---

Identifier casing: `get_info` here is `getInfo` in Swift, Kotlin, JavaScript, React Native and Flutter, and `GetInfo` in Go and C#. Enum variants: `SdkEvent::Synced` is `SdkEvent.SYNCED` in Python, `SdkEvent.synced` in Swift, `SdkEventSynced` in Go, and `SdkEvent.Synced` elsewhere.
