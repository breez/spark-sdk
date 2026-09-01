# Customizing the SDK

Using the SDK Builder gives you more control over the initialization and modular components used when the SDK is running. Below you can find examples of initializing the SDK using the SDK Builder and implementing modular components.

The shared-pool, shared-chain-service, and shared-connection-manager components on this page are designed for multi-tenant server deployments — they're most useful in combination with the [Server mode](server_mode.md) SDK profile.

- [Storage](#with-storage) to manage stored data
- [PostgreSQL Backend](#with-postgres-backend) as an alternative storage backend
- [MySQL Backend](#with-mysql-backend) as an alternative storage backend
- [Bitcoin Chain Service](#with-chain-service) to provide network data
- [Shared REST Chain Service](#with-shared-rest-chain-service) to share the chain service HTTP client across SDK instances
- [LNURL Client](#with-lnurl-client) to make REST requests
- [Fiat Service](#with-fiat-service) to provide Fiat currencies and exchange rates
- Change the [Account Number](#with-account-number) to derive an independent wallet from the same seed
- [Payment Observer](#with-payment-observer) to be notified before payments occur
- [Session Store](#with-session-store) to customize how cached auth tokens are persisted (for example, at-rest encryption)
- [Shared SDK Context](#with-shared-context) to share connection pools and HTTP/gRPC clients across SDK instances

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

// Build the SDK using the config, seed and default storage
let builder = SdkBuilder::new(config, seed).with_default_storage("./.data".to_string());
// You can also pass your custom implementations:
// let builder = builder.with_storage_backend(custom_storage(<your storage implementation>))
// let builder = builder.with_chain_service(<your chain service implementation>)
// let builder = builder.with_rest_client(<your rest client implementation>)
// let builder = builder.with_account_number(<account number>)
// let builder = builder.with_payment_observer(<your payment observer implementation>);
let sdk = builder.build().await?;
```



## With Storage

API docs: https://breez.github.io/spark-sdk/breez_sdk_spark/struct.SdkBuilder.html#method.with_storage

When using the SDK Builder, you either have to provide a Storage implementation or use the default storage from the SDK.

**Note:** Flutter currently only supports using the default storage.

## With PostgreSQL Backend

API docs: https://breez.github.io/spark-sdk/breez_sdk_spark/struct.SdkBuilder.html#method.with_storage_backend

The SDK includes a PostgreSQL backend as an alternative to file-based storage. Build a storage config with `postgres_storage` and pass it to the builder via `with_storage_backend` — this configures PostgreSQL for all stores (storage, tree store, and token store), which is suitable for server-side deployments with horizontal scaling. To share a single connection pool across multiple SDK instances, supply the same config through a [Shared SDK Context](#with-shared-context); per-tenant scoping (rows isolated by seed identity) is preserved either way.

If your service owns the database schema and applies SDK-compatible migrations externally, set `run_migration` to `false` on the storage config. The SDK will trust the existing schema and skip all migration runs, including writes to schema migration tables.

**Note:** Not available for React Native or Flutter. For JavaScript/TypeScript, only supported in Node.js (not in the browser).

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

// Configure PostgreSQL backend
// Connection string format: "host=localhost user=postgres password=secret dbname=spark"
// Or URI format: "postgres://user:password@host:port/dbname?sslmode=require"
// TLS: "sslmode=require" encrypts and verifies the server certificate
let mut postgres_config =
    default_postgres_storage_config("host=localhost user=postgres dbname=spark".to_string());
// Optionally pool settings can be adjusted. Some examples:
postgres_config.max_pool_size = 8; // Max connections in pool
postgres_config.wait_timeout_secs = Some(30); // Timeout waiting for connection

// If your service owns SDK-compatible schema migrations:
postgres_config.run_migration = false;

// Build the SDK with the PostgreSQL storage backend (storage, tree store,
// and token store). Per-tenant scoping (rows isolated by seed identity) is
// applied automatically.
let sdk = SdkBuilder::new(config, seed)
    .with_storage_backend(postgres_storage(postgres_config)?)
    .build()
    .await?;
```



**Developer note**

TLS is controlled by the `sslmode` connection-string parameter. For production, set `sslmode=require`: it encrypts the connection and verifies the server certificate. `verify-ca` and `verify-full` are also supported, and `no-verify` is the explicit opt-in for TLS without certificate verification (for example, a self-signed certificate you cannot add to a trust store). When `sslmode` is absent, TLS is used when the server supports it and is always verified; the exception is JavaScript/TypeScript on Node.js, where an absent `sslmode` means no TLS. Servers using a private CA are trusted via `root_ca_pem` on the storage config, or on Node.js via the `sslrootcert=<path>` URI parameter or Node's trust store (for example, the `NODE_EXTRA_CA_CERTS` environment variable). `verify-ca` performs chain verification without a hostname check and requires a pinned CA: `root_ca_pem` on the storage config, or on Node.js the `sslrootcert=<path>` URI parameter. Use it when hostname verification cannot succeed, such as connecting to the server by IP address.

Sharing the same PostgreSQL database with multiple SDK instances is incompatible with real-time sync. See [Real-time sync server URL](./config.md#real-time-sync-server-url) for how to disable it.

The PostgreSQL tree store can use the same or a separate PostgreSQL database as the PostgreSQL storage. The tree store uses its own set of tables prefixed with `tree_`.

## With MySQL Backend

API docs: https://breez.github.io/spark-sdk/breez_sdk_spark/struct.SdkBuilder.html#method.with_storage_backend

The SDK includes a MySQL backend (MySQL 8.0+) as an alternative to file-based storage. Build a storage config with `mysql_storage` and pass it to the builder via `with_storage_backend` — this configures MySQL for all stores (storage, tree store, and token store), which is suitable for server-side deployments with horizontal scaling. To share a single connection pool across multiple SDK instances, supply the same config through a [Shared SDK Context](#with-shared-context); per-tenant scoping (rows isolated by seed identity) is preserved either way.

If your service owns the database schema and applies SDK-compatible migrations externally, set `run_migration` to `false` on the storage config. The SDK will trust the existing schema and skip all migration runs, including writes to schema migration tables.

**Note:** Not available for React Native or Flutter. For JavaScript/TypeScript, only supported in Node.js (not in the browser).

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

// Configure MySQL backend (MySQL 8.0+).
// Connection string format (URL only):
//   "mysql://user:password@host:3306/dbname?ssl-mode=required"
// TLS: "ssl-mode=required" encrypts and verifies the server certificate
let mut mysql_config =
    default_mysql_storage_config("mysql://user:password@localhost:3306/spark".to_string());
// Optionally pool settings can be adjusted. Some examples:
mysql_config.max_pool_size = 8; // Max connections in pool
mysql_config.recycle_timeout_secs = Some(60); // Recycle idle connections after this many seconds

// Provide a custom CA certificate when the server uses a private CA:
// mysql_config.root_ca_pem = Some("-----BEGIN CERTIFICATE-----\n...".to_string());

// Build the SDK with the MySQL storage backend (storage, tree store, and
// token store). Per-tenant scoping (rows isolated by seed identity) is
// applied automatically.
let sdk = SdkBuilder::new(config, seed)
    .with_storage_backend(mysql_storage(mysql_config)?)
    .build()
    .await?;
```



**Developer note**

MySQL only accepts URL-form connection strings (`mysql://user:password@host:3306/dbname`); the key=value form supported by PostgreSQL is not available. TLS is controlled by the `ssl-mode` URL parameter, with the same spellings on every platform: `required` (recommended for production) and `verify_identity` verify the server certificate chain and hostname, `verify_ca` verifies the chain only, and `no-verify` is the explicit opt-in for TLS without certificate verification. An absent `ssl-mode` means no TLS. Servers using a private CA are trusted via `root_ca_pem` on the storage config, or on JavaScript/TypeScript (Node.js) via the `ssl-ca=<path>` URL parameter or Node's trust store (for example, the `NODE_EXTRA_CA_CERTS` environment variable). `verify_ca` performs chain verification without a hostname check and requires a pinned CA: `root_ca_pem` on the storage config, or on Node.js the `ssl-ca=<path>` URL parameter. Use it when hostname verification cannot succeed, such as connecting to the server by IP address.

Sharing the same MySQL database with multiple SDK instances is incompatible with real-time sync. See [Real-time sync server URL](./config.md#real-time-sync-server-url) for how to disable it.

The MySQL tree store can use the same or a separate MySQL database as the MySQL storage. The tree store uses its own set of tables prefixed with `tree_`.

## With Chain Service

API docs: https://breez.github.io/spark-sdk/breez_sdk_spark/struct.SdkBuilder.html#method.with_chain_service

The SDK provides a default Bitcoin Chain Service implementation. If you want to use your own, you can provide it either by using [With REST Chain Service](#with-rest-chain-service) or by implementing the Bitcoin Chain Service interface.

## With REST Chain Service

API docs: https://breez.github.io/spark-sdk/breez_sdk_spark/struct.SdkBuilder.html#method.with_rest_chain_service

The SDK provides a default Bitcoin Chain Service implementation. If you want to use your own, you can provide it either by using [With Chain Service](#with-chain-service) or by providing a URL and optional credentials.

```rust
let url = "<your REST chain service URL>".to_string();
let chain_api_type = ChainApiType::MempoolSpace;
let optional_credentials = Credentials {
    username: "<username>".to_string(),
    password: "<password>".to_string(),
};
builder.with_rest_chain_service(url, chain_api_type, Some(optional_credentials))
```



## With Shared REST Chain Service

API docs: https://breez.github.io/spark-sdk/breez_sdk_spark/fn.new_rest_chain_service.html

[With REST Chain Service](#with-rest-chain-service) builds a fresh chain service inside each SDK instance. Server processes hosting many wallets at once can share a single REST chain service between every SDK, so they reuse the same pooled HTTP client (and its HTTP/2 connection pool) instead of each opening a fresh one.

Construct one via `new_rest_chain_service` and pass it to each `SdkBuilder` via `with_chain_service`. All SDK instances sharing the chain service must be configured for the same network.

## With Fiat Service

API docs: https://breez.github.io/spark-sdk/breez_sdk_spark/struct.SdkBuilder.html#method.with_fiat_service

The SDK by default provides a list of available Fiat currencies and current exchange rates. If you want to use your own, you can provide it by implementing the Fiat Service interface.

## With LNURL Client

API docs: https://breez.github.io/spark-sdk/breez_sdk_spark/struct.SdkBuilder.html#method.with_lnurl_client

The LNURL Client is used to make REST requests specifically when interacting with LNURL. If you want to use your own, you can it provide by implementing the REST Service interface.

## With Account Number

API docs: https://breez.github.io/spark-sdk/breez_sdk_spark/struct.SdkBuilder.html#method.with_account_number

The SDK derives all wallet keys from the seed at the derivation path `m/8797555'/<account number>'`. By default the account number is 0 on Regtest and 1 on all other networks. Set a different account number to derive an independent wallet from the same seed:

```rust
let account_number = 21;
builder.with_account_number(account_number)
```



## With Payment Observer

API docs: https://breez.github.io/spark-sdk/breez_sdk_spark/struct.SdkBuilder.html#method.with_payment_observer

By implementing the Payment Observer interface you can be notified before a payment is sent. It includes information about the provisional payment including the payment ID, amount to be sent (in satoshis or token base units) and payment details based on the payment method.

**Note:** Flutter currently does not support this.

```rust
pub(crate) struct ExamplePaymentObserver {}

#[async_trait]
impl PaymentObserver for ExamplePaymentObserver {
    async fn before_send(
        &self,
        payments: Vec<ProvisionalPayment>,
    ) -> Result<(), PaymentObserverError> {
        for payment in payments {
            info!(
                "About to send payment: {:?} of amount {:?}",
                payment.payment_id, payment.amount
            );
        }
        Ok(())
    }

    async fn after_send(&self, updates: Vec<PaymentIdUpdate>) -> Result<(), PaymentObserverError> {
        for update in updates {
            info!(
                "Token tx broadcast: {} -> {}",
                update.provisional_payment_id, update.final_payment_id
            );
        }
        Ok(())
    }
}

pub(crate) fn with_payment_observer(builder: SdkBuilder) -> SdkBuilder {
    let observer = ExamplePaymentObserver {};
    builder.with_payment_observer(Arc::new(observer))
}
```



## With Session Store

API docs: https://breez.github.io/spark-sdk/breez_sdk_spark/struct.SdkBuilder.html#method.with_session_store

The SDK caches the auth tokens it obtains from the Spark operators and the SSP in a session store, keyed by each service's identity. By default the store is provided by the storage backend (a `brz_`-prefixed table on the PostgreSQL/MySQL backends, an in-memory store otherwise), and tokens are stored as-is.

Use `with_session_store` to provide your own `SessionStore`. This can be a completely different persistence layer, or a decorator that wraps the backend's own store to transform tokens on read and write while keeping its persistence: fetch the backend's store with `default_session_store`, then intercept `get_session` and `set_session`.

At-rest encryption is one such transform (the SDK does not encrypt tokens itself), shown below: encrypt the token in `set_session` and decrypt it in `get_session`.

```rust
pub(crate) struct EncryptingSessionStore {
    inner: Arc<dyn SessionStore>,
}

#[async_trait]
impl SessionStore for EncryptingSessionStore {
    async fn get_session(
        &self,
        service_identity_key: PublicKey,
    ) -> Result<Session, SessionStoreError> {
        let session = self.inner.get_session(service_identity_key).await?;
        // Decrypt session.token here before returning it.
        Ok(session)
    }

    async fn set_session(
        &self,
        service_identity_key: PublicKey,
        session: Session,
    ) -> Result<(), SessionStoreError> {
        // Encrypt session.token here before persisting it.
        self.inner.set_session(service_identity_key, session).await
    }
}

// `identity` is the wallet identity public key bytes, used to scope the store.
pub(crate) async fn with_session_store(
    config: Config,
    seed: Seed,
    identity: Vec<u8>,
) -> Result<SdkBuilder> {
    // Reuse one storage backend for both the SDK storage and the session store.
    let backend = default_storage("./.data".to_string());

    // Get the session store the backend provides, then wrap it to add encryption.
    let inner = default_session_store(backend.clone(), config.network, identity).await?;
    let session_store = Arc::new(EncryptingSessionStore { inner });

    Ok(SdkBuilder::new(config, seed)
        .with_storage_backend(backend)
        .with_session_store(session_store))
}
```



**Developer note**

When wrapping the backend's store, pass the same storage backend to both `with_storage_backend` and `default_session_store` so the session store shares the SDK's persistence. On the WASM binding `default_session_store` takes the storage config and the wallet identity public key (hex) instead of a backend.

**Note:** Not supported in Flutter.

## With Shared SDK Context

API docs: https://breez.github.io/spark-sdk/breez_sdk_spark/struct.SdkContext.html

An SDK Context bundles every process-shareable resource: the HTTP client (used for SSP GraphQL, chain service and LNURL), the gRPC channels to the Spark operators, the gRPC client to the Breez backend, and — optionally — a PostgreSQL or MySQL connection pool. By default each SDK builds its own. Server processes hosting many wallets at once can construct one SDK Context and pass it to every `SdkBuilder` so they reuse the same pooled clients instead of each opening fresh ones.

Construct one via `new_shared_sdk_context` and pass it to each `SdkBuilder` via `with_shared_context`. Connections close when the last reference to the SDK Context is dropped; calling `disconnect` on an SDK instance does not affect them.

The `connections_per_operator` setting on `SdkContextConfig` controls how many gRPC connections the context opens to each Spark operator:

- `None` — one connection per operator, multiplexed across every SDK sharing this context. The right choice for almost every deployment.
- `Some(n)` — opens `n` connections per operator and balances requests across them. Worth setting only if the single shared connection has become a bottleneck — for example, latency that climbs with throughput, or operators deployed behind an L7 load balancer where you want client-side fan-out across backend instances.

To route a context's pooled connections through a SOCKS5 proxy, set `proxy` on `SdkContextConfig` as well as on each SDK's `Config`. See [SOCKS5 proxy](./proxy.md).

**Developer note**

All SDK instances sharing an SDK Context must be configured for the same network and operator pool, and must agree on `proxy`. The user agent of the first SDK to construct the context is reused for all subsequent instances.

### Browser

The SDK Context's gRPC channel pooling is not effective in the browser. Browsers maintain a single HTTP/2 connection per origin and multiplex everything over it; the SDK cannot create or share more.

### Node.js

Node's global `fetch` (undici) negotiates HTTP/2 with the Spark operators automatically and opens additional connections per origin as needed, so most deployments need no tuning. If you do want to cap or expand the per-origin pool, configure undici globally before initialising the SDK:

```js
import { Agent, setGlobalDispatcher } from 'undici'
setGlobalDispatcher(new Agent({ connections: 8 }))
```

This affects every `fetch` in the process, including the SDK's gRPC-web traffic.
