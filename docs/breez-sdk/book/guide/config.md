# Custom configuration

API docs: https://breez.github.io/spark-sdk/breez_sdk_spark/struct.Config.html

The SDK supports various configuration options to customize its behavior. During [initialization](./initializing.md#basic-initialization), you must provide a configuration object, which we recommend creating by modifying the default configuration. This page describes the available configuration options.

## Max deposit claim fee

Receiving Bitcoin payments through on-chain deposits may involve fees. This configuration option controls the automatic claiming of incoming funds, allowing it when the required fees are below specified thresholds. The available options are:

- Absolute fee amount in sats
- Feerate in sats/vbyte
- Fastest network recommended fee at the time of claim, with a leeway in sats/vbyte

You can also disable automatic claiming entirely. Deposits that are not automatically claimed require manual intervention.

By default, automatic claiming is enabled with a maximum feerate of 1 sats/vbyte.

More information can be found in the [Claiming on-chain deposits](./onchain_claims.md) page.

### Rust

```rust
// Create the default config
let mut config = default_config(Network::Mainnet);
config.api_key = Some("<breez api key>".to_string());

// Disable automatic claiming
config.max_deposit_claim_fee = None;

// Set a maximum feerate of 10 sat/vB
config.max_deposit_claim_fee = Some(MaxFee::Rate { sat_per_vbyte: 10 });

// Set a maximum fee of 1000 sat
config.max_deposit_claim_fee = Some(MaxFee::Fixed { amount: 1000 });

// Set the maximum fee to the fastest network recommended fee at the time of claim
// with a leeway of 1 sats/vbyte
config.max_deposit_claim_fee = Some(MaxFee::NetworkRecommended {
    leeway_sat_per_vbyte: 1,
});
```

### Swift

```swift
// Create the default config
var config = defaultConfig(network: Network.mainnet)
config.apiKey = "<breez api key>"

// Disable automatic claiming
config.maxDepositClaimFee = nil

// Set a maximum feerate of 10 sat/vB
config.maxDepositClaimFee = MaxFee.rate(satPerVbyte: 10)

// Set a maximum fee of 1000 sat
config.maxDepositClaimFee = MaxFee.fixed(amount: 1000)

// Set the maximum fee to the fastest network recommended fee at the time of claim
// with a leeway of 1 sats/vbyte
config.maxDepositClaimFee = MaxFee.networkRecommended(leewaySatPerVbyte: 1)
```

### Kotlin

```kotlin
// Create the default config
val config = defaultConfig(Network.MAINNET)
config.apiKey = "<breez api key>"

// Disable automatic claiming
config.maxDepositClaimFee = null

// Set a maximum feerate of 10 sat/vB
config.maxDepositClaimFee = MaxFee.Rate(10u)

// Set a maximum fee of 1000 sat
config.maxDepositClaimFee = MaxFee.Fixed(1000u)

// Set the maximum fee to the fastest network recommended fee at the time of claim
// with a leeway of 1 sats/vbyte
config.maxDepositClaimFee = MaxFee.NetworkRecommended(1u)
```

### C#

```csharp
// Create the default config with API key
var config = BreezSdkSparkMethods.DefaultConfig(Network.Mainnet) with
{
    apiKey = "<breez api key>"
};

// Disable automatic claiming
config = config with { maxDepositClaimFee = null };

// Set a maximum feerate of 10 sat/vB
config = config with { maxDepositClaimFee = new MaxFee.Rate(satPerVbyte: 10) };

// Set a maximum fee of 1000 sat
config = config with { maxDepositClaimFee = new MaxFee.Fixed(amount: 1000) };

// Set the maximum fee to the fastest network recommended fee at the time of claim
// with a leeway of 1 sats/vbyte
config = config with { maxDepositClaimFee = new MaxFee.NetworkRecommended(leewaySatPerVbyte: 1) };
```

### Javascript (Wasm)

```typescript
// Create the default config
const config = defaultConfig('mainnet')
config.apiKey = '<breez api key>'

// Disable automatic claiming
config.maxDepositClaimFee = undefined

// Set a maximum feerate of 10 sat/vB
config.maxDepositClaimFee = { type: 'rate', satPerVbyte: 10 }

// Set a maximum fee of 1000 sat
config.maxDepositClaimFee = { type: 'fixed', amount: 1000 }

// Set the maximum fee to the fastest network recommended fee at the time of claim
// with a leeway of 1 sats/vbyte
config.maxDepositClaimFee = { type: 'networkRecommended', leewaySatPerVbyte: 1 }
```

### React Native

```typescript
// Create the default config
const config = defaultConfig(Network.Mainnet)
config.apiKey = '<breez api key>'

// Disable automatic claiming
config.maxDepositClaimFee = undefined

// Set a maximum feerate of 10 sat/vB
config.maxDepositClaimFee = new MaxFee.Rate({ satPerVbyte: BigInt(10) })

// Set a maximum fee of 1000 sat
config.maxDepositClaimFee = new MaxFee.Fixed({ amount: BigInt(1000) })

// Set the maximum fee to the fastest network recommended fee at the time of claim
// with a leeway of 1 sats/vbyte
config.maxDepositClaimFee = new MaxFee.NetworkRecommended({ leewaySatPerVbyte: BigInt(1) })
```

### Flutter

```dart
// Create the default config
var config = defaultConfig(network: Network.mainnet)
    .copyWith(apiKey: "<breez api key>");

// Disable automatic claiming
config = config.copyWith(maxDepositClaimFee: null);

// Set a maximum feerate of 10 sat/vB
config = config.copyWith(
    maxDepositClaimFee: MaxFee.rate(satPerVbyte: BigInt.from(10)));

// Set a maximum fee of 1000 sat
config = config.copyWith(
    maxDepositClaimFee: MaxFee.fixed(amount: BigInt.from(1000)));

// Set the maximum fee to the fastest network recommended fee at the time of claim
// with a leeway of 1 sats/vbyte
config = config.copyWith(
    maxDepositClaimFee:
        MaxFee.networkRecommended(leewaySatPerVbyte: BigInt.from(1)));
```

### Python

```python
# Create the default config
config = default_config(network=Network.MAINNET)
config.api_key = "<breez api key>"

# Disable automatic claiming
config.max_deposit_claim_fee = None

# Set a maximum feerate of 10 sat/vB
config.max_deposit_claim_fee = MaxFee.RATE(sat_per_vbyte=10)

# Set a maximum fee of 1000 sat
config.max_deposit_claim_fee = MaxFee.FIXED(amount=1000)

# Set the maximum fee to the fastest network recommended fee at the time of claim
# with a leeway of 1 sats/vbyte
config.max_deposit_claim_fee = MaxFee.NETWORK_RECOMMENDED(leeway_sat_per_vbyte=1)
```

### Go

```go
// Create the default config
config := breez_sdk_spark.DefaultConfig(breez_sdk_spark.NetworkMainnet)
apiKey := "<breez api key>"
config.ApiKey = &apiKey

// Disable automatic claiming
config.MaxDepositClaimFee = nil

// Set a maximum feerate of 10 sat/vB
feeRateInterface := breez_sdk_spark.MaxFee(breez_sdk_spark.MaxFeeRate{SatPerVbyte: 10})
config.MaxDepositClaimFee = &feeRateInterface

// Set a maximum fee of 1000 sat
feeFixedInterface := breez_sdk_spark.MaxFee(breez_sdk_spark.MaxFeeFixed{Amount: 1000})
config.MaxDepositClaimFee = &feeFixedInterface

// Set the maximum fee to the fastest network recommended fee at the time of claim
// with a leeway of 1 sats/vbyte
networkRecommendedInterface := breez_sdk_spark.MaxFee(
	breez_sdk_spark.MaxFeeNetworkRecommended{LeewaySatPerVbyte: 1},
)
config.MaxDepositClaimFee = &networkRecommendedInterface
```



## Synchronization interval

The SDK performs regular background synchronization to check for payment status updates. You can configure how often this synchronization occurs.

The synchronization process is used to detect some payment status updates that are not detected in real-time through event streams.

A shorter synchronization interval provides more responsive detection of payment updates but increases resource usage and may trigger API rate limits. The default interval balances responsiveness with resource efficiency for most use cases.

## Background tasks enabled

Master switch for all per-instance background tasks. Defaults to `true`, which is the right choice for mobile and single-instance deployments — the SDK runs its periodic sync, real-time sync client, lightning-address recovery, spark private-mode init, leaf and token-output optimizers, the spark-wallet background processor, and the flashnet conversion refunder.

Set to `false` for multi-tenant server deployments where the SDK is built per request and the host orchestrates sync, claiming, and event delivery (typically via webhooks) explicitly. No background work is started; explicit operations such as `sync_wallet`, `claim_deposit`, `list_unclaimed_deposits`, `refund_deposit`, and `refund_pending_conversions` continue to work and are the intended entry points in this mode.

The recommended way to opt into server mode is via `default_server_config`, which returns the same `Config` as `default_config` with this flag flipped off. See [Server mode](./server_mode.md) for the full profile, lifecycle pattern, and shared-infrastructure wiring. Configuring this field directly is supported if you build your `Config` another way:

### Rust

```rust
// Server-mode profile: equivalent to default_server_config(Network::Mainnet).
// Recommended when you build the SDK per request in a multi-tenant server
// deployment. See the "Server mode" page for the full profile.
let mut config = default_config(Network::Mainnet);
config.background_tasks_enabled = false;
```

### Swift

```swift
// Server-mode profile: equivalent to defaultServerConfig(network: .mainnet).
// Recommended when you build the SDK per request in a multi-tenant server
// deployment. See the "Server mode" page for the full profile.
var config = defaultConfig(network: Network.mainnet)
config.backgroundTasksEnabled = false
```

### Kotlin

```kotlin
// Server-mode profile: equivalent to defaultServerConfig(Network.MAINNET).
// Recommended when you build the SDK per request in a multi-tenant
// server deployment. See the "Server mode" page for the full profile.
val config = defaultConfig(Network.MAINNET)
config.backgroundTasksEnabled = false
```

### C#

```csharp
// Server-mode profile: equivalent to DefaultServerConfig(Network.Mainnet).
// Recommended when you build the SDK per request in a multi-tenant
// server deployment. See the "Server mode" page for the full profile.
var config = BreezSdkSparkMethods.DefaultConfig(Network.Mainnet) with
{
    backgroundTasksEnabled = false
};
```

### Javascript (Wasm)

```typescript
// Server-mode profile: equivalent to defaultServerConfig('mainnet').
// Recommended when you build the SDK per request in a multi-tenant server
// deployment. See the "Server mode" page for the full profile.
const config = defaultConfig('mainnet')
config.backgroundTasksEnabled = false
```

### React Native

```typescript
// Server-mode profile: equivalent to defaultServerConfig(Network.Mainnet).
// Recommended when you build the SDK per request in a multi-tenant server
// deployment. See the "Server mode" page for the full profile.
const config = defaultConfig(Network.Mainnet)
config.backgroundTasksEnabled = false
```

### Flutter

```dart
// Server-mode profile: equivalent to defaultServerConfig(network: Network.mainnet).
// Recommended when you build the SDK per request in a multi-tenant server
// deployment. See the "Server mode" page for the full profile.
final config = defaultConfig(network: Network.mainnet)
    .copyWith(backgroundTasksEnabled: false);
```

### Python

```python
# Server-mode profile: equivalent to default_server_config(network=Network.MAINNET).
# Recommended when you build the SDK per request in a multi-tenant server
# deployment. See the "Server mode" page for the full profile.
config = default_config(network=Network.MAINNET)
config.background_tasks_enabled = False
```

### Go

```go
// Server-mode profile: equivalent to DefaultServerConfig(NetworkMainnet).
// Recommended when you build the SDK per request in a multi-tenant server
// deployment. See the "Server mode" page for the full profile.
config := breez_sdk_spark.DefaultConfig(breez_sdk_spark.NetworkMainnet)
config.BackgroundTasksEnabled = false
```



**Developer note**

When this flag is `false`, related per-field options whose backing service is gated off must be in their inactive shape:

- [`real_time_sync_server_url`](#real-time-sync-server-url) must be `None`.
- [`leaf_optimization_config.auto_enabled`](#optimization-configuration) must be `false`.
- [`token_optimization_config.auto_enabled`](#optimization-configuration) must be `false`.

The SDK rejects builds that leave any of them in their active shape with an invalid-input error. `default_server_config` sets these compatible values automatically.

## LNURL Domain

The LNURL domain to be used for receiving LNURL and Lightning address payments. By default, the [Breez LNURL server](https://github.com/breez/spark-sdk/tree/main/crates/breez-sdk/lnurl) instance will be used. You may configure a different domain, or set no domain to disable receiving payments using LNURL. For more information, see [Receiving payments using LNURL-Pay](./receive_lnurl_pay.md).

## Prefer Spark over Lightning

When enabled, the SDK will settle Lightning invoice payments using the Spark protocol instead of Lightning where possible. It's not recommended to enable it because of the following trade-offs:

- **Sending:** No proof of payment (no Lightning preimage). Invoice metadata (the invoice itself, its description) is not persisted with the payment.
- **Receiving:** The SDK [embeds a Spark address](https://docs.spark.money/api-reference/wallet/create-lightning-invoice) in the invoice's fallback field. If the payer uses this Spark address, the received payment cannot be linked back to the invoice.

## External input parsing

The SDK's parsing module can be extended by providing external parsers that are used when input is not recognized. Some [default external parsers](./parse.md#default-external-parsers) are provided but can be disabled. You can add new external parsers as described in [Configuring external parsers](./parse.md#configuring-external-parsers).

## Real-time sync server URL

The SDK synchronizes user data across different SDK instances using a [real-time synchronization server](https://github.com/breez/data-sync). By default, a Breez instance will be used, but you may configure a different instance by providing its URL, or disable it entirely by providing no URL.

## Private mode enabled by default

Configures whether the Spark private mode should be enabled by default. By default, it is enabled. When enabled, the Spark private mode will be enabled on the first initialization of the SDK. If disabled, no changes will be made to the Spark private mode.

### Rust

```rust
// Disable Spark private mode by default
let mut config = default_config(Network::Mainnet);
config.private_enabled_default = false;
```

### Swift

```swift
// Disable Spark private mode by default
var config = defaultConfig(network: Network.mainnet)
config.privateEnabledDefault = false
```

### Kotlin

```kotlin
// Disable Spark private mode by default
val config = defaultConfig(Network.MAINNET)
config.privateEnabledDefault = false
```

### C#

```csharp
// Disable Spark private mode by default
var config = BreezSdkSparkMethods.DefaultConfig(Network.Mainnet) with
{
    privateEnabledDefault = false
};
```

### Javascript (Wasm)

```typescript
// Disable Spark private mode by default
const config = defaultConfig('mainnet')
config.privateEnabledDefault = false
```

### React Native

```typescript
// Disable Spark private mode by default
const config = defaultConfig(Network.Mainnet)
config.privateEnabledDefault = false
```

### Flutter

```dart
// Disable Spark private mode by default
var config = defaultConfig(network: Network.mainnet)
    .copyWith(privateEnabledDefault: false);
```

### Python

```python
# Disable Spark private mode by default
config = default_config(network=Network.MAINNET)
config.private_enabled_default = False
```

### Go

```go
// Disable Spark private mode by default
config := breez_sdk_spark.DefaultConfig(breez_sdk_spark.NetworkMainnet)
config.PrivateEnabledDefault = false
```



**Developer note**

This configuration option is only relevant when the SDK is initialized for the first time. To update the user settings after that, or to explicitly disable the Spark private mode, see the [User settings](./user_settings.md) page.

## Optimization configuration

The SDK can automatically optimize both the Spark leaf set and a token's
output set. Leaf optimization and token-output optimization are configured
independently.

### Leaf optimization

Leaf optimization maximizes unilateral exit efficiency or increases payment speed. Fewer, larger leaves allow more funds to be exited unilaterally, while having more leaves enables payments without requiring swaps, improving payment speed.

- **Automatic optimization enabled**: whether leaf optimization runs automatically when a payment is sent or received. Enabled by default.
- **Multiplicity**: the desired multiplicity for the leaf set. Default value is 1. Setting it to 0 fully optimizes for unilateral exit efficiency. Setting it to a value greater than 0 also optimizes for payment speed, with higher values prioritizing payment speed more aggressively at the cost of higher unilateral exit fees. Values above 5 are intended for high-throughput server environments that require maximum TPS and are not recommended for end-user wallets.

See [Custom leaf optimization](./optimize.md) for more information and recommendations on how to configure leaf optimization.

### Token-output optimization

Token-output optimization automatically consolidates a token's available outputs to keep the output set small while preserving enough distinct outputs for concurrent sends.

- **Automatic optimization enabled**: whether token-output consolidation runs automatically. Enabled by default.
- **Target output count**: the number of token outputs to produce when consolidation fires. Instead of collapsing a token's outputs into a single output (which would serialize subsequent sends), the SDK splits the consolidated balance across this many outputs of roughly equal value. Higher values preserve concurrency for parallel sends at the cost of a slightly larger output set. Must be at least 1 and strictly less than the minimum outputs threshold. Default value is 5.
- **Minimum outputs threshold**: the output count that triggers per-token auto-consolidation. Consolidation runs for a token once its available output count exceeds this value. Must be greater than 1. Default value is 50.

#### Rust

```rust
let mut config = default_config(Network::Mainnet);
config.leaf_optimization_config = LeafOptimizationConfig {
    auto_enabled: true,
    multiplicity: 1,
};
config.token_optimization_config = TokenOptimizationConfig {
    auto_enabled: true,
    target_output_count: 5,
    min_outputs_threshold: 50,
};
```

#### Swift

```swift
var config = defaultConfig(network: Network.mainnet)
config.leafOptimizationConfig = LeafOptimizationConfig(autoEnabled: true, multiplicity: 1)
config.tokenOptimizationConfig = TokenOptimizationConfig(
    autoEnabled: true,
    targetOutputCount: 5,
    minOutputsThreshold: 50
)
```

#### Kotlin

```kotlin
val config = defaultConfig(Network.MAINNET)
config.leafOptimizationConfig = LeafOptimizationConfig(
    autoEnabled = true,
    multiplicity = 1u,
)
config.tokenOptimizationConfig = TokenOptimizationConfig(
    autoEnabled = true,
    targetOutputCount = 5u,
    minOutputsThreshold = 50u,
)
```

#### C#

```csharp
var config = BreezSdkSparkMethods.DefaultConfig(Network.Mainnet) with
{
    leafOptimizationConfig = new LeafOptimizationConfig(
        autoEnabled: true,
        multiplicity: 1
    ),
    tokenOptimizationConfig = new TokenOptimizationConfig(
        autoEnabled: true,
        targetOutputCount: 5,
        minOutputsThreshold: 50
    )
};
```

#### Javascript (Wasm)

```typescript
const config = defaultConfig('mainnet')
config.leafOptimizationConfig = { autoEnabled: true, multiplicity: 1 }
config.tokenOptimizationConfig = {
  autoEnabled: true,
  targetOutputCount: 5,
  minOutputsThreshold: 50
}
```

#### React Native

```typescript
const config = defaultConfig(Network.Mainnet)
config.leafOptimizationConfig = { autoEnabled: true, multiplicity: 1 }
config.tokenOptimizationConfig = {
  autoEnabled: true,
  targetOutputCount: 5,
  minOutputsThreshold: 50
}
```

#### Flutter

```dart
var config = defaultConfig(network: Network.mainnet).copyWith(
    leafOptimizationConfig:
        LeafOptimizationConfig(autoEnabled: true, multiplicity: 1),
    tokenOptimizationConfig: TokenOptimizationConfig(
        autoEnabled: true, targetOutputCount: 5, minOutputsThreshold: 50));
```

#### Python

```python
config = default_config(network=Network.MAINNET)
config.leaf_optimization_config = LeafOptimizationConfig(
    auto_enabled=True, multiplicity=1
)
config.token_optimization_config = TokenOptimizationConfig(
    auto_enabled=True, target_output_count=5, min_outputs_threshold=50
)
```

#### Go

```go
config := breez_sdk_spark.DefaultConfig(breez_sdk_spark.NetworkMainnet)
config.LeafOptimizationConfig = breez_sdk_spark.LeafOptimizationConfig{
	AutoEnabled:  true,
	Multiplicity: 1,
}
config.TokenOptimizationConfig = breez_sdk_spark.TokenOptimizationConfig{
	AutoEnabled:         true,
	TargetOutputCount:   5,
	MinOutputsThreshold: 50,
}
```



## Spark environment configuration

The SDK comes pre-configured for the default Spark operator network. For advanced use cases such as connecting to alternative Spark deployments (e.g. dev or staging environments), you can override the operator pool, service provider, threshold, and token withdrawal settings by providing a custom Spark configuration.

The configuration requires:

- **Signing operators**: The set of Spark operators with their identifiers, gRPC addresses, and identity public keys.
- **Coordinator identifier**: Which operator acts as the coordinator.
- **Threshold**: The FROST signing threshold (e.g. 2-of-3).
- **SSP configuration**: The Service Provider's base URL, identity public key, and optionally a custom GraphQL schema endpoint path.
- **Token withdrawal settings**: Expected bond amount and relative block locktime for token withdrawals.

### Rust

```rust
let mut config = default_config(Network::Mainnet);

// Connect to a custom Spark environment
config.spark_config = Some(SparkConfig {
    coordinator_identifier: "0000000000000000000000000000000000000000000000000000000000000001"
        .to_string(),
    threshold: 2,
    signing_operators: vec![
        SparkSigningOperator {
            id: 0,
            identifier: "0000000000000000000000000000000000000000000000000000000000000001"
                .to_string(),
            address: "https://0.spark.example.com".to_string(),
            identity_public_key:
                "03acd9a5a88db102730ff83dee69d69088cc4c9d93bbee893e90fd5051b7da9651".to_string(),
            ca_cert_pem: None,
        },
        SparkSigningOperator {
            id: 1,
            identifier: "0000000000000000000000000000000000000000000000000000000000000002"
                .to_string(),
            address: "https://1.spark.example.com".to_string(),
            identity_public_key:
                "02d2d103cacb1d6355efeab27637c74484e2a7459e49110c3fe885210369782e23".to_string(),
            ca_cert_pem: None,
        },
        SparkSigningOperator {
            id: 2,
            identifier: "0000000000000000000000000000000000000000000000000000000000000003"
                .to_string(),
            address: "https://2.spark.example.com".to_string(),
            identity_public_key:
                "0350f07ffc21bfd59d31e0a7a600e2995273938444447cb9bc4c75b8a895dbb853".to_string(),
            ca_cert_pem: None,
        },
    ],
    ssp_config: SparkSspConfig {
        base_url: "https://api.example.com".to_string(),
        identity_public_key:
            "02e0b8d42c5d3b5fe4c5beb6ea796ab3bc8aaf28a3d3195407482c67e0b58228a5".to_string(),
        schema_endpoint: Some("graphql/spark/rc".to_string()),
    },
    expected_withdraw_bond_sats: 10_000,
    expected_withdraw_relative_block_locktime: 1_000,
    max_token_transaction_inputs: None,
});
```

### Kotlin

```kotlin
val config = defaultConfig(Network.MAINNET)

// Connect to a custom Spark environment
config.sparkConfig = SparkConfig(
    coordinatorIdentifier = "0000000000000000000000000000000000000000000000000000000000000001",
    threshold = 2u,
    signingOperators = listOf(
        SparkSigningOperator(
            id = 0u,
            identifier = "0000000000000000000000000000000000000000000000000000000000000001",
            address = "https://0.spark.example.com",
            identityPublicKey = "03acd9a5a88db102730ff83dee69d69088cc4c9d93bbee893e90fd5051b7da9651",
            caCertPem = null
        ),
        SparkSigningOperator(
            id = 1u,
            identifier = "0000000000000000000000000000000000000000000000000000000000000002",
            address = "https://1.spark.example.com",
            identityPublicKey = "02d2d103cacb1d6355efeab27637c74484e2a7459e49110c3fe885210369782e23",
            caCertPem = null
        ),
        SparkSigningOperator(
            id = 2u,
            identifier = "0000000000000000000000000000000000000000000000000000000000000003",
            address = "https://2.spark.example.com",
            identityPublicKey = "0350f07ffc21bfd59d31e0a7a600e2995273938444447cb9bc4c75b8a895dbb853",
            caCertPem = null
        )
    ),
    sspConfig = SparkSspConfig(
        baseUrl = "https://api.example.com",
        identityPublicKey = "02e0b8d42c5d3b5fe4c5beb6ea796ab3bc8aaf28a3d3195407482c67e0b58228a5",
        schemaEndpoint = "graphql/spark/rc"
    ),
    expectedWithdrawBondSats = 10_000u,
    expectedWithdrawRelativeBlockLocktime = 1_000u,
    maxTokenTransactionInputs = null
)
```

### C#

```csharp
var config = BreezSdkSparkMethods.DefaultConfig(Network.Mainnet) with
{
    // Connect to a custom Spark environment
    sparkConfig = new SparkConfig(
        coordinatorIdentifier: "0000000000000000000000000000000000000000000000000000000000000001",
        threshold: 2,
        signingOperators: new[]
        {
            new SparkSigningOperator(
                id: 0,
                identifier: "0000000000000000000000000000000000000000000000000000000000000001",
                address: "https://0.spark.example.com",
                identityPublicKey:
                    "03acd9a5a88db102730ff83dee69d69088cc4c9d93bbee893e90fd5051b7da9651",
                caCertPem: null
            ),
            new SparkSigningOperator(
                id: 1,
                identifier: "0000000000000000000000000000000000000000000000000000000000000002",
                address: "https://1.spark.example.com",
                identityPublicKey:
                    "02d2d103cacb1d6355efeab27637c74484e2a7459e49110c3fe885210369782e23",
                caCertPem: null
            ),
            new SparkSigningOperator(
                id: 2,
                identifier: "0000000000000000000000000000000000000000000000000000000000000003",
                address: "https://2.spark.example.com",
                identityPublicKey:
                    "0350f07ffc21bfd59d31e0a7a600e2995273938444447cb9bc4c75b8a895dbb853",
                caCertPem: null
            )
        },
        sspConfig: new SparkSspConfig(
            baseUrl: "https://api.example.com",
            identityPublicKey: "02e0b8d42c5d3b5fe4c5beb6ea796ab3bc8aaf28a3d3195407482c67e0b58228a5",
            schemaEndpoint: "graphql/spark/rc"
        ),
        expectedWithdrawBondSats: 10000,
        expectedWithdrawRelativeBlockLocktime: 1000,
        maxTokenTransactionInputs: null
    )
};
```

### Javascript (Wasm)

```typescript
const config = defaultConfig('mainnet')

// Connect to a custom Spark environment
config.sparkConfig = {
  coordinatorIdentifier: '0000000000000000000000000000000000000000000000000000000000000001',
  threshold: 2,
  signingOperators: [
    {
      id: 0,
      identifier: '0000000000000000000000000000000000000000000000000000000000000001',
      address: 'https://0.spark.example.com',
      identityPublicKey: '03acd9a5a88db102730ff83dee69d69088cc4c9d93bbee893e90fd5051b7da9651'
    },
    {
      id: 1,
      identifier: '0000000000000000000000000000000000000000000000000000000000000002',
      address: 'https://1.spark.example.com',
      identityPublicKey: '02d2d103cacb1d6355efeab27637c74484e2a7459e49110c3fe885210369782e23'
    },
    {
      id: 2,
      identifier: '0000000000000000000000000000000000000000000000000000000000000003',
      address: 'https://2.spark.example.com',
      identityPublicKey: '0350f07ffc21bfd59d31e0a7a600e2995273938444447cb9bc4c75b8a895dbb853'
    }
  ],
  sspConfig: {
    baseUrl: 'https://api.example.com',
    identityPublicKey: '02e0b8d42c5d3b5fe4c5beb6ea796ab3bc8aaf28a3d3195407482c67e0b58228a5',
    schemaEndpoint: 'graphql/spark/rc'
  },
  expectedWithdrawBondSats: 10_000,
  expectedWithdrawRelativeBlockLocktime: 1_000
}
```

### React Native

```typescript
const config = defaultConfig(Network.Mainnet)

// Connect to a custom Spark environment
config.sparkConfig = {
  coordinatorIdentifier: '0000000000000000000000000000000000000000000000000000000000000001',
  threshold: 2,
  signingOperators: [
    {
      id: 0,
      identifier: '0000000000000000000000000000000000000000000000000000000000000001',
      address: 'https://0.spark.example.com',
      identityPublicKey: '03acd9a5a88db102730ff83dee69d69088cc4c9d93bbee893e90fd5051b7da9651',
      caCertPem: undefined
    },
    {
      id: 1,
      identifier: '0000000000000000000000000000000000000000000000000000000000000002',
      address: 'https://1.spark.example.com',
      identityPublicKey: '02d2d103cacb1d6355efeab27637c74484e2a7459e49110c3fe885210369782e23',
      caCertPem: undefined
    },
    {
      id: 2,
      identifier: '0000000000000000000000000000000000000000000000000000000000000003',
      address: 'https://2.spark.example.com',
      identityPublicKey: '0350f07ffc21bfd59d31e0a7a600e2995273938444447cb9bc4c75b8a895dbb853',
      caCertPem: undefined
    }
  ],
  sspConfig: {
    baseUrl: 'https://api.example.com',
    identityPublicKey: '02e0b8d42c5d3b5fe4c5beb6ea796ab3bc8aaf28a3d3195407482c67e0b58228a5',
    schemaEndpoint: 'graphql/spark/rc'
  },
  expectedWithdrawBondSats: BigInt(10_000),
  expectedWithdrawRelativeBlockLocktime: BigInt(1_000),
  maxTokenTransactionInputs: undefined
}
```

### Flutter

```dart
var config = defaultConfig(network: Network.mainnet).copyWith(
    // Connect to a custom Spark environment
    sparkConfig: SparkConfig(
        coordinatorIdentifier:
            '0000000000000000000000000000000000000000000000000000000000000001',
        threshold: 2,
        signingOperators: [
          SparkSigningOperator(
              id: 0,
              identifier:
                  '0000000000000000000000000000000000000000000000000000000000000001',
              address: 'https://0.spark.example.com',
              identityPublicKey:
                  '03acd9a5a88db102730ff83dee69d69088cc4c9d93bbee893e90fd5051b7da9651',
              caCertPem: null),
          SparkSigningOperator(
              id: 1,
              identifier:
                  '0000000000000000000000000000000000000000000000000000000000000002',
              address: 'https://1.spark.example.com',
              identityPublicKey:
                  '02d2d103cacb1d6355efeab27637c74484e2a7459e49110c3fe885210369782e23',
              caCertPem: null),
          SparkSigningOperator(
              id: 2,
              identifier:
                  '0000000000000000000000000000000000000000000000000000000000000003',
              address: 'https://2.spark.example.com',
              identityPublicKey:
                  '0350f07ffc21bfd59d31e0a7a600e2995273938444447cb9bc4c75b8a895dbb853',
              caCertPem: null),
        ],
        sspConfig: SparkSspConfig(
            baseUrl: 'https://api.example.com',
            identityPublicKey:
                '02e0b8d42c5d3b5fe4c5beb6ea796ab3bc8aaf28a3d3195407482c67e0b58228a5',
            schemaEndpoint: 'graphql/spark/rc'),
        expectedWithdrawBondSats: BigInt.from(10000),
        expectedWithdrawRelativeBlockLocktime: BigInt.from(1000)));
```

### Python

```python
config = default_config(network=Network.MAINNET)

# Connect to a custom Spark environment
config.spark_config = SparkConfig(
    coordinator_identifier="0000000000000000000000000000000000000000000000000000000000000001",
    threshold=2,
    signing_operators=[
        SparkSigningOperator(
            id=0,
            identifier="0000000000000000000000000000000000000000000000000000000000000001",
            address="https://0.spark.example.com",
            identity_public_key=(
                "03acd9a5a88db102730ff83dee69d69088cc4c9d93bbee893e90fd5051b7da9651"
            ),
            ca_cert_pem=None,
        ),
        SparkSigningOperator(
            id=1,
            identifier="0000000000000000000000000000000000000000000000000000000000000002",
            address="https://1.spark.example.com",
            identity_public_key=(
                "02d2d103cacb1d6355efeab27637c74484e2a7459e49110c3fe885210369782e23"
            ),
            ca_cert_pem=None,
        ),
        SparkSigningOperator(
            id=2,
            identifier="0000000000000000000000000000000000000000000000000000000000000003",
            address="https://2.spark.example.com",
            identity_public_key=(
                "0350f07ffc21bfd59d31e0a7a600e2995273938444447cb9bc4c75b8a895dbb853"
            ),
            ca_cert_pem=None,
        ),
    ],
    ssp_config=SparkSspConfig(
        base_url="https://api.example.com",
        identity_public_key=(
            "02e0b8d42c5d3b5fe4c5beb6ea796ab3bc8aaf28a3d3195407482c67e0b58228a5"
        ),
        schema_endpoint="graphql/spark/rc",
    ),
    expected_withdraw_bond_sats=10_000,
    expected_withdraw_relative_block_locktime=1_000,
    max_token_transaction_inputs=None,
)
```

### Go

```go
config := breez_sdk_spark.DefaultConfig(breez_sdk_spark.NetworkMainnet)

// Connect to a custom Spark environment
schemaEndpoint := "graphql/spark/rc"
sparkConfig := breez_sdk_spark.SparkConfig{
	CoordinatorIdentifier: "0000000000000000000000000000000000000000000000000000000000000001",
	Threshold:             2,
	SigningOperators: []breez_sdk_spark.SparkSigningOperator{
		{
			Id:                0,
			Identifier:        "0000000000000000000000000000000000000000000000000000000000000001",
			Address:           "https://0.spark.example.com",
			IdentityPublicKey: "03acd9a5a88db102730ff83dee69d69088cc4c9d93bbee893e90fd5051b7da9651",
			CaCertPem:         nil,
		},
		{
			Id:                1,
			Identifier:        "0000000000000000000000000000000000000000000000000000000000000002",
			Address:           "https://1.spark.example.com",
			IdentityPublicKey: "02d2d103cacb1d6355efeab27637c74484e2a7459e49110c3fe885210369782e23",
			CaCertPem:         nil,
		},
		{
			Id:                2,
			Identifier:        "0000000000000000000000000000000000000000000000000000000000000003",
			Address:           "https://2.spark.example.com",
			IdentityPublicKey: "0350f07ffc21bfd59d31e0a7a600e2995273938444447cb9bc4c75b8a895dbb853",
			CaCertPem:         nil,
		},
	},
	SspConfig: breez_sdk_spark.SparkSspConfig{
		BaseUrl:           "https://api.example.com",
		IdentityPublicKey: "02e0b8d42c5d3b5fe4c5beb6ea796ab3bc8aaf28a3d3195407482c67e0b58228a5",
		SchemaEndpoint:    &schemaEndpoint,
	},
	ExpectedWithdrawBondSats:              10_000,
	ExpectedWithdrawRelativeBlockLocktime: 1_000,
}
config.SparkConfig = &sparkConfig
```



**Developer note**

This is an advanced configuration option intended for Spark operators and developers working with custom Spark deployments. Most integrators should use the default configuration.

## Maximum concurrent claims

Controls how many pending Spark transfers can be claimed in parallel. The default value of 4 provides a good balance between throughput and resource usage for most applications.

For server environments or applications that receive a high volume of incoming payments, increasing this value can further improve throughput by processing more claims concurrently.

**Default**: 4

**Recommendation**: The default value works well for most applications. Server applications handling many simultaneous incoming payments may benefit from higher values (e.g., 8-16), depending on their infrastructure capacity. End-user wallets with limited resources may reduce this to 1-2.

## Stable balance configuration

API docs: https://breez.github.io/spark-sdk/breez_sdk_spark/struct.StableBalanceConfig.html

The SDK can convert Bitcoin to a stable token on receive and vice versa on send, protecting against price volatility. Configure the available tokens, default behavior, conversion threshold, and slippage tolerance. See the [Stable balance](./stable_balance.md) guide for full details.

### Rust

```rust
let mut config = default_config(Network::Mainnet);

// Enable stable balance with USDB conversion
config.stable_balance_config = Some(StableBalanceConfig {
    tokens: vec![StableBalanceToken {
        label: "USDB".to_string(),
        token_identifier: "btkn1xgrvjwey5ngcagvap2dzzvsy4uk8ua9x69k82dwvt5e7ef9drm9qztux87"
            .to_string(),
    }],
    default_active_label: Some("USDB".to_string()),
    threshold_sats: None,
    max_slippage_bps: None,
});
```

### Swift

```swift
var config = defaultConfig(network: Network.mainnet)

// Enable stable balance with USDB conversion
config.stableBalanceConfig = StableBalanceConfig(
    tokens: [StableBalanceToken(
        label: "USDB",
        tokenIdentifier: "btkn1xgrvjwey5ngcagvap2dzzvsy4uk8ua9x69k82dwvt5e7ef9drm9qztux87"
    )],
    defaultActiveLabel: "USDB"
)
```

### Kotlin

```kotlin
val config = defaultConfig(Network.MAINNET)

// Enable stable balance with USDB conversion
config.stableBalanceConfig = StableBalanceConfig(
    tokens = listOf(StableBalanceToken(
        label = "USDB",
        tokenIdentifier = "btkn1xgrvjwey5ngcagvap2dzzvsy4uk8ua9x69k82dwvt5e7ef9drm9qztux87",
    )),
    defaultActiveLabel = "USDB",
)
```

### C#

```csharp
var config = BreezSdkSparkMethods.DefaultConfig(Network.Mainnet) with
{
    // Enable stable balance with USDB conversion
    stableBalanceConfig = new StableBalanceConfig(
        tokens: new StableBalanceToken[] {
            new StableBalanceToken(
                label: "USDB",
                tokenIdentifier: "btkn1xgrvjwey5ngcagvap2dzzvsy4uk8ua9x69k82dwvt5e7ef9drm9qztux87"
            )
        },
        defaultActiveLabel: "USDB"
    )
};
```

### Javascript (Wasm)

```typescript
const config = defaultConfig('mainnet')

// Enable stable balance with USDB conversion
config.stableBalanceConfig = {
  tokens: [{
    label: 'USDB',
    tokenIdentifier: 'btkn1xgrvjwey5ngcagvap2dzzvsy4uk8ua9x69k82dwvt5e7ef9drm9qztux87'
  }],
  defaultActiveLabel: 'USDB'
}
```

### React Native

```typescript
const config = defaultConfig(Network.Mainnet)

// Enable stable balance with USDB conversion
config.stableBalanceConfig = {
  tokens: [{
    label: 'USDB',
    tokenIdentifier: 'btkn1xgrvjwey5ngcagvap2dzzvsy4uk8ua9x69k82dwvt5e7ef9drm9qztux87'
  }],
  defaultActiveLabel: 'USDB',
  thresholdSats: undefined,
  maxSlippageBps: undefined
}
```

### Flutter

```dart
var config = defaultConfig(network: Network.mainnet).copyWith(
    // Enable stable balance with USDB conversion
    stableBalanceConfig: StableBalanceConfig(
        tokens: [StableBalanceToken(
          label: "USDB",
          tokenIdentifier: "btkn1xgrvjwey5ngcagvap2dzzvsy4uk8ua9x69k82dwvt5e7ef9drm9qztux87",
        )],
        defaultActiveLabel: "USDB",
        ));
```

### Python

```python
config = default_config(network=Network.MAINNET)

# Enable stable balance with USDB conversion
config.stable_balance_config = StableBalanceConfig(
    tokens=[StableBalanceToken(
        label="USDB",
        token_identifier="btkn1xgrvjwey5ngcagvap2dzzvsy4uk8ua9x69k82dwvt5e7ef9drm9qztux87",
    )],
    default_active_label="USDB",
)
```

### Go

```go
config := breez_sdk_spark.DefaultConfig(breez_sdk_spark.NetworkMainnet)

// Enable stable balance with USDB conversion
defaultActiveLabel := "USDB"
stableBalanceConfig := breez_sdk_spark.StableBalanceConfig{
	Tokens: []breez_sdk_spark.StableBalanceToken{
		{
			Label:           "USDB",
			TokenIdentifier: "btkn1xgrvjwey5ngcagvap2dzzvsy4uk8ua9x69k82dwvt5e7ef9drm9qztux87",
		},
	},
	DefaultActiveLabel: &defaultActiveLabel,
}
config.StableBalanceConfig = &stableBalanceConfig
```



## Send USDC/USDT

API docs: https://breez.github.io/spark-sdk/breez_sdk_spark/struct.CrossChainConfig.html

USDC/USDT sends require explicit opt-in: `default_config` leaves `cross_chain_config` unset. Set it to a default `CrossChainConfig` to enable the feature, or to your own to override the slippage default. The SDK only returns routes whose destination is USDC or USDT on a supported chain.

Constraints:

- **Mainnet only**: `validate` rejects a set `cross_chain_config` on any network other than mainnet.
- **Background tasks required**: both providers run background monitors that reconcile delivery status onto the local payment row, so `cross_chain_config` is incompatible with `background_tasks_enabled` disabled. `default_server_config` leaves the field unset for this reason.

### Rust

```rust
let mut config = default_config(Network::Mainnet);
config.api_key = Some("<breez api key>".to_string());

// Set to enable cross-chain payments. Slippage override is optional (10 to 500 bps).
config.cross_chain_config = Some(CrossChainConfig {
    default_slippage_bps: Some(50),
    default_target_overpay_bps: None,
});
```

### Swift

```swift
var config = defaultConfig(network: Network.mainnet)
config.apiKey = "<breez api key>"

// Set to enable cross-chain payments. Slippage override is optional (10 to 500 bps).
config.crossChainConfig = CrossChainConfig(
    defaultSlippageBps: 50,
    defaultTargetOverpayBps: nil
)
```

### Kotlin

```kotlin
val config = defaultConfig(Network.MAINNET)
config.apiKey = "<breez api key>"

// Set to enable cross-chain payments. Slippage override is optional (10 to 500 bps).
config.crossChainConfig = CrossChainConfig(
    defaultSlippageBps = 50u,
    defaultTargetOverpayBps = null,
)
```

### C#

```csharp
// Set to enable cross-chain payments. Slippage override is optional (10 to 500 bps).
var config = BreezSdkSparkMethods.DefaultConfig(Network.Mainnet) with
{
    apiKey = "<breez api key>",
    crossChainConfig = new CrossChainConfig(
        defaultSlippageBps: 50,
        defaultTargetOverpayBps: null)
};
```

### Javascript (Wasm)

```typescript
const config = defaultConfig('mainnet')
config.apiKey = '<breez api key>'

// Set to enable cross-chain payments. Slippage override is optional (10 to 500 bps).
config.crossChainConfig = { defaultSlippageBps: 50 }
```

### React Native

```typescript
const config = defaultConfig(Network.Mainnet)
config.apiKey = '<breez api key>'

// Set to enable cross-chain payments. Slippage override is optional (10 to 500 bps).
config.crossChainConfig = {
  defaultSlippageBps: 50,
  defaultTargetOverpayBps: undefined
}
```

### Flutter

```dart
// Set to enable cross-chain payments. Slippage override is optional (10 to 500 bps).
final config = defaultConfig(network: Network.mainnet).copyWith(
  apiKey: "<breez api key>",
  crossChainConfig: const CrossChainConfig(
    defaultSlippageBps: 50,
    defaultTargetOverpayBps: null,
  ),
);
```

### Python

```python
config = default_config(network=Network.MAINNET)
config.api_key = "<breez api key>"

# Set to enable cross-chain payments. Slippage override is optional (10 to 500 bps).
config.cross_chain_config = CrossChainConfig(
    default_slippage_bps=50,
    default_target_overpay_bps=None,
)
```

### Go

```go
config := breez_sdk_spark.DefaultConfig(breez_sdk_spark.NetworkMainnet)
apiKey := "<breez api key>"
config.ApiKey = &apiKey

// Set to enable cross-chain payments. Slippage override is optional (10 to 500 bps).
defaultSlippageBps := uint32(50)
config.CrossChainConfig = &breez_sdk_spark.CrossChainConfig{
	DefaultSlippageBps:       &defaultSlippageBps,
	DefaultTargetOverpayBps:  nil,
}
```



The `default_slippage_bps` field sets the per-instance slippage default applied when the per-request `max_slippage_bps` is unset. It must be in the 10 to 500 basis-point range; when `default_slippage_bps` itself is unset, the SDK falls back to a built-in default of 100 bps (1%).

See [Send USDC/USDT](./cross_chain.md) for the provider lineup, status lifecycle, retry-safety semantics, and limitations.

---

Identifier casing: `get_info` here is `getInfo` in Swift, Kotlin, JavaScript, React Native and Flutter, and `GetInfo` in Go and C#. Enum variants: `SdkEvent::Synced` is `SdkEvent.SYNCED` in Python, `SdkEvent.synced` in Swift, `SdkEventSynced` in Go, and `SdkEvent.Synced` elsewhere.
