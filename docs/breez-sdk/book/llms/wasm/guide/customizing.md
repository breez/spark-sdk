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

```typescript
// Call init when using the SDK in a web environment before calling any other SDK
// methods. This is not needed when using the SDK in a Node.js/Deno environment.
await init()

// Construct the seed using a mnemonic, entropy or passkey
const mnemonic = '<mnemonic words>'
const seed: Seed = { type: 'mnemonic', mnemonic, passphrase: undefined }

// Create the default config
const config = defaultConfig('mainnet')
config.apiKey = '<breez api key>'

// Build the SDK using the config, seed and default storage
let builder = SdkBuilder.new(config, seed)
builder = await builder.withDefaultStorage('./.data')
// You can also pass your custom implementations:
// builder = builder.withStorage(<your storage implementation>)
// builder = builder.withChainService(<your chain service implementation>)
// builder = builder.withRestClient(<your rest client implementation>)
// builder = builder.withAccountNumber(<account number>)
// builder = builder.withPaymentObserver(<your payment observer implementation>)
const sdk = await builder.build()
```



## With Storage

API docs: https://breez.github.io/spark-sdk/breez_sdk_spark/struct.SdkBuilder.html#method.with_storage

When using the SDK Builder, you either have to provide a Storage implementation or use the default storage from the SDK.

**Note:** Flutter currently only supports using the default storage.

## With PostgreSQL Backend

API docs: https://breez.github.io/spark-sdk/breez_sdk_spark/struct.SdkBuilder.html#method.with_storage_backend

