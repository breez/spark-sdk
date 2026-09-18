# Treasury configuration

A treasury wallet is a single wallet the service itself owns, held in one long-lived SDK instance: a float, a settlement account, or the balance a backend pays out of and receives into.

This is a standard deployment. Background tasks stay on and the instance stays connected, so the rest of this guide applies unchanged. Build the config with `default_config`, not `default_server_config`: that preset turns off the background tasks a treasury relies on and exists for the [multi-user configuration](server_mode.md).

Three defaults are chosen for a mobile wallet and are worth changing:

## Rust

```rust
// Construct the seed using a mnemonic, entropy or passkey
let mnemonic = "<mnemonic words>".to_string();
let seed = Seed::Mnemonic {
    mnemonic,
    passphrase: None,
};

// A treasury wallet is an ordinary long-lived SDK instance, so start from
// default_config and keep background tasks on.
let mut config = default_config(Network::Mainnet);
config.api_key = Some("<breez api key>".to_string());

// Keeps user data in step across devices that each hold their own
// storage, which a server-side wallet has no use for.
config.real_time_sync_server_url = None;

// Stays connected, and syncs cost more with more leaves, so it can
// afford a longer interval.
config.sync_interval_secs = 300;

// Optional: on a treasury that pays often and holds many leaves, collecting
// unilateral exit data behind every operation adds up. Turn it off and
// sync on a cadence of your own instead: a sync collects regardless of
// this flag.
config.exit_chain_auto_fetch_enabled = false;

let sdk = SdkBuilder::new(config, seed)
    .with_default_storage("./.data".to_string())
    .build()
    .await?;
```

## Swift

```swift
// Construct the seed using a mnemonic, entropy or passkey
let mnemonic = "<mnemonic words>"
let seed = Seed.mnemonic(mnemonic: mnemonic, passphrase: nil)

// A treasury wallet is an ordinary long-lived SDK instance, so start from
// defaultConfig and keep background tasks on.
var config = defaultConfig(network: Network.mainnet)
config.apiKey = "<breez api key>"

// Keeps user data in step across devices that each hold their own
// storage, which a server-side wallet has no use for.
config.realTimeSyncServerUrl = nil

// Stays connected, and syncs cost more with more leaves, so it can
// afford a longer interval.
config.syncIntervalSecs = UInt32(300)

// Optional: on a treasury that pays often and holds many leaves, collecting
// unilateral exit data behind every operation adds up. Turn it off and
// sync on a cadence of your own instead: a sync collects regardless of
// this flag.
config.exitChainAutoFetchEnabled = false

let builder = SdkBuilder(config: config, seed: seed)
await builder.withDefaultStorage(storageDir: "./.data")
let sdk = try await builder.build()
```

## Kotlin

```kotlin
// Construct the seed using a mnemonic, entropy or passkey
val mnemonic = "<mnemonic words>"
val seed = Seed.Mnemonic(mnemonic, null)

// A treasury wallet is an ordinary long-lived SDK instance, so start
// from defaultConfig and keep background tasks on.
val config = defaultConfig(Network.MAINNET)
config.apiKey = "<breez api key>"

// Keeps user data in step across devices that each hold their own
// storage, which a server-side wallet has no use for.
config.realTimeSyncServerUrl = null

// Stays connected, and syncs cost more with more leaves, so it can
// afford a longer interval.
config.syncIntervalSecs = 300u

// Optional: on a treasury that pays often and holds many leaves,
// collecting unilateral exit data behind every operation adds up. Turn
// it off and sync on a cadence of your own instead: a sync collects
// regardless of this flag.
config.exitChainAutoFetchEnabled = false

try {
    val builder = SdkBuilder(config, seed)
    builder.withDefaultStorage("./.data")
    val sdk = builder.build()
} catch (e: Exception) {
    // handle error
}
```

## C#

