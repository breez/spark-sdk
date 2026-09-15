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

// Build the SDK using the config, seed and default storage
builder := breez_sdk_spark.NewSdkBuilder(config, seed)
builder.WithDefaultStorage("./.data")
// You can also pass your custom implementations:
// builder.WithStorage(<your storage implementation>)
// builder.WithChainService(<your chain service implementation>)
// builder.WithRestClient(<your rest client implementation>)
// builder.WithAccountNumber(<account number>)
// builder.WithPaymentObserver(<your payment observer implementation>)
sdk, err := builder.Build()

return sdk, err
```



## With Storage

API docs: https://breez.github.io/spark-sdk/breez_sdk_spark/struct.SdkBuilder.html#method.with_storage

When using the SDK Builder, you either have to provide a Storage implementation or use the default storage from the SDK.

**Note:** Flutter currently only supports using the default storage.

## With PostgreSQL Backend

API docs: https://breez.github.io/spark-sdk/breez_sdk_spark/struct.SdkBuilder.html#method.with_storage_backend

The SDK includes a PostgreSQL backend as an alternative to file-based storage. Build a storage config with `PostgresStorage` and pass it to the builder via `WithStorageBackend` — this configures PostgreSQL for all stores (storage, tree store, and token store), which is suitable for server-side deployments with horizontal scaling. To share a single connection pool across multiple SDK instances, supply the same config through a [Shared SDK Context](#with-shared-context); per-tenant scoping (rows isolated by seed identity) is preserved either way.

If your service owns the database schema and applies SDK-compatible migrations externally, set `RunMigration` to `false` on the storage config. The SDK will trust the existing schema and skip all migration runs, including writes to schema migration tables.

**Note:** Not available for React Native or Flutter. For JavaScript/TypeScript, only supported in Node.js (not in the browser).

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

// Configure PostgreSQL backend
// Connection string format: "host=localhost user=postgres password=secret dbname=spark"
// Or URI format: "postgres://user:password@host:port/dbname?sslmode=require"
// TLS: "sslmode=require" encrypts and verifies the server certificate
postgresConfig := breez_sdk_spark.DefaultPostgresStorageConfig(
	"host=localhost user=postgres dbname=spark",
)
// Optionally pool settings can be adjusted. Some examples:
postgresConfig.MaxPoolSize = 8 // Max connections in pool
waitTimeoutSecs := uint64(30)
postgresConfig.WaitTimeoutSecs = &waitTimeoutSecs // Timeout waiting for connection
// If your service owns SDK-compatible schema migrations:
postgresConfig.RunMigration = false

// Build the SDK with the PostgreSQL storage backend (storage, tree store,
// and token store). Per-tenant scoping (rows isolated by seed identity)
// is applied automatically.
builder := breez_sdk_spark.NewSdkBuilder(config, seed)
storageBackend, err := breez_sdk_spark.PostgresStorage(postgresConfig)
if err != nil {
	return nil, err
}
builder.WithStorageBackend(storageBackend)
sdk, err := builder.Build()
if err != nil {
	return nil, err
}
```



**Developer note**

TLS is controlled by the `sslmode` connection-string parameter. For production, set `sslmode=require`: it encrypts the connection and verifies the server certificate. `verify-ca` and `verify-full` are also supported, and `no-verify` is the explicit opt-in for TLS without certificate verification (for example, a self-signed certificate you cannot add to a trust store). When `sslmode` is absent, TLS is used when the server supports it and is always verified; the exception is JavaScript/TypeScript on Node.js, where an absent `sslmode` means no TLS. Servers using a private CA are trusted via `RootCaPem` on the storage config, or on Node.js via the `sslrootcert=<path>` URI parameter or Node's trust store (for example, the `NODE_EXTRA_CA_CERTS` environment variable). `verify-ca` performs chain verification without a hostname check and requires a pinned CA: `RootCaPem` on the storage config, or on Node.js the `sslrootcert=<path>` URI parameter. Use it when hostname verification cannot succeed, such as connecting to the server by IP address.

