# Using an External Signer

The External Signer feature allows you to provide custom signing logic for the SDK rather than relying on the SDK's internal key management. This is useful when you want to:

- Keep keys in a secured environment
- Implement custom key derivation logic
- Integrate with existing wallet infrastructure

## Using the Default External Signers

The external signer interface is split into two parts: an `ExternalBreezSigner` for SDK-layer signing (LNURL-auth, sync, message signing, ECIES) and an `ExternalSparkSigner` for the Spark wallet flows (transfers, claims, FROST signing, deposits). The SDK also ships a Turnkey-backed implementation that keeps the keys in a secure enclave; see [Using Turnkey](turnkey.md).

The SDK provides a convenient factory function `default_external_signers` that creates both signers from a mnemonic:

### Rust

```rust
fn create_signers() -> Result<ExternalSigners, SdkError> {
    let mnemonic = "<mnemonic words>".to_string();
    let network = Network::Mainnet;

    let signers = default_external_signers(
        mnemonic,
        None, // passphrase
        network,
        Some(0), // account number
    )?;

    Ok(signers)
}
```

### Swift

```swift
func createSigners() throws -> ExternalSigners {
    let mnemonic = "<mnemonic words>"
    let network = Network.mainnet

    let signers = try defaultExternalSigners(
        mnemonic: mnemonic,
        passphrase: nil,
        network: network,
        accountNumber: 0
    )

    return signers
}
```

### Kotlin

```kotlin
fun createSigners(): breez_sdk_spark.ExternalSigners {
    val mnemonic = "<mnemonic words>"
    val network = Network.MAINNET
    val accountNumber = 0U

    val signers = defaultExternalSigners(
        mnemonic = mnemonic,
        passphrase = null,
        network = network,
        accountNumber = accountNumber
    )

    return signers
}
```

### C#

```csharp
public static ExternalSigners CreateSigners()
{
    var mnemonic = "<mnemonic words>";
    var network = Network.Mainnet;
    uint accountNumber = 0;

    var signers = BreezSdkSparkMethods.DefaultExternalSigners(
        mnemonic: mnemonic,
        passphrase: null,
        network: network,
        accountNumber: accountNumber
    );

    return signers;
}
```

### Javascript (Wasm)

```typescript
const createSigners = () => {
  const mnemonic = '<mnemonic words>'
  const accountNumber = 0

  // Create the default signers from the SDK
  const signers = defaultExternalSigners(mnemonic, null, 'mainnet', accountNumber)

  return signers
}
```

### React Native

```typescript
const createSigners = () => {
  const mnemonic = '<mnemonic words>'
  const accountNumber = 0

  // Create the default signers from the SDK
  const signers = defaultExternalSigners(mnemonic, undefined, Network.Mainnet, accountNumber)

  return signers
}
```

### Python

```python
def create_signers() -> ExternalSigners:
    mnemonic = "<mnemonic words>"
    network = Network.MAINNET
    account_number = 0

    signers = default_external_signers(
        mnemonic=mnemonic,
        passphrase=None,
        network=network,
        account_number=account_number,
    )

    return signers
```

### Go

```go
func createSigners() (breez_sdk_spark.ExternalSigners, error) {
	mnemonic := "<mnemonic words>"
	network := breez_sdk_spark.NetworkMainnet
	var accountNumber uint32 = 0

	signers, err := breez_sdk_spark.DefaultExternalSigners(
		mnemonic,
		nil, // passphrase
		network,
		&accountNumber,
	)
	if err != nil {
		var sdkErr *breez_sdk_spark.SdkError
		if errors.As(err, &sdkErr) {
			// Handle SdkError - can inspect specific variants if needed
			// e.g., switch on sdkErr variant for InsufficientFunds, NetworkError, etc.
		}
		return breez_sdk_spark.ExternalSigners{}, err
	}

	return signers, nil
}
```



Provide both signers to the `connect_with_signer` method instead of the regular `connect` method:

### Rust

```rust
async fn connect_example(signers: ExternalSigners) -> Result<BreezSdk, SdkError> {
    // Create the config
    let mut config = default_config(Network::Mainnet);
    config.api_key = Some("<breez api key>".to_string());

    // Connect using the external signers
    let sdk = connect_with_signer(ConnectWithSignerRequest {
        config,
        breez_signer: signers.breez_signer,
        spark_signer: signers.spark_signer,
        storage_dir: "./.data".to_string(),
    })
    .await?;

    Ok(sdk)
}
```

### Swift

```swift
func connectExample(signers: ExternalSigners) async throws -> BreezSdk {
    // Create the config
    var config = defaultConfig(network: .mainnet)
    config.apiKey = "<breez api key>"

    // Connect using the external signers
    let sdk = try await BreezSdkSpark.connectWithSigner(request: ConnectWithSignerRequest(
        config: config,
        breezSigner: signers.breezSigner,
        sparkSigner: signers.sparkSigner,
        storageDir: "./.data"
    ))

    return sdk
}
```

