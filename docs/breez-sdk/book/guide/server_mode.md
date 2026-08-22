# Server mode

API docs: https://breez.github.io/spark-sdk/breez_sdk_spark/fn.default_server_config.html

Server mode is the SDK profile for **multi-tenant server deployments** where a single process hosts many wallets and builds an ephemeral SDK instance per request. The SDK is treated as a library: the host orchestrates sync, claiming, and event delivery (typically via webhooks) explicitly, so each per-request SDK stays cheap, predictable, and returns fresh state.

Use server mode when:

- You run the SDK behind an HTTP/gRPC service that handles many wallets in the same process.
- Each request builds the SDK, performs one operation, and disconnects.
- Background work that makes sense for a long-lived mobile client (periodic sync, real-time sync, leaf optimization, lightning-address recovery) would be wasted on a per-request lifecycle.

If you're building a mobile or desktop wallet, stay on the default ([client mode](initializing.md)) — server mode disables features your app relies on.

When the server must not be able to send payments on its own, server mode pairs with [Client signing](client_signing.md): the user reviews and signs each payment on their side, and the build and publish steps are stateless, so they fit the per-request lifecycle.

## Selecting server mode

Build the config with `default_server_config` instead of `default_config`:

### Rust

```rust
// Construct the seed using a mnemonic, entropy or passkey
let mnemonic = "<mnemonic words>".to_string();
let seed = Seed::Mnemonic {
    mnemonic,
    passphrase: None,
};

// Build a server-mode config: same as default_config(network) with
// background_tasks_enabled = false. No periodic sync, no real-time sync
// client, no leaf/token optimizer, no flashnet refunder, no lightning-
// address recovery, no spark private-mode init.
let mut config = default_server_config(Network::Mainnet);
config.api_key = Some("<breez api key>".to_string());

// Typically server-mode SDKs are built per request and share infrastructure
// (DB pool, REST chain service, SSP/Connection Manager) across instances.
// Pass the shared resources via the builder; see the "Customizing the SDK"
// page for each component.
let sdk = SdkBuilder::new(config, seed)
    .with_default_storage("./.data".to_string())
    .build()
    .await?;
```

### Swift

```swift
// Construct the seed using a mnemonic, entropy or passkey
let mnemonic = "<mnemonic words>"
let seed = Seed.mnemonic(mnemonic: mnemonic, passphrase: nil)

// Build a server-mode config: same as defaultConfig(network) with
// backgroundTasksEnabled = false. No periodic sync, no real-time sync
// client, no leaf/token optimizer, no flashnet refunder, no lightning-
// address recovery, no spark private-mode init.
var config = defaultServerConfig(network: Network.mainnet)
config.apiKey = "<breez api key>"

// Typically server-mode SDKs are built per request and share infrastructure
// (DB pool, REST chain service, SSP/Connection Manager) across instances.
// Pass the shared resources via the builder.
let builder = SdkBuilder(config: config, seed: seed)
await builder.withDefaultStorage(storageDir: "./.data")
let sdk = try await builder.build()
```

### Kotlin

```kotlin
// Construct the seed using a mnemonic, entropy or passkey
val mnemonic = "<mnemonic words>"
val seed = Seed.Mnemonic(mnemonic, null)

// Build a server-mode config: same as defaultConfig(network) with
// backgroundTasksEnabled = false. No periodic sync, no real-time sync
// client, no leaf/token optimizer, no flashnet refunder, no lightning-
// address recovery, no spark private-mode init.
val config = defaultServerConfig(Network.MAINNET)
config.apiKey = "<breez api key>"

try {
    // Typically server-mode SDKs are built per request and share
    // infrastructure (DB pool, REST chain service, SSP/Connection
    // Manager) across instances. Pass the shared resources via the
    // builder.
    val builder = SdkBuilder(config, seed)
    builder.withDefaultStorage("./.data")
    val sdk = builder.build()
} catch (e: Exception) {
    // handle error
}
```

### C#