Sharing the same PostgreSQL database with multiple SDK instances is incompatible with real-time sync. See [Real-time sync server URL](./config.md#real-time-sync-server-url) for how to disable it.

The PostgreSQL tree store can use the same or a separate PostgreSQL database as the PostgreSQL storage. The tree store uses its own set of tables prefixed with `tree_`.

## With MySQL Backend

API docs: https://breez.github.io/spark-sdk/breez_sdk_spark/struct.SdkBuilder.html#method.with_storage_backend

The SDK includes a MySQL backend (MySQL 8.0+) as an alternative to file-based storage. Build a storage config with `MysqlStorage` and pass it to the builder via `WithStorageBackend` — this configures MySQL for all stores (storage, tree store, and token store), which is suitable for server-side deployments with horizontal scaling. To share a single connection pool across multiple SDK instances, supply the same config through a [Shared SDK Context](#with-shared-context); per-tenant scoping (rows isolated by seed identity) is preserved either way.

If your service owns the database schema and applies SDK-compatible migrations externally, set `RunMigration` to `false` on the storage config. The SDK will trust the existing schema and skip all migration runs, including writes to schema migration tables.

**Note:** Not available for React Native or Flutter. For JavaScript/TypeScript, only supported in Node.js (not in the browser).

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

// Configure MySQL backend (MySQL 8.0+).
// Connection string format (URL only):
//   "mysql://user:password@host:3306/dbname?ssl-mode=required"
// TLS: "ssl-mode=required" encrypts and verifies the server certificate
mysqlConfig := breez_sdk_spark.DefaultMysqlStorageConfig(
	"mysql://user:password@localhost:3306/spark",
)
// Optionally pool settings can be adjusted. Some examples:
mysqlConfig.MaxPoolSize = 8 // Max connections in pool
recycleTimeoutSecs := uint64(60)
// Recycle idle connections after this many seconds
mysqlConfig.RecycleTimeoutSecs = &recycleTimeoutSecs
// Provide a custom CA certificate when the server uses a private CA:
// rootCa := "-----BEGIN CERTIFICATE-----\n..."
// mysqlConfig.RootCaPem = &rootCa

// Build the SDK with the MySQL storage backend (storage, tree store, and
// token store). Per-tenant scoping (rows isolated by seed identity) is
// applied automatically.
builder := breez_sdk_spark.NewSdkBuilder(config, seed)
storageBackend, err := breez_sdk_spark.MysqlStorage(mysqlConfig)
if err != nil {
	return nil, err
}
builder.WithStorageBackend(storageBackend)
sdk, err := builder.Build()
if err != nil {
	return nil, err
}
```



**Developer note**

MySQL only accepts URL-form connection strings (`mysql://user:password@host:3306/dbname`); the key=value form supported by PostgreSQL is not available. TLS is controlled by the `ssl-mode` URL parameter, with the same spellings on every platform: `required` (recommended for production) and `verify_identity` verify the server certificate chain and hostname, `verify_ca` verifies the chain only, and `no-verify` is the explicit opt-in for TLS without certificate verification. An absent `ssl-mode` means no TLS. Servers using a private CA are trusted via `RootCaPem` on the storage config, or on JavaScript/TypeScript (Node.js) via the `ssl-ca=<path>` URL parameter or Node's trust store (for example, the `NODE_EXTRA_CA_CERTS` environment variable). `verify_ca` performs chain verification without a hostname check and requires a pinned CA: `RootCaPem` on the storage config, or on Node.js the `ssl-ca=<path>` URL parameter. Use it when hostname verification cannot succeed, such as connecting to the server by IP address.

Sharing the same MySQL database with multiple SDK instances is incompatible with real-time sync. See [Real-time sync server URL](./config.md#real-time-sync-server-url) for how to disable it.

The MySQL tree store can use the same or a separate MySQL database as the MySQL storage. The tree store uses its own set of tables prefixed with `tree_`.

## With Chain Service

API docs: https://breez.github.io/spark-sdk/breez_sdk_spark/struct.SdkBuilder.html#method.with_chain_service

The SDK provides a default Bitcoin Chain Service implementation. If you want to use your own, you can provide it either by using [With REST Chain Service](#with-rest-chain-service) or by implementing the Bitcoin Chain Service interface.

## With REST Chain Service

API docs: https://breez.github.io/spark-sdk/breez_sdk_spark/struct.SdkBuilder.html#method.with_rest_chain_service

The SDK provides a default Bitcoin Chain Service implementation. If you want to use your own, you can provide it either by using [With Chain Service](#with-chain-service) or by providing a URL and optional credentials.

```go
url := "<your REST chain service URL>"
chainApiType := breez_sdk_spark.ChainApiTypeMempoolSpace
optionalCredentials := &breez_sdk_spark.Credentials{
	Username: "<username>",
	Password: "<password>",
}
builder.WithRestChainService(url, chainApiType, optionalCredentials)
```



## With Shared REST Chain Service

API docs: https://breez.github.io/spark-sdk/breez_sdk_spark/fn.new_rest_chain_service.html

[With REST Chain Service](#with-rest-chain-service) builds a fresh chain service inside each SDK instance. Server processes hosting many wallets at once can share a single REST chain service between every SDK, so they reuse the same pooled HTTP client (and its HTTP/2 connection pool) instead of each opening a fresh one.

Construct one via `NewRestChainService` and pass it to each `SdkBuilder` via `WithChainService`. All SDK instances sharing the chain service must be configured for the same network.

## With Fiat Service

API docs: https://breez.github.io/spark-sdk/breez_sdk_spark/struct.SdkBuilder.html#method.with_fiat_service

The SDK by default provides a list of available Fiat currencies and current exchange rates. If you want to use your own, you can provide it by implementing the Fiat Service interface.

## With LNURL Client

API docs: https://breez.github.io/spark-sdk/breez_sdk_spark/struct.SdkBuilder.html#method.with_lnurl_client

The LNURL Client is used to make REST requests specifically when interacting with LNURL. If you want to use your own, you can it provide by implementing the REST Service interface.

## With Account Number

API docs: https://breez.github.io/spark-sdk/breez_sdk_spark/struct.SdkBuilder.html#method.with_account_number

The SDK derives all wallet keys from the seed at the derivation path `m/8797555'/<account number>'`. By default the account number is 0 on Regtest and 1 on all other networks. Set a different account number to derive an independent wallet from the same seed:

```go
accountNumber := uint32(21)
builder.WithAccountNumber(accountNumber)
```



## With Payment Observer

API docs: https://breez.github.io/spark-sdk/breez_sdk_spark/struct.SdkBuilder.html#method.with_payment_observer

By implementing the Payment Observer interface you can be notified before a payment is sent. It includes information about the provisional payment including the payment ID, amount to be sent (in satoshis or token base units) and payment details based on the payment method.

**Note:** Flutter currently does not support this.

```go
type ExamplePaymentObserver struct{}

func (ExamplePaymentObserver) BeforeSend(payments []breez_sdk_spark.ProvisionalPayment) error {
	for _, payment := range payments {
		log.Printf("About to send payment: %v of amount %v", payment.PaymentId, payment.Amount)
	}
	return nil
}

func (ExamplePaymentObserver) AfterSend(updates []breez_sdk_spark.PaymentIdUpdate) error {
	for _, update := range updates {
		log.Printf("Token tx broadcast: %v -> %v", update.ProvisionalPaymentId, update.FinalPaymentId)
	}
	return nil
}

func WithPaymentObserver(builder *breez_sdk_spark.SdkBuilder) {
	observer := ExamplePaymentObserver{}
	builder.WithPaymentObserver(observer)
}
```



## With Session Store

API docs: https://breez.github.io/spark-sdk/breez_sdk_spark/struct.SdkBuilder.html#method.with_session_store

The SDK caches the auth tokens it obtains from the Spark operators and the SSP in a session store, keyed by each service's identity. By default the store is provided by the storage backend (a `brz_`-prefixed table on the PostgreSQL/MySQL backends, an in-memory store otherwise), and tokens are stored as-is.

Use `WithSessionStore` to provide your own `SessionStore`. This can be a completely different persistence layer, or a decorator that wraps the backend's own store to transform tokens on read and write while keeping its persistence: fetch the backend's store with `DefaultSessionStore`, then intercept `GetSession` and `SetSession`.

At-rest encryption is one such transform (the SDK does not encrypt tokens itself), shown below: encrypt the token in `SetSession` and decrypt it in `GetSession`.

```go
type EncryptingSessionStore struct {
	inner breez_sdk_spark.SessionStore
}

func (s EncryptingSessionStore) GetSession(
	serviceIdentityKey string,
) (breez_sdk_spark.Session, error) {
	session, err := s.inner.GetSession(serviceIdentityKey)
	if err != nil {
		return breez_sdk_spark.Session{}, err
	}
	// Decrypt session.Token here before returning it.
	return session, nil
}

func (s EncryptingSessionStore) SetSession(
	serviceIdentityKey string,
	session breez_sdk_spark.Session,
) error {
	// Encrypt session.Token here before persisting it.
	return s.inner.SetSession(serviceIdentityKey, session)
}

// `identity` is the wallet identity public key bytes, used to scope the store.
func WithSessionStore(
	config breez_sdk_spark.Config,
	seed breez_sdk_spark.Seed,
	identity []byte,
) (*breez_sdk_spark.SdkBuilder, error) {
	// Reuse one storage backend for both the SDK storage and the session store.
	backend := breez_sdk_spark.DefaultStorage("./.data")

	// Get the session store the backend provides, then wrap it to add encryption.
	inner, err := breez_sdk_spark.DefaultSessionStore(backend, config.Network, identity)
	if err != nil {
		return nil, err
	}
	sessionStore := EncryptingSessionStore{inner: inner}

	builder := breez_sdk_spark.NewSdkBuilder(config, seed)
	builder.WithStorageBackend(backend)
	builder.WithSessionStore(sessionStore)
	return builder, nil
}
```



**Developer note**

When wrapping the backend's store, pass the same storage backend to both `WithStorageBackend` and `DefaultSessionStore` so the session store shares the SDK's persistence. On the WASM binding `DefaultSessionStore` takes the storage config and the wallet identity public key (hex) instead of a backend.

**Note:** Not supported in Flutter.

## With Shared SDK Context

API docs: https://breez.github.io/spark-sdk/breez_sdk_spark/struct.SdkContext.html

An SDK Context bundles every process-shareable resource: the HTTP client (used for SSP GraphQL, chain service and LNURL), the gRPC channels to the Spark operators, the gRPC client to the Breez backend, and — optionally — a PostgreSQL or MySQL connection pool. By default each SDK builds its own. Server processes hosting many wallets at once can construct one SDK Context and pass it to every `SdkBuilder` so they reuse the same pooled clients instead of each opening fresh ones.

Construct one via `NewSharedSdkContext` and pass it to each `SdkBuilder` via `WithSharedContext`. Connections close when the last reference to the SDK Context is dropped; calling `Disconnect` on an SDK instance does not affect them.

The `ConnectionsPerOperator` setting on `SdkContextConfig` controls how many gRPC connections the context opens to each Spark operator:

- `None` — one connection per operator, multiplexed across every SDK sharing this context. The right choice for almost every deployment.
- `Some(n)` — opens `n` connections per operator and balances requests across them. Worth setting only if the single shared connection has become a bottleneck — for example, latency that climbs with throughput, or operators deployed behind an L7 load balancer where you want client-side fan-out across backend instances.

To route a context's pooled connections through a SOCKS5 proxy, set `Proxy` on `SdkContextConfig` as well as on each SDK's `Config`. See [SOCKS5 proxy](./proxy.md).

**Developer note**

All SDK instances sharing an SDK Context must be configured for the same network and operator pool, and must agree on `Proxy`. The user agent of the first SDK to construct the context is reused for all subsequent instances.

### Browser

The SDK Context's gRPC channel pooling is not effective in the browser. Browsers maintain a single HTTP/2 connection per origin and multiplex everything over it; the SDK cannot create or share more.

### Node.js

Node's global `fetch` (undici) negotiates HTTP/2 with the Spark operators automatically and opens additional connections per origin as needed, so most deployments need no tuning. If you do want to cap or expand the per-origin pool, configure undici globally before initialising the SDK:

```js
import { Agent, setGlobalDispatcher } from 'undici'
setGlobalDispatcher(new Agent({ connections: 8 }))
```

This affects every `fetch` in the process, including the SDK's gRPC-web traffic.