### Kotlin

```kotlin
suspend fun connectWithSigner(signers: breez_sdk_spark.ExternalSigners) {
    // Create the config
    val config = defaultConfig(Network.MAINNET)
    config.apiKey = "<breez api key>"

    try {
        // Connect using the external signers
        val sdk = connectWithSigner(ConnectWithSignerRequest(
            config = config,
            breezSigner = signers.breezSigner,
            sparkSigner = signers.sparkSigner,
            storageDir = "./.data"
        ))
    } catch (e: Exception) {
        // handle error
    }
}
```

### C#

```csharp
public static async Task<BreezSdk> ConnectWithSigner(ExternalSigners signers)
{
    // Create the config
    var config = BreezSdkSparkMethods.DefaultConfig(Network.Mainnet) with
    {
        apiKey = "<breez api key>"
    };

    // Connect using the external signers
    var sdk = await BreezSdkSparkMethods.ConnectWithSigner(new ConnectWithSignerRequest(
        config: config,
        breezSigner: signers.breezSigner,
        sparkSigner: signers.sparkSigner,
        storageDir: "./.data"
    ));

    return sdk;
}
```

### Javascript (Wasm)

```typescript
const exampleConnectWithSigner = async (
  signers: ReturnType<typeof defaultExternalSigners>
) => {
  // Create the config
  const config = defaultConfig('mainnet')
  config.apiKey = '<breez api key>'

  // Connect using the external signers
  const sdk = await connectWithSigner(
    config,
    signers.breezSigner,
    signers.sparkSigner,
    'breez_spark_db' // For WASM, this is the IndexedDB database name
  )
}
```

### React Native

```typescript
const exampleConnectWithSigner = async (
  signers: ReturnType<typeof defaultExternalSigners>
) => {
  // Create the config
  const config = defaultConfig(Network.Mainnet)
  config.apiKey = '<breez api key>'

  // Connect using the external signers
  const sdk = await connectWithSigner({
    config,
    breezSigner: signers.breezSigner,
    sparkSigner: signers.sparkSigner,
    storageDir: `${RNFS.DocumentDirectoryPath}/data`
  })
}
```

### Python

```python
async def example_connect_with_signer(signers: ExternalSigners) -> BreezSdk:
    # Create the config
    config = default_config(Network.MAINNET)
    config.api_key = "<breez api key>"

    # Connect using the external signers
    sdk = await connect_with_signer(ConnectWithSignerRequest(
        config=config,
        breez_signer=signers.breez_signer,
        spark_signer=signers.spark_signer,
        storage_dir="./.data"
    ))

    return sdk
```

### Go

```go
func connectWithSigner(
	signers breez_sdk_spark.ExternalSigners,
) (*breez_sdk_spark.BreezSdk, error) {
	// Create the config
	config := breez_sdk_spark.DefaultConfig(breez_sdk_spark.NetworkMainnet)
	apiKey := "<breez api key>"
	config.ApiKey = &apiKey

	// Connect using the external signers
	sdk, err := breez_sdk_spark.ConnectWithSigner(breez_sdk_spark.ConnectWithSignerRequest{
		Config:      config,
		BreezSigner: signers.BreezSigner,
		SparkSigner: signers.SparkSigner,
		StorageDir:  "./.data",
	})
	if err != nil {
		var sdkErr *breez_sdk_spark.SdkError
		if errors.As(err, &sdkErr) {
			// Handle SdkError - can inspect specific variants if needed
			// e.g., switch on sdkErr variant for InsufficientFunds, NetworkError, etc.
		}
		return nil, err
	}

	return sdk, nil
}
```



**Developer note**

When using an external signer, you don't provide a seed directly to the SDK. Instead, the signer handles all cryptographic operations internally.

## Advanced Setup with Sdk Builder

To compose an external signer along with the options in [customizing the SDK](./customizing.md) (custom storage backends, a shared SDK context, an account number), build the SDK with `new_with_signer` instead. It takes the same two signers and returns an `SdkBuilder` you chain the customization methods on before calling `build`:

### Rust

```rust
async fn build_with_signer(signers: ExternalSigners) -> Result<BreezSdk, SdkError> {
    let mut config = default_config(Network::Mainnet);
    config.api_key = Some("<breez api key>".to_string());

    let builder = SdkBuilder::new_with_signer(config, signers.breez_signer, signers.spark_signer);
    // let builder = builder.with_storage_backend(<your storage backend>);
    // let builder = builder.with_shared_context(<your shared context>);
    let sdk = builder.build().await?;

    Ok(sdk)
}
```