```csharp
// Construct the seed using a mnemonic, entropy or passkey
var mnemonic = "<mnemonic words>";
var seed = new Seed.Mnemonic(mnemonic: mnemonic, passphrase: null);

// A treasury wallet is an ordinary long-lived SDK instance, so start
// from DefaultConfig and keep background tasks on.
var config = BreezSdkSparkMethods.DefaultConfig(Network.Mainnet) with
{
    apiKey = "<breez api key>"
};

// Keeps user data in step across devices that each hold their own
// storage, which a server-side wallet has no use for.
config = config with { realTimeSyncServerUrl = null };

// Stays connected, and syncs cost more with more leaves, so it can
// afford a longer interval.
config = config with { syncIntervalSecs = 300U };

// Optional: on a treasury that pays often and holds many leaves,
// collecting unilateral exit data behind every operation adds up.
// Turn it off and sync on a cadence of your own instead: a sync
// collects regardless of this flag.
config = config with { exitChainAutoFetchEnabled = false };

var builder = new SdkBuilder(config: config, seed: seed);
await builder.WithDefaultStorage(storageDir: "./.data");
var sdk = await builder.Build();
```

## Javascript (Wasm)

```typescript
// Construct the seed using a mnemonic, entropy or passkey
const mnemonic = '<mnemonic words>'
const seed: Seed = { type: 'mnemonic', mnemonic, passphrase: undefined }

// A treasury wallet is an ordinary long-lived SDK instance, so start from
// defaultConfig and keep background tasks on.
const config = defaultConfig('mainnet')
config.apiKey = '<breez api key>'

// Keeps user data in step across devices that each hold their own
// storage, which a server-side wallet has no use for.
config.realTimeSyncServerUrl = undefined

// Stays connected, and syncs cost more with more leaves, so it can
// afford a longer interval.
config.syncIntervalSecs = 300

// Optional: on a treasury that pays often and holds many leaves, collecting
// unilateral exit data behind every operation adds up. Turn it off and
// sync on a cadence of your own instead: a sync collects regardless of
// this flag.
config.exitChainAutoFetchEnabled = false

let builder = SdkBuilder.new(config, seed)
builder = await builder.withDefaultStorage('./.data')
const sdk = await builder.build()
```

## Python

```python
# Construct the seed using a mnemonic, entropy or passkey
mnemonic = "<mnemonic words>"
seed = Seed.MNEMONIC(mnemonic=mnemonic, passphrase=None)

# A treasury wallet is an ordinary long-lived SDK instance, so start from
# default_config and keep background tasks on.
config = default_config(network=Network.MAINNET)
config.api_key = "<breez api key>"

# Keeps user data in step across devices that each hold their own
# storage, which a server-side wallet has no use for.
config.real_time_sync_server_url = None

# Stays connected, and syncs cost more with more leaves, so it can
# afford a longer interval.
config.sync_interval_secs = 300

# Optional: on a treasury that pays often and holds many leaves, collecting
# unilateral exit data behind every operation adds up. Turn it off and
# sync on a cadence of your own instead: a sync collects regardless of
# this flag.
config.exit_chain_auto_fetch_enabled = False

try:
    builder = SdkBuilder(config=config, seed=seed)
    await builder.with_default_storage(storage_dir="./.data")
    sdk = await builder.build()
    return sdk
except Exception as error:
    logging.error(error)
    raise
```

## Go

```go
// Construct the seed using a mnemonic, entropy or passkey
mnemonic := "<mnemonic words>"
var seed breez_sdk_spark.Seed = breez_sdk_spark.SeedMnemonic{
	Mnemonic:   mnemonic,
	Passphrase: nil,
}

// A treasury wallet is an ordinary long-lived SDK instance, so start from
// DefaultConfig and keep background tasks on.
apiKey := "<breez api key>"
config := breez_sdk_spark.DefaultConfig(breez_sdk_spark.NetworkMainnet)
config.ApiKey = &apiKey

// Keeps user data in step across devices that each hold their own
// storage, which a server-side wallet has no use for.
config.RealTimeSyncServerUrl = nil

// Stays connected, and syncs cost more with more leaves, so it can
// afford a longer interval.
config.SyncIntervalSecs = 300

// Optional: on a treasury that pays often and holds many leaves, collecting
// unilateral exit data behind every operation adds up. Turn it off and
// sync on a cadence of your own instead: a sync collects regardless of
// this flag.
config.ExitChainAutoFetchEnabled = false

builder := breez_sdk_spark.NewSdkBuilder(config, seed)
builder.WithDefaultStorage("./.data")
sdk, err := builder.Build()
if err != nil {
	return nil, err
}
```



## Disable real-time sync

