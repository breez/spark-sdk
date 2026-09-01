# Initializing the SDK

API docs: https://breez.github.io/spark-sdk/breez_sdk_spark/struct.BreezSdk.html#method.connect

## Basic Initialization

The easiest way to initialize the SDK is with the `connect` method. This method requires:

- The network, mnemonic, and Breez API key you intend to use
- A storage directory path where the SDK can manage its data

**Developer note**

For WASM Web, SDK storage is managed using IndexedDB.

The storage is used to persist the SDK’s state. If you run multiple SDK instances, each must have its own unique storage directory.

Once connected, you’re ready to start interacting with the SDK.

### Rust

```rust
// Construct the seed using a mnemonic, entropy or passkey
let mnemonic = "<mnemonic words>".to_string();
let seed = Seed::Mnemonic {
    mnemonic,
    passphrase: None,
};

// Create the default config
let mut config = default_config(Network::Mainnet);
config.api_key = Some("<breez api key>".to_string());

// Connect to the SDK using the simplified connect method
let sdk = connect(ConnectRequest {
    config,
    seed,
    storage_dir: "./.data".to_string(),
})
.await?;
```

### Swift

```swift
// Construct the seed using a mnemonic, entropy or passkey
let mnemonic = "<mnemonic words>"
let seed = Seed.mnemonic(mnemonic: mnemonic, passphrase: nil)

// Create the default config
var config = defaultConfig(network: Network.mainnet)
config.apiKey = "<breez api key>"

// Connect to the SDK using the simplified connect method
let sdk = try await connect(
    request: ConnectRequest(
        config: config,
        seed: seed,
        storageDir: "./.data"
    ))
```

### Kotlin

```kotlin
// Construct the seed using a mnemonic, entropy or passkey
val mnemonic = "<mnemonic words>"
val seed = Seed.Mnemonic(mnemonic, null)

// Create the default config
val config = defaultConfig(Network.MAINNET)
config.apiKey = "<breez api key>"

try {
    // Connect to the SDK using the simplified connect method
    val sdk = connect(ConnectRequest(
        config = config,
        seed = seed,
        storageDir = "./.data"
    ))
} catch (e: Exception) {
    // handle error
}
```

### C#

```csharp
// Construct the seed using a mnemonic, entropy or passkey
var mnemonic = "<mnemonic words>";
var seed = new Seed.Mnemonic(mnemonic: mnemonic, passphrase: null);
// Create the default config
var config = BreezSdkSparkMethods.DefaultConfig(Network.Mainnet) with
{
    apiKey = "<breez api key>"
};
// Connect to the SDK using the simplified connect method
var sdk = await BreezSdkSparkMethods.Connect(
    request: new ConnectRequest(
        config: config,
        seed: seed,
        storageDir: "./.data"
    )
);
```

### Javascript (Wasm)

```typescript
// Call init to load the WASM module before calling any other SDK methods.
// This is not needed when using the SDK via require() in Node.js.
//
// For SSR frameworks (Next.js, SvelteKit, Nuxt), use the /ssr subpath:
//   import init, { connect } from '@breeztech/breez-sdk-spark/ssr'
// The /ssr import is safe during server-side rendering. Call init() on the
// client only (e.g., inside useEffect or onMount).
//
// import init from '@breeztech/breez-sdk-spark'
await init()

// Construct the seed using a mnemonic, entropy or passkey
const mnemonic = '<mnemonic words>'
const seed: Seed = { type: 'mnemonic', mnemonic, passphrase: undefined }

// Create the default config
const config = defaultConfig('mainnet')
config.apiKey = '<breez api key>'

// Connect to the SDK using the simplified connect method
const sdk = await connect({
  config,
  seed,
  storageDir: './.data'
})
```

### React Native

```typescript
// Construct the seed using a mnemonic, entropy or passkey
const mnemonic = '<mnemonics words>'
const seed = new Seed.Mnemonic({ mnemonic, passphrase: undefined })

// Create the default config
const config = defaultConfig(Network.Mainnet)
config.apiKey = '<breez api key>'

const sdk = await connect({
  config,
  seed,
  storageDir: `${RNFS.DocumentDirectoryPath}/data`
})
```

### Flutter

```dart
// Call once on your Dart entrypoint file, e.g.; `lib/main.dart`
// or singleton SDK service. It is recommended to use a single instance
// of the SDK across your Flutter app.
await BreezSdkSparkLib.init();

// Construct the seed using a mnemonic, entropy or passkey
String mnemonic = "<mnemonic words>";
final seed = Seed.mnemonic(mnemonic: mnemonic, passphrase: null);

// Create the default config
final config = defaultConfig(network: Network.mainnet)
    .copyWith(apiKey: "<breez api key>");

final connectRequest =
    ConnectRequest(config: config, seed: seed, storageDir: "./.data");

final sdk = await connect(request: connectRequest);
```