### Swift

```swift
func buildWithSigner(signers: ExternalSigners) async throws -> BreezSdk {
    var config = defaultConfig(network: .mainnet)
    config.apiKey = "<breez api key>"

    let builder = SdkBuilder.newWithSigner(
        config: config,
        breezSigner: signers.breezSigner,
        sparkSigner: signers.sparkSigner
    )
    // await builder.withStorageBackend(storage: <your storage backend>)
    // await builder.withSharedContext(<your shared context>)
    let sdk = try await builder.build()

    return sdk
}
```

### Kotlin

```kotlin
suspend fun buildWithSigner(signers: breez_sdk_spark.ExternalSigners) {
    // Create the config
    val config = defaultConfig(Network.MAINNET)
    config.apiKey = "<breez api key>"

    try {
        val builder = SdkBuilder.newWithSigner(
            config,
            signers.breezSigner,
            signers.sparkSigner
        )
        // builder.withStorageBackend(<your storage backend>)
        // builder.withSharedContext(<your shared context>)
        val sdk = builder.build()
    } catch (e: Exception) {
        // handle error
    }
}
```

### C#

```csharp
public static async Task<BreezSdk> BuildWithSigner(ExternalSigners signers)
{
    var config = BreezSdkSparkMethods.DefaultConfig(Network.Mainnet) with
    {
        apiKey = "<breez api key>"
    };

    var builder = SdkBuilder.NewWithSigner(
        config: config,
        breezSigner: signers.breezSigner,
        sparkSigner: signers.sparkSigner
    );
    // await builder.WithStorageBackend(storage: <your storage backend>);
    // await builder.WithSharedContext(<your shared context>);
    var sdk = await builder.Build();

    return sdk;
}
```

### Javascript (Wasm)

```typescript
const exampleBuildWithSigner = async (
  signers: ExternalSigners
): Promise<BreezSdk> => {
  const config = defaultConfig('mainnet')
  config.apiKey = '<breez api key>'
  const builder = SdkBuilder.newWithSigner(
    config,
    signers.breezSigner,
    signers.sparkSigner
  )
  // builder = builder.withStorageBackend(<your storage backend>)
  // builder = builder.withSharedContext(<your shared context>)
  const sdk = await builder.build()
  return sdk
}
```

### React Native

```typescript
const exampleBuildWithSigner = async (signers: ExternalSigners) => {
  // Create the config
  const config = defaultConfig(Network.Mainnet)
  config.apiKey = '<breez api key>'

  const builder = SdkBuilder.newWithSigner(config, signers.breezSigner, signers.sparkSigner)
  // await builder.withStorage(<your storage implementation>)
  // await builder.withAccountNumber(<account number>)
  const sdk = await builder.build()
}
```

### Python

```python
async def example_build_with_signer(signers: ExternalSigners) -> BreezSdk:
    config = default_config(Network.MAINNET)
    config.api_key = "<breez api key>"
    builder = SdkBuilder.new_with_signer(
        config=config,
        breez_signer=signers.breez_signer,
        spark_signer=signers.spark_signer,
    )
    # await builder.with_storage_backend(<your storage backend>)
    # await builder.with_shared_context(<your shared context>)
    sdk = await builder.build()
    return sdk
```

### Go

```go
func buildWithSigner(
	signers breez_sdk_spark.ExternalSigners,
) (*breez_sdk_spark.BreezSdk, error) {
	config := breez_sdk_spark.DefaultConfig(breez_sdk_spark.NetworkMainnet)
	apiKey := "<breez api key>"
	config.ApiKey = &apiKey

	builder := breez_sdk_spark.SdkBuilderNewWithSigner(config, signers.BreezSigner, signers.SparkSigner)
	// builder.WithStorageBackend(<your storage backend>)
	// builder.WithSharedContext(<your shared context>)
	sdk, err := builder.Build()
	if err != nil {
		return nil, err
	}

	return sdk, nil
}
```