```csharp
// Construct the seed using a mnemonic, entropy or passkey
var mnemonic = "<mnemonic words>";
var seed = new Seed.Mnemonic(mnemonic: mnemonic, passphrase: null);

// Build a server-mode config: same as DefaultConfig(network) with
// backgroundTasksEnabled = false. No periodic sync, no real-time
// sync client, no leaf/token optimizer, no flashnet refunder, no
// lightning-address recovery, no spark private-mode init.
var config = BreezSdkSparkMethods.DefaultServerConfig(Network.Mainnet) with
{
    apiKey = "<breez api key>"
};

// Typically server-mode SDKs are built per request and share
// infrastructure (DB pool, REST chain service, SSP/Connection
// Manager) across instances. Pass the shared resources via the
// builder.
var builder = new SdkBuilder(config: config, seed: seed);
await builder.WithDefaultStorage(storageDir: "./.data");
var sdk = await builder.Build();
```

### Javascript (Wasm)

```typescript
// Construct the seed using a mnemonic, entropy or passkey
const mnemonic = '<mnemonic words>'
const seed: Seed = { type: 'mnemonic', mnemonic, passphrase: undefined }

// Build a server-mode config: same as defaultConfig(network) with
// backgroundTasksEnabled = false. No periodic sync, no real-time sync
// client, no leaf/token optimizer, no flashnet refunder, no lightning-
// address recovery, no spark private-mode init.
const config = defaultServerConfig('mainnet')
config.apiKey = '<breez api key>'

// Typically server-mode SDKs are built per request and share infrastructure
// (DB pool, REST chain service, SSP/Connection Manager) across instances.
// Pass the shared resources via the builder; see the "Customizing the SDK"
// page for each component.
let builder = SdkBuilder.new(config, seed)
builder = await builder.withDefaultStorage('./.data')
const sdk = await builder.build()
```

### React Native

```typescript
// Construct the seed using a mnemonic, entropy or passkey
const mnemonic = '<mnemonics words>'
const seed = new Seed.Mnemonic({ mnemonic, passphrase: undefined })

// Build a server-mode config: same as defaultConfig(network) with
// backgroundTasksEnabled = false. No periodic sync, no real-time sync
// client, no leaf/token optimizer, no flashnet refunder, no lightning-
// address recovery, no spark private-mode init.
const config = defaultServerConfig(Network.Mainnet)
config.apiKey = '<breez api key>'

// Typically server-mode SDKs are built per request and share infrastructure
// (DB pool, REST chain service, SSP/Connection Manager) across instances.
// Pass the shared resources via the builder.
const builder = new SdkBuilder(config, seed)
await builder.withDefaultStorage(`${RNFS.DocumentDirectoryPath}/data`)
const sdk = await builder.build()
```

### Flutter

```dart
// Construct the seed using a mnemonic, entropy or passkey
String mnemonic = "<mnemonic words>";
final seed = Seed.mnemonic(mnemonic: mnemonic, passphrase: null);

// Build a server-mode config: same as defaultConfig(network) with
// backgroundTasksEnabled = false. No periodic sync, no real-time sync
// client, no leaf/token optimizer, no flashnet refunder, no lightning-
// address recovery, no spark private-mode init.
final config = defaultServerConfig(network: Network.mainnet)
    .copyWith(apiKey: "<breez api key>");

// Typically server-mode SDKs are built per request and share infrastructure
// (DB pool, REST chain service, SSP/Connection Manager) across instances.
// Pass the shared resources via the builder.
final builder = SdkBuilder(config: config, seed: seed);
builder.withDefaultStorage(storageDir: "./.data");
final sdk = await builder.build();
```

### Python

```python
# Construct the seed using a mnemonic, entropy or passkey
mnemonic = "<mnemonic words>"
seed = Seed.MNEMONIC(mnemonic=mnemonic, passphrase=None)

# Build a server-mode config: same as default_config(network) with
# background_tasks_enabled = False. No periodic sync, no real-time sync
# client, no leaf/token optimizer, no flashnet refunder, no lightning-
# address recovery, no spark private-mode init.
config = default_server_config(network=Network.MAINNET)
config.api_key = "<breez api key>"

try:
    # Typically server-mode SDKs are built per request and share
    # infrastructure (DB pool, REST chain service, SSP/Connection Manager)
    # across instances. Pass the shared resources via the builder.
    builder = SdkBuilder(config=config, seed=seed)
    await builder.with_default_storage(storage_dir="./.data")
    sdk = await builder.build()
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

// Build a server-mode config: same as DefaultConfig(network) with
// BackgroundTasksEnabled = false. No periodic sync, no real-time sync
// client, no leaf/token optimizer, no flashnet refunder, no lightning-
// address recovery, no spark private-mode init.
apiKey := "<breez api key>"
config := breez_sdk_spark.DefaultServerConfig(breez_sdk_spark.NetworkMainnet)
config.ApiKey = &apiKey

// Typically server-mode SDKs are built per request and share infrastructure
// (DB pool, REST chain service, SSP/Connection Manager) across instances.
// Pass the shared resources via the builder.
builder := breez_sdk_spark.NewSdkBuilder(config, seed)
builder.WithDefaultStorage("./.data")
sdk, err := builder.Build()
if err != nil {
	return nil, err
}
```