### Python

```python
# Construct the seed using a mnemonic, entropy or passkey
mnemonic = "<mnemonic words>"
seed = Seed.MNEMONIC(mnemonic=mnemonic, passphrase=None)
# Create the default config
config = default_config(network=Network.MAINNET)
config.api_key = "<breez api key>"
try:
    # Connect to the SDK using the simplified connect method
    sdk = await connect(
        request=ConnectRequest(config=config, seed=seed, storage_dir="./.data")
    )
    return sdk
except Exception as error:
    logging.error(error)
    raise
```

### Go

```go
// Construct the seed using a mnemonic, entropy or passkey
mnemonic := "<mnemonic words>"
var seed breez_sdk_spark.Seed = breez_sdk_spark.SeedMnemonic{
	Mnemonic:   mnemonic,
	Passphrase: nil,
}

// Create the default config
apiKey := "<breez api key>"
config := breez_sdk_spark.DefaultConfig(breez_sdk_spark.NetworkMainnet)
config.ApiKey = &apiKey

connectRequest := breez_sdk_spark.ConnectRequest{
	Config:     config,
	Seed:       seed,
	StorageDir: "./.data",
}

// Connect to the SDK using the simplified connect method
sdk, err := breez_sdk_spark.Connect(connectRequest)

return sdk, err
```



**Developer note**

On some platforms (e.g., Android, iOS), you must use an application-specific writable directory within the app's sandbox for SDK storage.

### Connecting with a Passkey

Instead of managing mnemonics directly, you can use passkeys to derive wallet seeds deterministically. This eliminates the need for mnemonic backup and provides a seamless authentication experience using biometrics or device PIN.

See [Connecting with a Passkey](passkey.md) for the full setup guide including PRF provider implementation, platform configuration, and label management.

## Advanced Initialization

If you're building a multi-tenant server deployment, start with [Server mode](server_mode.md) for the recommended profile, lifecycle pattern, and shared-infrastructure wiring.

For advanced use cases where you need more control, you can configure the SDK using the Builder pattern. With the SDK Builder you can define:

- [Storage](customizing.md#with-storage) to manage stored data
- [Bitcoin Chain Service](customizing.md#with-chain-service) to provide network data
- [LNURL Client](customizing.md#with-lnurl-client) to make REST requests
- [Fiat Service](customizing.md#with-fiat-service) to provide Fiat currencies and exchange rates 
- Change the [Account Number](customizing.md#with-account-number) to derive an independent wallet from the same seed
- [Payment Observer](customizing.md#with-payment-observer) to be notified before payments occur

See [Customizing the SDK](customizing.md) for examples of this advanced initialization pattern.

## Disconnecting

API docs: https://breez.github.io/spark-sdk/breez_sdk_spark/struct.BreezSdk.html#method.disconnect

When you’re done using the SDK, call the disconnect method to release any resources in use.

This is particularly useful if you need to re-instantiate the SDK, such as when changing the mnemonic or updating configuration.

### Rust

```rust
pub(crate) async fn disconnect(sdk: &BreezSdk) -> Result<()> {
    sdk.disconnect().await?;
    Ok(())
}
```

### Swift

```swift
func disconnect(sdk: BreezSdk) async throws {
    try await sdk.disconnect()
}
```

### Kotlin

```kotlin
suspend fun disconnect(sdk: BreezSdk)  {
    try {
        sdk.disconnect()
    } catch (e: Exception) {
        // handle error
    }
}
```

### C#

```csharp
async Task Disconnect(BreezSdk sdk)
{
    await sdk.Disconnect();
}
```

### Javascript (Wasm)

```typescript
await sdk.disconnect()
```

### React Native

```typescript
await sdk.disconnect()
```

### Flutter

```dart
Future<void> disconnect(BreezSdk sdk) async {
  await sdk.disconnect();
}
```

### Python

```python
async def disconnect(sdk: BreezSdk):
    try:
        await sdk.disconnect()
    except Exception as error:
        logging.error(error)
        raise
```

### Go

```go
func Disconnect(sdk *breez_sdk_spark.BreezSdk) {
	sdk.Disconnect()
}
```



---

Identifier casing: `get_info` here is `getInfo` in Swift, Kotlin, JavaScript, React Native and Flutter, and `GetInfo` in Go and C#. Enum variants: `SdkEvent::Synced` is `SdkEvent.SYNCED` in Python, `SdkEvent.synced` in Swift, `SdkEventSynced` in Go, and `SdkEvent.Synced` elsewhere.
