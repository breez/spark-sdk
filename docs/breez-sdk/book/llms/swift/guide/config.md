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



## Synchronization interval

The SDK performs regular background synchronization to check for payment status updates. You can configure how often this synchronization occurs.

The synchronization process is used to detect some payment status updates that are not detected in real-time through event streams.

A shorter synchronization interval provides more responsive detection of payment updates but increases resource usage and may trigger API rate limits. The default interval balances responsiveness with resource efficiency for most use cases.

## Background tasks enabled

Master switch for all per-instance background tasks. Defaults to `true`, which is the right choice for mobile and single-instance deployments — the SDK runs its periodic sync, real-time sync client, lightning-address recovery, spark private-mode init, leaf and token-output optimizers, the spark-wallet background processor, and the flashnet conversion refunder.

Set to `false` for multi-tenant server deployments where the SDK is built per request and the host orchestrates sync, claiming, and event delivery (typically via webhooks) explicitly. No background work is started; explicit operations such as `syncWallet`, `claimDeposit`, `listUnclaimedDeposits`, `refundDeposit`, and `refundPendingConversions` continue to work and are the intended entry points in this mode.

The recommended way to opt into server mode is via `defaultServerConfig`, which returns the same `Config` as `defaultConfig` with this flag flipped off. See [Server mode](./server_mode.md) for the full profile, lifecycle pattern, and shared-infrastructure wiring. Configuring this field directly is supported if you build your `Config` another way:

```swift
// Server-mode profile: equivalent to defaultServerConfig(network: .mainnet).
// Recommended when you build the SDK per request in a multi-tenant server
// deployment. See the "Server mode" page for the full profile.
var config = defaultConfig(network: Network.mainnet)
config.backgroundTasksEnabled = false
```



**Developer note**

When this flag is `false`, related per-field options whose backing service is gated off must be in their inactive shape:

- [`realTimeSyncServerUrl`](#real-time-sync-server-url) must be `None`.
- [`leafOptimizationConfig.autoEnabled`](#optimization-configuration) must be `false`.
- [`tokenOptimizationConfig.autoEnabled`](#optimization-configuration) must be `false`.

The SDK rejects builds that leave any of them in their active shape with an invalid-input error. `defaultServerConfig` sets these compatible values automatically.

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

```swift
// Disable Spark private mode by default
var config = defaultConfig(network: Network.mainnet)
config.privateEnabledDefault = false
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

```swift
var config = defaultConfig(network: Network.mainnet)
config.leafOptimizationConfig = LeafOptimizationConfig(autoEnabled: true, multiplicity: 1)
config.tokenOptimizationConfig = TokenOptimizationConfig(
    autoEnabled: true,
    targetOutputCount: 5,
    minOutputsThreshold: 50
)
```



## Spark environment configuration

The SDK comes pre-configured for the default Spark operator network. For advanced use cases such as connecting to alternative Spark deployments (e.g. dev or staging environments), you can override the operator pool, service provider, threshold, and token withdrawal settings by providing a custom Spark configuration.

The configuration requires:

- **Signing operators**: The set of Spark operators with their identifiers, gRPC addresses, and identity public keys.
- **Coordinator identifier**: Which operator acts as the coordinator.
- **Threshold**: The FROST signing threshold (e.g. 2-of-3).
- **SSP configuration**: The Service Provider's base URL, identity public key, and optionally a custom GraphQL schema endpoint path.
- **Token withdrawal settings**: Expected bond amount and relative block locktime for token withdrawals.



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



## Send USDC/USDT

API docs: https://breez.github.io/spark-sdk/breez_sdk_spark/struct.CrossChainConfig.html

USDC/USDT sends require explicit opt-in: `defaultConfig` leaves `crossChainConfig` unset. Set it to a default `CrossChainConfig` to enable the feature, or to your own to override the slippage default. The SDK only returns routes whose destination is USDC or USDT on a supported chain.

Constraints:

- **Mainnet only**: `validate` rejects a set `crossChainConfig` on any network other than mainnet.
- **Background tasks required**: both providers run background monitors that reconcile delivery status onto the local payment row, so `crossChainConfig` is incompatible with `backgroundTasksEnabled` disabled. `defaultServerConfig` leaves the field unset for this reason.

```swift
var config = defaultConfig(network: Network.mainnet)
config.apiKey = "<breez api key>"

// Set to enable cross-chain payments. Slippage override is optional (10 to 500 bps).
config.crossChainConfig = CrossChainConfig(
    defaultSlippageBps: 50,
    defaultTargetOverpayBps: nil
)
```



The `defaultSlippageBps` field sets the per-instance slippage default applied when the per-request `maxSlippageBps` is unset. It must be in the 10 to 500 basis-point range; when `defaultSlippageBps` itself is unset, the SDK falls back to a built-in default of 100 bps (1%).

See [Send USDC/USDT](./cross_chain.md) for the provider lineup, status lifecycle, retry-safety semantics, and limitations.