`default_server_config` returns the same `Config` as `default_config` with [`background_tasks_enabled`](./config.md#background-tasks-enabled) set to `false` and the fields whose background services are gated off — [`real_time_sync_server_url`](./config.md#real-time-sync-server-url), [`leaf_optimization_config.auto_enabled`](./config.md#optimization-configuration), and [`token_optimization_config.auto_enabled`](./config.md#optimization-configuration) — reset to their inactive shape. The SDK rejects builds that leave those fields in their active shape while `background_tasks_enabled` is `false`, so prefer this preset over flipping the flag by hand.

Server mode usually pairs with **shared infrastructure** across SDK instances. See [Customizing the SDK](customizing.md) and the [Shared infrastructure](#shared-infrastructure) section below for the exact wiring.

## What server mode turns off

None of the following per-instance background work is started when `background_tasks_enabled` is `false`:

- **Periodic sync loop** — the SDK does not auto-sync with the Spark network.
- **Real-time sync client** — no WebSocket subscription to the [real-time sync server](./config.md#real-time-sync-server-url).
- **Spark wallet background processor** — no operator-event subscription, leaf optimizer, or token-output optimizer.
- **Unilateral exit data collector** does not run, so the data a [unilateral exit](unilateral_exit.md) needs is not collected in the background. `sync_wallet` collects it instead; see below.
- **Lightning-address recovery** — the SDK does not refresh the registered lightning address on startup.
- **Spark private-mode init** — the [`private_enabled_default`](./config.md#private-mode-enabled-by-default) preset is **not** applied automatically on first startup; you must opt in once via `update_user_settings` (see [User settings](user_settings.md)).
- **Flashnet conversion refunder** — no periodic refund pass for failed token conversions.
- **Stable Balance** — Stable Balance is not supported in server mode because its conversion worker is a background service. Do not rely on automatic Bitcoin-to-token conversion on receive, activation/deactivation conversion, or other Stable Balance background behavior in this profile.

Explicit operations such as `sync_wallet`, `claim_deposit`, `list_unclaimed_deposits`, `refund_deposit`, and `refund_pending_conversions` continue to work and are the intended entry points in this mode.

## Driving the SDK explicitly

Because nothing runs in the background, the user is responsible for calling the operations that the SDK would otherwise schedule itself. In practice there are only three things to drive, plus one one-time setup per wallet:

### Sync

Call `sync_wallet` **only when an external event tells you the wallet state has changed**. The two common cases:

1. **A webhook fires for an incoming payment** — a Lightning receive completes, an on-chain deposit confirms, an incoming Spark transfer lands. Run `sync_wallet()` from the webhook handler so the wallet picks up the new state before downstream consumers (balance reads, payment lists, etc.) need it.
2. **You explicitly need to reconcile state** — e.g. a periodic reconciliation job for a specific wallet, or a manual admin action. This is rare in practice; the webhook path covers the steady state.

**Do not** call `sync_wallet` from user-facing request handlers (e.g. a `GET /balance` endpoint) as a precaution — it's a network round-trip to operators and is not needed if your webhooks are wired up. `get_info` reads from the local tree store directly and is the right primitive for read paths.

The `SdkEvent::Synced` event pattern documented in [Listening to events](events.md) is **not available** in server mode — the SDK has no background subscriber to emit it. Treat `sync_wallet` as the synchronous primitive instead.

`sync_wallet` is also what keeps [unilateral exit](unilateral_exit.md) possible here. The data that makes a leaf exitable without the Spark operators is normally collected by a background task, which does not exist in server mode, so the sync collects it instead: any leaf still missing a chain is attempted before the sync returns. Every sync pays a scan of the stored leaves for this, plus a round trip to the operators on the syncs that find leaves needing one. Turn it off with [`exit_chain_auto_fetch_enabled`](./config.md#unilateral-exit-data) if the wallets you run never need to exit without the operators.

### Claiming on-chain deposits

Server-mode SDKs do not run the periodic deposit detection and claim sweep that the mobile profile uses. When your webhook or chain watcher observes a relevant on-chain deposit, handle it explicitly:

- Call `sync_wallet` to run the SDK's deposit sync and automatic claim logic using your configured [`max_deposit_claim_fee`](./config.md#max-deposit-claim-fee).
- If your backend already knows the deposit outpoint and wants to drive a specific claim, call `claim_deposit` for that `txid`/`vout`.

The standard claim flow documented in [Claiming on-chain deposits](onchain_claims.md) applies.

### Stable Balance

Stable Balance is not available in server mode. The feature depends on the client runtime's background conversion worker, so server-mode SDKs will not automatically convert received Bitcoin to the active stable token and will not process Stable Balance activation/deactivation conversions in the background.

If [`stable_balance_config`](./config.md#stable-balance-configuration) is set while using server mode, SDK initialization fails with an invalid input error. Explicit token conversion flows used by payment APIs can still be used, but do not configure Stable Balance for a server-mode deployment.

### Token conversion refunds

**Only relevant if your deployment uses [token conversions](token_conversion.md).** If you don't issue or convert tokens, skip this section.

The flashnet conversion refunder doesn't run in the background in server mode. If you do use tokens, your host needs to drive `refund_pending_conversions` per affected wallet so failed conversions get refunded. A practical pattern is to track which wallets have pending conversions (e.g. by recording them when a conversion fails) and to run the refund pass for just those wallets on a cadence you control — not to spin up an SDK per wallet every minute regardless.

### One-time setup: Spark private mode

The client-mode SDK applies [`private_enabled_default`](./config.md#private-mode-enabled-by-default) on first startup. Server-mode SDKs do not — each per-request SDK would otherwise pay a redundant storage read to check the flag. At provisioning time (when a new wallet is first registered), call `update_user_settings` with `spark_private_mode_enabled` set to `true`. See [User settings](user_settings.md).

## Event delivery via webhooks

Without the background processor, the SDK doesn't emit `PaymentSucceeded` / `PaymentPending` / `ClaimedDeposits` events from operator activity. Deliver those signals through webhooks at your own infrastructure instead:

- [Managing webhooks](webhooks.md) describes the supported event types and registration flow.
- [Lightning Address payment notifications](lnurl_webhooks.md) covers the LNURL server's webhook for incoming LNURL payments.

A typical pipeline: webhook arrives → webhook handler builds a per-request SDK, calls `sync_wallet` or the relevant explicit operation (e.g. `claim_deposit`), disconnects.

## Lifecycle pattern

There are three distinct shapes for a server-mode interaction, depending on what triggered it.

### User-facing request handlers

Generate an invoice, send a payment, list history, etc. **Do not call `sync_wallet` here** — operations that read from local storage (`get_info`, `list_payments`, etc.) do not need a defensive sync, and a network round-trip to operators on every request adds latency without changing the answer.

```text
request in
  ↓
build SDK (default_server_config + shared infra)
  ↓
do work (receive_payment / send_payment / list_payments / …)
  ↓
disconnect()
  ↓
response out
```

#### Rust

```rust
// User-facing request handler: do not call sync_wallet here. Operations
// that read from local storage (get_info, list_payments, etc.) do not
// need a defensive sync. Call sync_wallet only from webhook handlers or
// reconciliation jobs that need to observe an external state change.
let response = sdk
    .receive_payment(ReceivePaymentRequest {
        payment_method: ReceivePaymentMethod::Bolt11Invoice {
            description: "<invoice description>".to_string(),
            amount_sats: Some(5_000),
            expiry_secs: Some(3600),
            payment_hash: None,
            receiver_identity_public_key: None,
        },
    })
    .await?;

// Always disconnect at the end of the request lifecycle to flush
// outstanding storage writes. See [Disconnecting](initializing.md).
sdk.disconnect().await?;
```

#### Swift

```swift
// User-facing request handler: do not call syncWallet here. Operations
// that read from local storage (getInfo, listPayments, etc.) do not need
// a defensive sync. Call syncWallet only from webhook handlers or
// reconciliation jobs that need to observe an external state change.
let response = try await sdk.receivePayment(
    request: ReceivePaymentRequest(
        paymentMethod: ReceivePaymentMethod.bolt11Invoice(
            description: "<invoice description>",
            amountSats: 5_000,
            expirySecs: 3600,
            paymentHash: nil,
            receiverIdentityPublicKey: nil
        )
    ))

// Always disconnect at the end of the request lifecycle to flush
// outstanding storage writes.
try await sdk.disconnect()
```

#### Kotlin

```kotlin
// User-facing request handler: do not call syncWallet here.
// Operations that read from local storage (getInfo, listPayments,
// etc.) do not need a defensive sync. Call syncWallet only from
// webhook handlers or reconciliation jobs that need to observe an
// external state change.
val response = sdk.receivePayment(
    ReceivePaymentRequest(
        ReceivePaymentMethod.Bolt11Invoice(
            "<invoice description>",
            5_000.toULong(),
            3600.toUInt(),
            null,
            null,
        )
    )
)

// Always disconnect at the end of the request lifecycle to flush
// outstanding storage writes.
sdk.disconnect()
```

#### C#

```csharp
// User-facing request handler: do not call SyncWallet here.
// Operations that read from local storage (GetInfo, ListPayments,
// etc.) do not need a defensive sync. Call SyncWallet only from
// webhook handlers or reconciliation jobs that need to observe
// an external state change.
var paymentMethod = new ReceivePaymentMethod.Bolt11Invoice(
    description: "<invoice description>",
    amountSats: 5_000UL,
    expirySecs: 3600U,
    paymentHash: null,
    receiverIdentityPublicKey: null
);
var response = await sdk.ReceivePayment(
    request: new ReceivePaymentRequest(paymentMethod: paymentMethod)
);

// Always disconnect at the end of the request lifecycle to flush
// outstanding storage writes.
await sdk.Disconnect();
```

#### Javascript (Wasm)

```typescript
// User-facing request handler: do not call syncWallet here. Operations
// that read from local storage (getInfo, listPayments, etc.) do not need
// a defensive sync. Call syncWallet only from webhook handlers or
// reconciliation jobs that need to observe an external state change.
const response = await sdk.receivePayment({
  paymentMethod: {
    type: 'bolt11Invoice',
    description: '<invoice description>',
    amountSats: 5_000,
    expirySecs: 3600,
    paymentHash: undefined,
    receiverIdentityPublicKey: undefined
  }
})

// Always disconnect at the end of the request lifecycle to flush
// outstanding storage writes.
await sdk.disconnect()
```

#### React Native

```typescript
// User-facing request handler: do not call syncWallet here. Operations
// that read from local storage (getInfo, listPayments, etc.) do not need
// a defensive sync. Call syncWallet only from webhook handlers or
// reconciliation jobs that need to observe an external state change.
const response = await sdk.receivePayment({
  paymentMethod: new ReceivePaymentMethod.Bolt11Invoice({
    description: '<invoice description>',
    amountSats: BigInt(5_000),
    expirySecs: 3600,
    paymentHash: undefined,
    receiverIdentityPublicKey: undefined
  })
})

// Always disconnect at the end of the request lifecycle to flush
// outstanding storage writes.
await sdk.disconnect()
```

#### Flutter

```dart
// User-facing request handler: do not call syncWallet here. Operations
// that read from local storage (getInfo, listPayments, etc.) do not need
// a defensive sync. Call syncWallet only from webhook handlers or
// reconciliation jobs that need to observe an external state change.
final response = await sdk.receivePayment(
  request: ReceivePaymentRequest(
    paymentMethod: ReceivePaymentMethod.bolt11Invoice(
      description: "<invoice description>",
      amountSats: BigInt.from(5000),
      expirySecs: 3600,
      paymentHash: null,
    ),
  ),
);

// Always disconnect at the end of the request lifecycle to flush
// outstanding storage writes.
await sdk.disconnect();
```

#### Python

```python
# User-facing request handler: do not call sync_wallet here. Operations
# that read from local storage (get_info, list_payments, etc.) do not
# need a defensive sync. Call sync_wallet only from webhook handlers or
# reconciliation jobs that need to observe an external state change.
payment_method = ReceivePaymentMethod.BOLT11_INVOICE(
    description="<invoice description>",
    amount_sats=5_000,
    expiry_secs=3600,
    payment_hash=None,
    receiver_identity_public_key=None,
)
response = await sdk.receive_payment(
    request=ReceivePaymentRequest(payment_method=payment_method)
)

# Always disconnect at the end of the request lifecycle to flush
# outstanding storage writes.
await sdk.disconnect()
```

#### Go

```go
// User-facing request handler: do not call SyncWallet here. Operations
// that read from local storage (GetInfo, ListPayments, etc.) do not need
// a defensive sync. Call SyncWallet only from webhook handlers or
// reconciliation jobs that need to observe an external state change.
amountSats := uint64(5_000)
expirySecs := uint32(3600)
response, err := sdk.ReceivePayment(breez_sdk_spark.ReceivePaymentRequest{
	PaymentMethod: breez_sdk_spark.ReceivePaymentMethodBolt11Invoice{
		Description: "<invoice description>",
		AmountSats:  &amountSats,
		ExpirySecs:  &expirySecs,
		PaymentHash: nil,
	},
})
if err != nil {
	return "", err
}

// Always disconnect at the end of the request lifecycle to flush
// outstanding storage writes.
if err := sdk.Disconnect(); err != nil {
	return "", err
}
```



### Webhook handlers and reconciliation jobs

Anything driven by an external signal that the wallet state changed. The exact operation depends on the trigger — they're not chained together in the same handler.

- **Incoming Lightning / Spark transfer webhook** — call `sync_wallet` so downstream reads see the new payment:

```text
webhook in  →  build SDK  →  sync_wallet()  →  disconnect()
```

- **On-chain deposit webhook** (or chain watcher) — call `sync_wallet` to run the deposit sync and automatic claim sweep, or call `claim_deposit` if you want to claim a known outpoint explicitly:

```text
webhook in  →  build SDK  →  sync_wallet() / claim_deposit()  →  disconnect()
```

### One-time provisioning

When a wallet is first registered, run a one-time setup pass to apply the configuration the client-mode SDK would otherwise apply itself on first startup — currently the [private mode preset](./config.md#private-mode-enabled-by-default):

```text
new wallet registered
  ↓
build SDK (default_server_config + shared infra)
  ↓
apply one-time user settings
  ↓
disconnect()
```

#### Rust

```rust
// One-time setup when a wallet is first registered. The client-mode SDK
// would normally apply the private-mode preset itself on first startup;
// server-mode SDKs do not, so opt in once here via update_user_settings.
sdk.update_user_settings(UpdateUserSettingsRequest {
    spark_private_mode_enabled: Some(true),
    stable_balance_active_label: None,
    spark_master_identity_public_key: None,
})
.await?;

sdk.disconnect().await?;
```

#### Swift

```swift
// One-time setup when a wallet is first registered. The client-mode SDK
// would normally apply the private-mode preset itself on first startup;
// server-mode SDKs do not, so opt in once here via updateUserSettings.
try await sdk.updateUserSettings(
    request: UpdateUserSettingsRequest(sparkPrivateModeEnabled: true))

try await sdk.disconnect()
```

#### Kotlin

```kotlin
// One-time setup when a wallet is first registered. The client-mode
// SDK would normally apply the private-mode preset itself on first
// startup; server-mode SDKs do not, so opt in once here via
// updateUserSettings.
sdk.updateUserSettings(UpdateUserSettingsRequest(sparkPrivateModeEnabled = true))

sdk.disconnect()
```

#### C#

```csharp
// One-time setup when a wallet is first registered. The
// client-mode SDK would normally apply the private-mode preset
// itself on first startup; server-mode SDKs do not, so opt in
// once here via UpdateUserSettings.
await sdk.UpdateUserSettings(
    request: new UpdateUserSettingsRequest(sparkPrivateModeEnabled: true)
);

await sdk.Disconnect();
```

#### Javascript (Wasm)

```typescript
// One-time setup when a wallet is first registered. The client-mode SDK
// would normally apply the private-mode preset itself on first startup;
// server-mode SDKs do not, so opt in once here via updateUserSettings.
await sdk.updateUserSettings({
  sparkPrivateModeEnabled: true,
  stableBalanceActiveLabel: undefined
})

await sdk.disconnect()
```

#### React Native

```typescript
// One-time setup when a wallet is first registered. The client-mode SDK
// would normally apply the private-mode preset itself on first startup;
// server-mode SDKs do not, so opt in once here via updateUserSettings.
await sdk.updateUserSettings({
  sparkPrivateModeEnabled: true,
  stableBalanceActiveLabel: undefined,
  sparkMasterIdentityPublicKey: undefined
})

await sdk.disconnect()
```

#### Flutter

```dart
// One-time setup when a wallet is first registered. The client-mode SDK
// would normally apply the private-mode preset itself on first startup;
// server-mode SDKs do not, so opt in once here via updateUserSettings.
await sdk.updateUserSettings(
  request: UpdateUserSettingsRequest(sparkPrivateModeEnabled: true),
);

await sdk.disconnect();
```

#### Python

```python
# One-time setup when a wallet is first registered. The client-mode SDK
# would normally apply the private-mode preset itself on first startup;
# server-mode SDKs do not, so opt in once here via update_user_settings.
await sdk.update_user_settings(
    request=UpdateUserSettingsRequest(spark_private_mode_enabled=True)
)

await sdk.disconnect()
```

#### Go

```go
// One-time setup when a wallet is first registered. The client-mode SDK
// would normally apply the private-mode preset itself on first startup;
// server-mode SDKs do not, so opt in once here via UpdateUserSettings.
sparkPrivateModeEnabled := true
if err := sdk.UpdateUserSettings(breez_sdk_spark.UpdateUserSettingsRequest{
	SparkPrivateModeEnabled: &sparkPrivateModeEnabled,
}); err != nil {
	return err
}

return sdk.Disconnect()
```



### A few notes

- **Building is cheap when infrastructure is shared.** With the shared chain service, MySQL/Postgres pool, and SSP/Connection Managers configured ([see below](#shared-infrastructure)), each per-request SDK reuses HTTP/2 connections, DB pool slots, and gRPC channels — there's no per-request handshake to operators.
- **Always disconnect.** Even though no background loops are running, calling `disconnect` flushes outstanding storage writes and is the documented lifecycle exit. See [Disconnecting](initializing.md#disconnecting).
- **One SDK per request, not one SDK pinned to a worker thread.** The per-request build is fast enough and avoids cross-tenant state leaks.

## Shared infrastructure

A server-mode deployment normally pairs the profile with shared resources across every per-request SDK. Each of the following is documented in [Customizing the SDK](customizing.md):

- [PostgreSQL Backend](customizing.md#with-postgres-backend) — shared DB pool for storage, tree, and token stores.
- [MySQL Backend](customizing.md#with-mysql-backend) — same for MySQL.
- [Shared REST Chain Service](customizing.md#with-shared-rest-chain-service) — one pooled HTTP client instead of one per SDK.
- [SSP Connection Manager](customizing.md#with-ssp-connection-manager) — share the SSP HTTP client across SDKs.
- [Connection Manager](customizing.md#with-connection-manager) — share gRPC channels to the Spark operators across SDKs.

Pair `default_server_config` with all of these shared resources — sharing the DB pool, chain service, SSP HTTP client, and gRPC channels across SDKs is the intended deployment shape.

## Driving the `background_tasks_enabled` field directly

`default_server_config` is the recommended entry point. If you need to flip the flag on an existing config built another way, see [Background tasks enabled](./config.md#background-tasks-enabled).

---

Identifier casing: `get_info` here is `getInfo` in Swift, Kotlin, JavaScript, React Native and Flutter, and `GetInfo` in Go and C#. Enum variants: `SdkEvent::Synced` is `SdkEvent.SYNCED` in Python, `SdkEvent.synced` in Swift, `SdkEventSynced` in Go, and `SdkEvent.Synced` elsewhere.