The [real-time sync server](./config.md#real-time-sync-server-url) keeps wallet data in step across devices with separate storage. A treasury has one storage layer, so the WebSocket subscription and the upload after every change serve no purpose.

Unset `real_time_sync_server_url`.

## Lengthen the synchronization interval

The [periodic sync](./config.md#synchronization-interval) defaults to 60 seconds. That suits a phone, whose event stream drops whenever the app is backgrounded. A treasury stays connected, and its syncs cost more because it holds more leaves.

300 seconds is a reasonable starting point. A wallet that settles rarely can go longer.

## Schedule the unilateral exit data collection

By default the SDK collects the data a [unilateral exit](unilateral_exit.md) needs in the background whenever the wallet gains leaves. On a treasury that pays often and holds many leaves, that is a round trip to the operators behind almost every operation.

Set `exit_chain_auto_fetch_enabled` to `false` to disable the automatic collection. `sync_wallet` collects the missing data regardless of the flag and returns once it is done, so call it on a schedule of your own:

### Rust

```rust
// With automatic collection off, an explicit sync is what collects the data
// a unilateral exit needs, and it waits for the collection to finish. Needs
// the Spark operators reachable, so run it on a schedule rather than at the
// moment an exit is needed.
sdk.sync_wallet(SyncWalletRequest {}).await?;
```

### Swift

```swift
// With automatic collection off, an explicit sync is what collects the data
// a unilateral exit needs, and it waits for the collection to finish. Needs
// the Spark operators reachable, so run it on a schedule rather than at the
// moment an exit is needed.
let _ = try await sdk.syncWallet(request: SyncWalletRequest())
```

### Kotlin

```kotlin
// With automatic collection off, an explicit sync is what collects the data
// a unilateral exit needs, and it waits for the collection to finish. Needs
// the Spark operators reachable, so run it on a schedule rather than at the
// moment an exit is needed.
sdk.syncWallet(SyncWalletRequest)
```

### C#

```csharp
// With automatic collection off, an explicit sync is what collects the data
// a unilateral exit needs, and it waits for the collection to finish. Needs
// the Spark operators reachable, so run it on a schedule rather than at the
// moment an exit is needed.
await sdk.SyncWallet(request: new SyncWalletRequest());
```

### Javascript (Wasm)

```typescript
// With automatic collection off, an explicit sync is what collects the data
// a unilateral exit needs, and it waits for the collection to finish. Needs
// the Spark operators reachable, so run it on a schedule rather than at the
// moment an exit is needed.
await sdk.syncWallet({})
```

### React Native

```typescript
// With automatic collection off, an explicit sync is what collects the data
// a unilateral exit needs, and it waits for the collection to finish. Needs
// the Spark operators reachable, so run it on a schedule rather than at the
// moment an exit is needed.
await sdk.syncWallet({})
```

### Flutter

```dart
// With automatic collection off, an explicit sync is what collects the data
// a unilateral exit needs, and it waits for the collection to finish. Needs
// the Spark operators reachable, so run it on a schedule rather than at the
// moment an exit is needed.
await sdk.syncWallet(request: SyncWalletRequest());
```

### Python

```python
# With automatic collection off, an explicit sync is what collects the data
# a unilateral exit needs, and it waits for the collection to finish. Needs
# the Spark operators reachable, so run it on a schedule rather than at the
# moment an exit is needed.
await sdk.sync_wallet(request=SyncWalletRequest())
```

### Go

```go
// With automatic collection off, an explicit sync is what collects the data
// a unilateral exit needs, and it waits for the collection to finish. Needs
// the Spark operators reachable, so run it on a schedule rather than at the
// moment an exit is needed.
_, err := sdk.SyncWallet(breez_sdk_spark.SyncWalletRequest{})
if err != nil {
	return err
}
```



Funds received between two syncs cannot be exited without the operators until the next one. Pick a schedule with a gap you can accept, and keep the default if the automatic collection is not a measurable cost.

---

Identifier casing: `get_info` here is `getInfo` in Swift, Kotlin, JavaScript, React Native and Flutter, and `GetInfo` in Go and C#. Enum variants: `SdkEvent::Synced` is `SdkEvent.SYNCED` in Python, `SdkEvent.synced` in Swift, `SdkEventSynced` in Go, and `SdkEvent.Synced` elsewhere.