The SDK includes a PostgreSQL backend as an alternative to file-based storage. Build a storage config with `postgresStorage` and pass it to the builder via `withStorageBackend` — this configures PostgreSQL for all stores (storage, tree store, and token store), which is suitable for server-side deployments with horizontal scaling. To share a single connection pool across multiple SDK instances, supply the same config through a [Shared SDK Context](#with-shared-context); per-tenant scoping (rows isolated by seed identity) is preserved either way.

If your service owns the database schema and applies SDK-compatible migrations externally, set `runMigration` to `false` on the storage config. The SDK will trust the existing schema and skip all migration runs, including writes to schema migration tables.

**Note:** Not available for React Native or Flutter. For JavaScript/TypeScript, only supported in Node.js (not in the browser).

```typescript
// Construct the seed using a mnemonic, entropy or passkey
const mnemonic = '<mnemonic words>'
const seed: Seed = { type: 'mnemonic', mnemonic, passphrase: undefined }

// Create the default config
const config = defaultConfig('mainnet')
config.apiKey = '<breez api key>'

// Configure PostgreSQL backend
// Connection string format: "host=localhost user=postgres password=secret dbname=spark"
// Or URI format: "postgres://user:password@host:port/dbname?sslmode=require"
// TLS: "sslmode=require" encrypts and verifies the server certificate
const pgConfig = defaultPostgresStorageConfig('host=localhost user=postgres dbname=spark')
// Optionally pool settings can be adjusted. Some examples:
pgConfig.maxPoolSize = 8 // Max connections in pool
pgConfig.createTimeoutSecs = 30 // Timeout for establishing a new connection
pgConfig.recycleTimeoutSecs = 30 // Timeout for recycling an idle connection
// If your service owns SDK-compatible schema migrations:
pgConfig.runMigration = false

// Build the SDK with the PostgreSQL storage backend (storage, tree store,
// and token store). Per-tenant scoping (rows isolated by seed identity) is
// applied automatically.
let builder = SdkBuilder.new(config, seed)
builder = builder.withStorageBackend(postgresStorage(pgConfig))
const sdk = await builder.build()
```



**Developer note**

TLS is controlled by the `sslmode` connection-string parameter. For production, set `sslmode=require`: it encrypts the connection and verifies the server certificate. `verify-ca` and `verify-full` are also supported, and `no-verify` is the explicit opt-in for TLS without certificate verification (for example, a self-signed certificate you cannot add to a trust store). When `sslmode` is absent, TLS is used when the server supports it and is always verified; the exception is JavaScript/TypeScript on Node.js, where an absent `sslmode` means no TLS. Servers using a private CA are trusted via `rootCaPem` on the storage config, or on Node.js via the `sslrootcert=<path>` URI parameter or Node's trust store (for example, the `NODE_EXTRA_CA_CERTS` environment variable). `verify-ca` performs chain verification without a hostname check and requires a pinned CA: `rootCaPem` on the storage config, or on Node.js the `sslrootcert=<path>` URI parameter. Use it when hostname verification cannot succeed, such as connecting to the server by IP address.

Sharing the same PostgreSQL database with multiple SDK instances is incompatible with real-time sync. See [Real-time sync server URL](./config.md#real-time-sync-server-url) for how to disable it.

The PostgreSQL tree store can use the same or a separate PostgreSQL database as the PostgreSQL storage. The tree store uses its own set of tables prefixed with `tree_`.

## With MySQL Backend

API docs: https://breez.github.io/spark-sdk/breez_sdk_spark/struct.SdkBuilder.html#method.with_storage_backend

The SDK includes a MySQL backend (MySQL 8.0+) as an alternative to file-based storage. Build a storage config with `mysqlStorage` and pass it to the builder via `withStorageBackend` — this configures MySQL for all stores (storage, tree store, and token store), which is suitable for server-side deployments with horizontal scaling. To share a single connection pool across multiple SDK instances, supply the same config through a [Shared SDK Context](#with-shared-context); per-tenant scoping (rows isolated by seed identity) is preserved either way.

If your service owns the database schema and applies SDK-compatible migrations externally, set `runMigration` to `false` on the storage config. The SDK will trust the existing schema and skip all migration runs, including writes to schema migration tables.

**Note:** Not available for React Native or Flutter. For JavaScript/TypeScript, only supported in Node.js (not in the browser).

```typescript
// Construct the seed using a mnemonic, entropy or passkey
const mnemonic = '<mnemonic words>'
const seed: Seed = { type: 'mnemonic', mnemonic, passphrase: undefined }

// Create the default config
const config = defaultConfig('mainnet')
config.apiKey = '<breez api key>'

// Configure MySQL backend (MySQL 8.0+).
// Connection string format (URL only):
//   "mysql://user:password@host:3306/dbname?ssl-mode=required"
// TLS: "ssl-mode=required" encrypts and verifies the server certificate
const mysqlConfig = defaultMysqlStorageConfig('mysql://user:password@localhost:3306/spark')
// Optionally pool settings can be adjusted. Some examples:
mysqlConfig.maxPoolSize = 8 // Max connections in pool
mysqlConfig.createTimeoutSecs = 30 // Timeout for establishing a new connection
mysqlConfig.recycleTimeoutSecs = 60 // Recycle idle connections after this many seconds

// To trust a private CA, add it to Node's trust store
// (e.g. via the NODE_EXTRA_CA_CERTS environment variable)

// Build the SDK with the MySQL storage backend (storage, tree store, and
// token store). Per-tenant scoping (rows isolated by seed identity) is
// applied automatically.
let builder = SdkBuilder.new(config, seed)
builder = builder.withStorageBackend(mysqlStorage(mysqlConfig))
const sdk = await builder.build()
```



**Developer note**

MySQL only accepts URL-form connection strings (`mysql://user:password@host:3306/dbname`); the key=value form supported by PostgreSQL is not available. TLS is controlled by the `ssl-mode` URL parameter, with the same spellings on every platform: `required` (recommended for production) and `verify_identity` verify the server certificate chain and hostname, `verify_ca` verifies the chain only, and `no-verify` is the explicit opt-in for TLS without certificate verification. An absent `ssl-mode` means no TLS. Servers using a private CA are trusted via `rootCaPem` on the storage config, or on JavaScript/TypeScript (Node.js) via the `ssl-ca=<path>` URL parameter or Node's trust store (for example, the `NODE_EXTRA_CA_CERTS` environment variable). `verify_ca` performs chain verification without a hostname check and requires a pinned CA: `rootCaPem` on the storage config, or on Node.js the `ssl-ca=<path>` URL parameter. Use it when hostname verification cannot succeed, such as connecting to the server by IP address.

Sharing the same MySQL database with multiple SDK instances is incompatible with real-time sync. See [Real-time sync server URL](./config.md#real-time-sync-server-url) for how to disable it.

The MySQL tree store can use the same or a separate MySQL database as the MySQL storage. The tree store uses its own set of tables prefixed with `tree_`.

## With Chain Service

API docs: https://breez.github.io/spark-sdk/breez_sdk_spark/struct.SdkBuilder.html#method.with_chain_service

The SDK provides a default Bitcoin Chain Service implementation. If you want to use your own, you can provide it either by using [With REST Chain Service](#with-rest-chain-service) or by implementing the Bitcoin Chain Service interface.

## With REST Chain Service

API docs: https://breez.github.io/spark-sdk/breez_sdk_spark/struct.SdkBuilder.html#method.with_rest_chain_service

The SDK provides a default Bitcoin Chain Service implementation. If you want to use your own, you can provide it either by using [With Chain Service](#with-chain-service) or by providing a URL and optional credentials.

```typescript
const url = '<your REST chain service URL>'
const chainApiType = 'mempoolSpace'
const optionalCredentials: Credentials = {
  username: '<username>',
  password: '<password>'
}
builder = builder.withRestChainService(url, chainApiType, optionalCredentials)
```



## With Shared REST Chain Service

API docs: https://breez.github.io/spark-sdk/breez_sdk_spark/fn.new_rest_chain_service.html

[With REST Chain Service](#with-rest-chain-service) builds a fresh chain service inside each SDK instance. Server processes hosting many wallets at once can share a single REST chain service between every SDK, so they reuse the same pooled HTTP client (and its HTTP/2 connection pool) instead of each opening a fresh one.

Construct one via `newRestChainService` and pass it to each `SdkBuilder` via `withChainService`. All SDK instances sharing the chain service must be configured for the same network.

## With Fiat Service

API docs: https://breez.github.io/spark-sdk/breez_sdk_spark/struct.SdkBuilder.html#method.with_fiat_service

The SDK by default provides a list of available Fiat currencies and current exchange rates. If you want to use your own, you can provide it by implementing the Fiat Service interface.

## With LNURL Client

API docs: https://breez.github.io/spark-sdk/breez_sdk_spark/struct.SdkBuilder.html#method.with_lnurl_client

The LNURL Client is used to make REST requests specifically when interacting with LNURL. If you want to use your own, you can it provide by implementing the REST Service interface.

## With Account Number

API docs: https://breez.github.io/spark-sdk/breez_sdk_spark/struct.SdkBuilder.html#method.with_account_number

The SDK derives all wallet keys from the seed at the derivation path `m/8797555'/<account number>'`. By default the account number is 0 on Regtest and 1 on all other networks. Set a different account number to derive an independent wallet from the same seed:

```typescript
builder = builder.withAccountNumber(21)
```



## With Payment Observer

API docs: https://breez.github.io/spark-sdk/breez_sdk_spark/struct.SdkBuilder.html#method.with_payment_observer

By implementing the Payment Observer interface you can be notified before a payment is sent. It includes information about the provisional payment including the payment ID, amount to be sent (in satoshis or token base units) and payment details based on the payment method.

**Note:** Flutter currently does not support this.

```typescript
class ExamplePaymentObserver {
  beforeSend = async (payments: ProvisionalPayment[]) => {
    for (const payment of payments) {
      console.log(`About to send payment: ${payment.paymentId} of amount ${payment.amount}`)
    }
  }

  afterSend = async (updates: PaymentIdUpdate[]) => {
    for (const update of updates) {
      console.log(`Token tx broadcast: ${update.provisionalPaymentId} -> ${update.finalPaymentId}`)
    }
  }
}

const exampleWithPaymentObserver = (builder: SdkBuilder): SdkBuilder => {
  const paymentObserver = new ExamplePaymentObserver()
  return builder.withPaymentObserver(paymentObserver)
}
```



## With Session Store

API docs: https://breez.github.io/spark-sdk/breez_sdk_spark/struct.SdkBuilder.html#method.with_session_store

The SDK caches the auth tokens it obtains from the Spark operators and the SSP in a session store, keyed by each service's identity. By default the store is provided by the storage backend (a `brz_`-prefixed table on the PostgreSQL/MySQL backends, an in-memory store otherwise), and tokens are stored as-is.

Use `withSessionStore` to provide your own `SessionStore`. This can be a completely different persistence layer, or a decorator that wraps the backend's own store to transform tokens on read and write while keeping its persistence: fetch the backend's store with `defaultSessionStore`, then intercept `getSession` and `setSession`.

At-rest encryption is one such transform (the SDK does not encrypt tokens itself), shown below: encrypt the token in `setSession` and decrypt it in `getSession`.

```typescript
class EncryptingSessionStore implements SessionStore {
  constructor (private readonly inner: DefaultSessionStore) {}

  getSession = async (serviceIdentityKey: string): Promise<Session> => {
    const session = await this.inner.getSession(serviceIdentityKey)
    // Decrypt session.token here before returning it.
    return session
  }

  setSession = async (serviceIdentityKey: string, session: Session): Promise<void> => {
    // Encrypt session.token here before persisting it.
    await this.inner.setSession(serviceIdentityKey, session)
  }
}

const exampleWithSessionStore = async (identity: string): Promise<SdkBuilder> => {
  // Construct the seed using a mnemonic, entropy or passkey
  const mnemonic = '<mnemonic words>'
  const seed: Seed = { type: 'mnemonic', mnemonic, passphrase: undefined }

  // Create the default config
  const config = defaultConfig('mainnet')
  config.apiKey = '<breez api key>'

  // Get the backend's own session store, then wrap it to encrypt at rest.
  // identity is the wallet identity public key, hex.
  const storageConfig = defaultStorage('./.data')
  const inner = await defaultSessionStore(storageConfig, 'mainnet', identity)
  const sessionStore = new EncryptingSessionStore(inner)

  let builder = SdkBuilder.new(config, seed)
  builder = builder.withStorageBackend(storageConfig)
  builder = builder.withSessionStore(sessionStore)
  return builder
}
```



**Developer note**

When wrapping the backend's store, pass the same storage backend to both `withStorageBackend` and `defaultSessionStore` so the session store shares the SDK's persistence. On the WASM binding `defaultSessionStore` takes the storage config and the wallet identity public key (hex) instead of a backend.

**Note:** Not supported in Flutter.

## With Shared SDK Context

API docs: https://breez.github.io/spark-sdk/breez_sdk_spark/struct.SdkContext.html

An SDK Context bundles every process-shareable resource: the HTTP client (used for SSP GraphQL, chain service and LNURL), the gRPC channels to the Spark operators, the gRPC client to the Breez backend, and — optionally — a PostgreSQL or MySQL connection pool. By default each SDK builds its own. Server processes hosting many wallets at once can construct one SDK Context and pass it to every `SdkBuilder` so they reuse the same pooled clients instead of each opening fresh ones.

Construct one via `newSharedSdkContext` and pass it to each `SdkBuilder` via `withSharedContext`. Connections close when the last reference to the SDK Context is dropped; calling `disconnect` on an SDK instance does not affect them.

The `connectionsPerOperator` setting on `SdkContextConfig` controls how many gRPC connections the context opens to each Spark operator:

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