For a signer that provides signing only (see [Signers Without Local ECIES/HMAC Support](#signers-without-local-ecieshmac-support)), use `new_with_signing_only_signer` the same way:

### Rust

```rust
async fn build_with_signing_only_signer(
    config: Config,
    signers: SigningOnlyExternalSigners,
) -> Result<BreezSdk, SdkError> {
    let builder = SdkBuilder::new_with_signing_only_signer(
        config,
        signers.breez_signer,
        signers.spark_signer,
    );
    let sdk = builder.build().await?;

    Ok(sdk)
}
```

### Swift

```swift
func buildWithSigningOnlySigner(
    config: Config,
    signers: SigningOnlyExternalSigners
) async throws -> BreezSdk {
    let builder = SdkBuilder.newWithSigningOnlySigner(
        config: config,
        breezSigner: signers.breezSigner,
        sparkSigner: signers.sparkSigner
    )
    let sdk = try await builder.build()

    return sdk
}
```

### Kotlin

```kotlin
suspend fun buildWithSigningOnlySigner(
    config: breez_sdk_spark.Config,
    signers: breez_sdk_spark.SigningOnlyExternalSigners
) {
    try {
        val builder = SdkBuilder.newWithSigningOnlySigner(
            config,
            signers.breezSigner,
            signers.sparkSigner
        )
        val sdk = builder.build()
    } catch (e: Exception) {
        // handle error
    }
}
```

### C#

```csharp
public static async Task<BreezSdk> BuildWithSigningOnlySigner(Config config, SigningOnlyExternalSigners signers)
{
    var builder = SdkBuilder.NewWithSigningOnlySigner(
        config: config,
        breezSigner: signers.breezSigner,
        sparkSigner: signers.sparkSigner
    );
    var sdk = await builder.Build();

    return sdk;
}
```

### Javascript (Wasm)

```typescript
const exampleBuildWithSigningOnlySigner = async (
  config: Config,
  signers: SigningOnlyExternalSigners
): Promise<BreezSdk> => {
  const builder = SdkBuilder.newWithSigningOnlySigner(
    config,
    signers.breezSigner,
    signers.sparkSigner
  )
  const sdk = await builder.build()
  return sdk
}
```

### React Native

```typescript
const exampleBuildWithSigningOnlySigner = async (
  config: Config,
  signers: SigningOnlyExternalSigners
) => {
  const builder = SdkBuilder.newWithSigningOnlySigner(
    config,
    signers.breezSigner,
    signers.sparkSigner
  )
  const sdk = await builder.build()
}
```

### Python

```python
async def example_build_with_signing_only_signer(
    config: Config, signers: SigningOnlyExternalSigners
) -> BreezSdk:
    builder = SdkBuilder.new_with_signing_only_signer(
        config=config,
        breez_signer=signers.breez_signer,
        spark_signer=signers.spark_signer,
    )
    sdk = await builder.build()
    return sdk
```

### Go

```go
func buildWithSigningOnlySigner(
	config breez_sdk_spark.Config,
	signers breez_sdk_spark.SigningOnlyExternalSigners,
) (*breez_sdk_spark.BreezSdk, error) {
	builder := breez_sdk_spark.SdkBuilderNewWithSigningOnlySigner(config, signers.BreezSigner, signers.SparkSigner)
	sdk, err := builder.Build()
	if err != nil {
		return nil, err
	}

	return sdk, nil
}
```



## Implementing a Custom Signer

If you need full control over the signing process, you can implement the [ExternalBreezSigner](https://github.com/breez/spark-sdk/blob/main/crates/breez-sdk/core/src/signer/external.rs) and [ExternalSparkSigner](https://github.com/breez/spark-sdk/blob/main/crates/breez-sdk/core/src/signer/external_spark.rs) interfaces in your application. These interfaces define all the cryptographic operations the SDK needs.

The default implementations of the two interfaces, [DefaultExternalSigner](https://github.com/breez/spark-sdk/blob/main/crates/breez-sdk/core/src/signer/default_external.rs) and [DefaultExternalSparkSigner](https://github.com/breez/spark-sdk/blob/main/crates/breez-sdk/core/src/signer/default_external_spark.rs), can be used as a reference for what's expected.

**Developer note**

Implementing a custom signer requires deep understanding of Bitcoin cryptography. The default signer implementations provide a solid reference for what's expected.

Most applications should use the default external signers factory function rather than implementing their own.

**Flutter Limitation**

External signers are not supported in Flutter due to limitations with passing trait objects through the flutter_rust_bridge FFI. Flutter applications should use the standard `connect` method with mnemonic-based key management.

### Signers Without Local ECIES/HMAC Support

Some external signers can't perform the SDK's local ECIES/HMAC operations (for example, a policy-restricted enclave that won't release key material). For these, implement `ExternalSigningSigner` instead of `ExternalBreezSigner`, then connect with `connect_with_signing_only_signer`. With such a signer:

- **LNURL-auth** returns an error when called.
- **Real-time sync** must be disabled: leave [`real_time_sync_server_url`](./config.md#real-time-sync-server-url) unset, or the build fails.
- **Cross-chain** must be disabled: leave [`cross_chain_config`](./config.md#send-usdc-usdt) unset, or the build fails.

---

Identifier casing: `get_info` here is `getInfo` in Swift, Kotlin, JavaScript, React Native and Flutter, and `GetInfo` in Go and C#. Enum variants: `SdkEvent::Synced` is `SdkEvent.SYNCED` in Python, `SdkEvent.synced` in Swift, `SdkEventSynced` in Go, and `SdkEvent.Synced` elsewhere.
