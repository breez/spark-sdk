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

## Rust

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

## Swift

```swift
// Construct the seed using a mnemonic, entropy or passkey
let mnemonic = "<mnemonic words>"
let seed = Seed.mnemonic(mnemonic: mnemonic, passphrase: nil)

// Create the default config
var config = defaultConfig(network: Network.mainnet)
config.apiKey = "<breez api key>"

// Build the SDK using the config, seed and default storage
let builder = SdkBuilder(config: config, seed: seed)
await builder.withDefaultStorage(storageDir: "./.data")
// You can also pass your custom implementations:
// await builder.withStorage(<your storage implementation>)
// await builder.withChainService(<your chain service implementation>)
// await builder.withRestClient(<your rest client implementation>)
// await builder.withAccountNumber(accountNumber: <account number>)
// await builder.withPaymentObserver(<your payment observer implementation>)
let sdk = try await builder.build()
```

## Kotlin

```kotlin
// Construct the seed using a mnemonic, entropy or passkey
val mnemonic = "<mnemonic words>"
val seed = Seed.Mnemonic(mnemonic, null)

// Create the default config
val config = defaultConfig(Network.MAINNET)
config.apiKey = "<breez api key>"

try {
    // Build the SDK using the config, seed and default storage
    val builder = SdkBuilder(config, seed)
    builder.withDefaultStorage("./.data")
    // You can also pass your custom implementations:
    // builder.withStorage(<your storage implementation>)
    // builder.withChainService(<your chain service implementation>)
    // builder.withRestClient(<your rest client implementation>)
    // builder.withAccountNumber(<account number>)
    // builder.withPaymentObserver(<your payment observer implementation>)
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
// Create the default config
var config = BreezSdkSparkMethods.DefaultConfig(Network.Mainnet) with
{
    apiKey = "<breez api key>"
};
// Build the SDK using the config, seed and default storage
var builder = new SdkBuilder(config: config, seed: seed);
await builder.WithDefaultStorage(storageDir: "./.data");
// You can also pass your custom implementations:
// await builder.WithStorage(<your storage implementation>)
// await builder.WithChainService(<your chain service implementation>)
// await builder.WithRestClient(<your rest client implementation>)
// await builder.WithAccountNumber(<account number>)
// await builder.WithPaymentObserver(<your payment observer implementation>);
var sdk = await builder.Build();
```

## Javascript (Wasm)

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

## React Native

```typescript
// Construct the seed using a mnemonic, entropy or passkey
const mnemonic = '<mnemonics words>'
const seed = new Seed.Mnemonic({ mnemonic, passphrase: undefined })

// Create the default config
const config = defaultConfig(Network.Mainnet)
config.apiKey = '<breez api key>'

// Build the SDK using the config, seed and default storage
const builder = new SdkBuilder(config, seed)
await builder.withDefaultStorage(`${RNFS.DocumentDirectoryPath}/data`)
// You can also pass your custom implementations:
// await builder.withStorage(<your storage implementation>)
// await builder.withChainService(<your chain service implementation>)
// await builder.withRestClient(<your rest client implementation>)
// await builder.withAccountNumber(<account number>)
// await builder.withPaymentObserver(<your payment observer implementation>)
const sdk = await builder.build()
```

## Flutter

```dart
// Construct the seed using a mnemonic, entropy or passkey
String mnemonic = "<mnemonic words>";
final seed = Seed.mnemonic(mnemonic: mnemonic, passphrase: null);

// Create the default config
final config = defaultConfig(network: Network.mainnet)
    .copyWith(apiKey: "<breez api key>");

// Build the SDK using the config, seed and default storage
final builder = SdkBuilder(config: config, seed: seed);
builder.withDefaultStorage(storageDir: "./.data");
// You can also pass your custom implementations:
// builder.withRestChainService(
//     url: "https://custom.chain.service",
//     credentials: Credentials(
//         username: "service-username", password: "service-password"));
// builder.withAccountNumber(accountNumber: <account number>);
final sdk = await builder.build();
```

## Python

```python
# Construct the seed using a mnemonic, entropy or passkey
mnemonic = "<mnemonic words>"
seed = Seed.MNEMONIC(mnemonic=mnemonic, passphrase=None)
# Create the default config
config = default_config(network=Network.MAINNET)
config.api_key = "<breez api key>"
try:
    # Build the SDK using the config, seed and default storage
    builder = SdkBuilder(config=config, seed=seed)
    await builder.with_default_storage(storage_dir="./.data")
    # You can also pass your custom implementations:
    # await builder.with_storage(<your storage implementation>)
    # await builder.with_chain_service(<your chain service implementation>)
    # await builder.with_rest_client(<your rest client implementation>)
    # await builder.with_account_number(<account number>)
    # await builder.with_payment_observer(<your payment observer implementation>)
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

The SDK includes a PostgreSQL backend as an alternative to file-based storage. Build a storage config with `postgres_storage` and pass it to the builder via `with_storage_backend` — this configures PostgreSQL for all stores (storage, tree store, and token store), which is suitable for server-side deployments with horizontal scaling. To share a single connection pool across multiple SDK instances, supply the same config through a [Shared SDK Context](#with-shared-context); per-tenant scoping (rows isolated by seed identity) is preserved either way.

If your service owns the database schema and applies SDK-compatible migrations externally, set `run_migration` to `false` on the storage config. The SDK will trust the existing schema and skip all migration runs, including writes to schema migration tables.

**Note:** Not available for React Native or Flutter. For JavaScript/TypeScript, only supported in Node.js (not in the browser).

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

### Swift

```swift
// Construct the seed using a mnemonic, entropy or passkey
let mnemonic = "<mnemonic words>"
let seed = Seed.mnemonic(mnemonic: mnemonic, passphrase: nil)

// Create the default config
var config = defaultConfig(network: Network.mainnet)
config.apiKey = "<breez api key>"

// Configure PostgreSQL backend
// Connection string format: "host=localhost user=postgres password=secret dbname=spark"
// Or URI format: "postgres://user:password@host:port/dbname?sslmode=require"
// TLS: "sslmode=require" encrypts and verifies the server certificate
var postgresConfig = defaultPostgresStorageConfig(
    connectionString: "host=localhost user=postgres dbname=spark"
)
// Optionally pool settings can be adjusted. Some examples:
postgresConfig.maxPoolSize = UInt32(8) // Max connections in pool
postgresConfig.waitTimeoutSecs = UInt64(30) // Timeout waiting for connection
// If your service owns SDK-compatible schema migrations:
postgresConfig.runMigration = false

// Build the SDK with the PostgreSQL storage backend (storage, tree store,
// and token store). Per-tenant scoping (rows isolated by seed identity)
// is applied automatically.
let builder = SdkBuilder(config: config, seed: seed)
await builder.withStorageBackend(
    storage: try postgresStorage(config: postgresConfig))
let sdk = try await builder.build()
```

### Kotlin

```kotlin
// Construct the seed using a mnemonic, entropy or passkey
val mnemonic = "<mnemonic words>"
val seed = Seed.Mnemonic(mnemonic, null)

// Create the default config
val config = defaultConfig(Network.MAINNET)
config.apiKey = "<breez api key>"

// Configure PostgreSQL backend
// Connection string format: "host=localhost user=postgres password=secret dbname=spark"
// Or URI format: "postgres://user:password@host:port/dbname?sslmode=require"
// TLS: "sslmode=require" encrypts and verifies the server certificate
val postgresConfig = defaultPostgresStorageConfig("host=localhost user=postgres dbname=spark")
// Optionally pool settings can be adjusted. Some examples:
postgresConfig.maxPoolSize = 8u // Max connections in pool
postgresConfig.waitTimeoutSecs = 30u // Timeout waiting for connection
// If your service owns SDK-compatible schema migrations:
postgresConfig.runMigration = false

try {
    // Build the SDK with the PostgreSQL storage backend (storage, tree
    // store, and token store). Per-tenant scoping (rows isolated by
    // seed identity) is applied automatically.
    val builder = SdkBuilder(config, seed)
    builder.withStorageBackend(postgresStorage(postgresConfig))
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

// Create the default config
var config = BreezSdkSparkMethods.DefaultConfig(Network.Mainnet) with
{
    apiKey = "<breez api key>"
};

// Configure PostgreSQL backend
// Connection string format: "host=localhost user=postgres password=secret dbname=spark"
// Or URI format: "postgres://user:password@host:port/dbname?sslmode=require"
// TLS: "sslmode=require" encrypts and verifies the server certificate
var postgresConfig = BreezSdkSparkMethods.DefaultPostgresStorageConfig(
    connectionString: "host=localhost user=postgres dbname=spark"
);
// Optionally pool settings can be adjusted. Some examples:
postgresConfig = postgresConfig with
{
    maxPoolSize = 8u,        // Max connections in pool
    waitTimeoutSecs = 30ul,  // Timeout waiting for connection
    // If your service owns SDK-compatible schema migrations:
    runMigration = false
};

// Build the SDK with the PostgreSQL storage backend (storage, tree
// store, and token store). Per-tenant scoping (rows isolated by
// seed identity) is applied automatically.
var builder = new SdkBuilder(config: config, seed: seed);
await builder.WithStorageBackend(
    storage: BreezSdkSparkMethods.PostgresStorage(postgresConfig)
);
var sdk = await builder.Build();
```

### Javascript (Wasm)

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

### Python

```python
async def init_sdk_postgres():
    # Construct the seed using a mnemonic, entropy or passkey
    mnemonic = "<mnemonic words>"
    seed = Seed.MNEMONIC(mnemonic=mnemonic, passphrase=None)

    # Create the default config
    config = default_config(network=Network.MAINNET)
    config.api_key = "<breez api key>"

    # Configure PostgreSQL storage
    # Connection string format: "host=localhost user=postgres password=secret dbname=spark"
    # Or URI format: "postgres://user:password@host:port/dbname?sslmode=require"
    # TLS: "sslmode=require" encrypts and verifies the server certificate
    postgres_config = default_postgres_storage_config(
        connection_string="host=localhost user=postgres dbname=spark"
    )
    # Optionally pool settings can be adjusted. Some examples:
    postgres_config.max_pool_size = 8  # Max connections in pool
    postgres_config.wait_timeout_secs = 30  # Timeout waiting for connection
    # If your service owns SDK-compatible schema migrations:
    postgres_config.run_migration = False

    try:
        # Build the SDK with the PostgreSQL storage backend (storage, tree
        # store, and token store). Per-tenant scoping (rows isolated by seed
        # identity) is applied automatically.
        builder = SdkBuilder(config=config, seed=seed)
        await builder.with_storage_backend(
            storage=postgres_storage(config=postgres_config)
        )
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

TLS is controlled by the `sslmode` connection-string parameter. For production, set `sslmode=require`: it encrypts the connection and verifies the server certificate. `verify-ca` and `verify-full` are also supported, and `no-verify` is the explicit opt-in for TLS without certificate verification (for example, a self-signed certificate you cannot add to a trust store). When `sslmode` is absent, TLS is used when the server supports it and is always verified; the exception is JavaScript/TypeScript on Node.js, where an absent `sslmode` means no TLS. Servers using a private CA are trusted via `root_ca_pem` on the storage config, or on Node.js via the `sslrootcert=<path>` URI parameter or Node's trust store (for example, the `NODE_EXTRA_CA_CERTS` environment variable). `verify-ca` performs chain verification without a hostname check and requires a pinned CA: `root_ca_pem` on the storage config, or on Node.js the `sslrootcert=<path>` URI parameter. Use it when hostname verification cannot succeed, such as connecting to the server by IP address.

Sharing the same PostgreSQL database with multiple SDK instances is incompatible with real-time sync. See [Real-time sync server URL](./config.md#real-time-sync-server-url) for how to disable it.

The PostgreSQL tree store can use the same or a separate PostgreSQL database as the PostgreSQL storage. The tree store uses its own set of tables prefixed with `tree_`.

## With MySQL Backend

API docs: https://breez.github.io/spark-sdk/breez_sdk_spark/struct.SdkBuilder.html#method.with_storage_backend

The SDK includes a MySQL backend (MySQL 8.0+) as an alternative to file-based storage. Build a storage config with `mysql_storage` and pass it to the builder via `with_storage_backend` — this configures MySQL for all stores (storage, tree store, and token store), which is suitable for server-side deployments with horizontal scaling. To share a single connection pool across multiple SDK instances, supply the same config through a [Shared SDK Context](#with-shared-context); per-tenant scoping (rows isolated by seed identity) is preserved either way.

If your service owns the database schema and applies SDK-compatible migrations externally, set `run_migration` to `false` on the storage config. The SDK will trust the existing schema and skip all migration runs, including writes to schema migration tables.

**Note:** Not available for React Native or Flutter. For JavaScript/TypeScript, only supported in Node.js (not in the browser).

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

### Swift

```swift
// Construct the seed using a mnemonic, entropy or passkey
let mnemonic = "<mnemonic words>"
let seed = Seed.mnemonic(mnemonic: mnemonic, passphrase: nil)

// Create the default config
var config = defaultConfig(network: Network.mainnet)
config.apiKey = "<breez api key>"

// Configure MySQL backend (MySQL 8.0+).
// Connection string format (URL only):
//   "mysql://user:password@host:3306/dbname?ssl-mode=required"
// TLS: "ssl-mode=required" encrypts and verifies the server certificate
var mysqlConfig = defaultMysqlStorageConfig(
    connectionString: "mysql://user:password@localhost:3306/spark"
)
// Optionally pool settings can be adjusted. Some examples:
mysqlConfig.maxPoolSize = UInt32(8) // Max connections in pool
mysqlConfig.recycleTimeoutSecs = UInt64(60) // Recycle idle connections after this many seconds
// Provide a custom CA certificate when the server uses a private CA:
// mysqlConfig.rootCaPem = "-----BEGIN CERTIFICATE-----\n..."

// Build the SDK with the MySQL storage backend (storage, tree store, and
// token store). Per-tenant scoping (rows isolated by seed identity) is
// applied automatically.
let builder = SdkBuilder(config: config, seed: seed)
await builder.withStorageBackend(
    storage: try mysqlStorage(config: mysqlConfig))
let sdk = try await builder.build()
```

### Kotlin

```kotlin
// Construct the seed using a mnemonic, entropy or passkey
val mnemonic = "<mnemonic words>"
val seed = Seed.Mnemonic(mnemonic, null)

// Create the default config
val config = defaultConfig(Network.MAINNET)
config.apiKey = "<breez api key>"

// Configure MySQL backend (MySQL 8.0+).
// Connection string format (URL only):
//   "mysql://user:password@host:3306/dbname?ssl-mode=required"
// TLS: "ssl-mode=required" encrypts and verifies the server certificate
val mysqlConfig = defaultMysqlStorageConfig("mysql://user:password@localhost:3306/spark")
// Optionally pool settings can be adjusted. Some examples:
mysqlConfig.maxPoolSize = 8u // Max connections in pool
mysqlConfig.recycleTimeoutSecs = 60u // Recycle idle connections after this many seconds

// Provide a custom CA certificate when the server uses a private CA:
// mysqlConfig.rootCaPem = "-----BEGIN CERTIFICATE-----\n..."

try {
    // Build the SDK with the MySQL storage backend (storage, tree
    // store, and token store). Per-tenant scoping (rows isolated by
    // seed identity) is applied automatically.
    val builder = SdkBuilder(config, seed)
    builder.withStorageBackend(mysqlStorage(mysqlConfig))
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

// Create the default config
var config = BreezSdkSparkMethods.DefaultConfig(Network.Mainnet) with
{
    apiKey = "<breez api key>"
};

// Configure MySQL backend (MySQL 8.0+).
// Connection string format (URL only):
//   "mysql://user:password@host:3306/dbname?ssl-mode=required"
// TLS: "ssl-mode=required" encrypts and verifies the server certificate
var mysqlConfig = BreezSdkSparkMethods.DefaultMysqlStorageConfig(
    connectionString: "mysql://user:password@localhost:3306/spark"
);
// Optionally pool settings can be adjusted. Some examples:
mysqlConfig = mysqlConfig with
{
    maxPoolSize = 8u,             // Max connections in pool
    recycleTimeoutSecs = 60ul     // Recycle idle connections after this many seconds
};

// Provide a custom CA certificate when the server uses a private CA:
// mysqlConfig = mysqlConfig with { rootCaPem = "-----BEGIN CERTIFICATE-----\n..." };

// Build the SDK with the MySQL storage backend (storage, tree
// store, and token store). Per-tenant scoping (rows isolated by
// seed identity) is applied automatically.
var builder = new SdkBuilder(config: config, seed: seed);
await builder.WithStorageBackend(
    storage: BreezSdkSparkMethods.MysqlStorage(mysqlConfig)
);
var sdk = await builder.Build();
```

### Javascript (Wasm)

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

### Python

```python
async def init_sdk_mysql():
    # Construct the seed using a mnemonic, entropy or passkey
    mnemonic = "<mnemonic words>"
    seed = Seed.MNEMONIC(mnemonic=mnemonic, passphrase=None)

    # Create the default config
    config = default_config(network=Network.MAINNET)
    config.api_key = "<breez api key>"

    # Configure MySQL backend (MySQL 8.0+).
    # Connection string format (URL only):
    #   "mysql://user:password@host:3306/dbname?ssl-mode=required"
    # TLS: "ssl-mode=required" encrypts and verifies the server certificate
    mysql_config = default_mysql_storage_config(
        connection_string="mysql://user:password@localhost:3306/spark"
    )
    # Optionally pool settings can be adjusted. Some examples:
    mysql_config.max_pool_size = 8  # Max connections in pool
    mysql_config.recycle_timeout_secs = 60  # Recycle idle connections after this many seconds
    # Provide a custom CA certificate when the server uses a private CA:
    # mysql_config.root_ca_pem = "-----BEGIN CERTIFICATE-----\n..."

    try:
        # Build the SDK with the MySQL storage backend (storage, tree store,
        # and token store). Per-tenant scoping (rows isolated by seed identity)
        # is applied automatically.
        builder = SdkBuilder(config=config, seed=seed)
        await builder.with_storage_backend(
            storage=mysql_storage(config=mysql_config)
        )
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

MySQL only accepts URL-form connection strings (`mysql://user:password@host:3306/dbname`); the key=value form supported by PostgreSQL is not available. TLS is controlled by the `ssl-mode` URL parameter, with the same spellings on every platform: `required` (recommended for production) and `verify_identity` verify the server certificate chain and hostname, `verify_ca` verifies the chain only, and `no-verify` is the explicit opt-in for TLS without certificate verification. An absent `ssl-mode` means no TLS. Servers using a private CA are trusted via `root_ca_pem` on the storage config, or on JavaScript/TypeScript (Node.js) via the `ssl-ca=<path>` URL parameter or Node's trust store (for example, the `NODE_EXTRA_CA_CERTS` environment variable). `verify_ca` performs chain verification without a hostname check and requires a pinned CA: `root_ca_pem` on the storage config, or on Node.js the `ssl-ca=<path>` URL parameter. Use it when hostname verification cannot succeed, such as connecting to the server by IP address.

Sharing the same MySQL database with multiple SDK instances is incompatible with real-time sync. See [Real-time sync server URL](./config.md#real-time-sync-server-url) for how to disable it.

The MySQL tree store can use the same or a separate MySQL database as the MySQL storage. The tree store uses its own set of tables prefixed with `tree_`.

## With Chain Service

API docs: https://breez.github.io/spark-sdk/breez_sdk_spark/struct.SdkBuilder.html#method.with_chain_service

The SDK provides a default Bitcoin Chain Service implementation. If you want to use your own, you can provide it either by using [With REST Chain Service](#with-rest-chain-service) or by implementing the Bitcoin Chain Service interface.

## With REST Chain Service

API docs: https://breez.github.io/spark-sdk/breez_sdk_spark/struct.SdkBuilder.html#method.with_rest_chain_service

The SDK provides a default Bitcoin Chain Service implementation. If you want to use your own, you can provide it either by using [With Chain Service](#with-chain-service) or by providing a URL and optional credentials.

### Rust

```rust
let url = "<your REST chain service URL>".to_string();
let chain_api_type = ChainApiType::MempoolSpace;
let optional_credentials = Credentials {
    username: "<username>".to_string(),
    password: "<password>".to_string(),
};
builder.with_rest_chain_service(url, chain_api_type, Some(optional_credentials))
```

### Swift

```swift
let url = "<your REST chain service URL>"
let chainApiType = ChainApiType.mempoolSpace
let optionalCredentials = Credentials(
    username: "<username>",
    password: "<password>"
)
await builder.withRestChainService(
    url: url,
    apiType: chainApiType,
    credentials: optionalCredentials
)
```

### Kotlin

```kotlin
val url = "<your REST chain service URL>"
val chainApiType = ChainApiType.MEMPOOL_SPACE
val optionalCredentials = Credentials(
    username = "<username>",
    password = "<password>"
)
builder.withRestChainService(
    url = url,
    apiType = chainApiType,
    credentials = optionalCredentials
)
```

### C#

```csharp
var url = "<your REST chain service URL>";
var chainApiType = ChainApiType.MempoolSpace;
var optionalCredentials = new Credentials(
    username: "<username>",
    password: "<password>"
);
await builder.WithRestChainService(
    url: url,
    apiType: chainApiType,
    credentials: optionalCredentials
);
```

### Javascript (Wasm)

```typescript
const url = '<your REST chain service URL>'
const chainApiType = 'mempoolSpace'
const optionalCredentials: Credentials = {
  username: '<username>',
  password: '<password>'
}
builder = builder.withRestChainService(url, chainApiType, optionalCredentials)
```

### React Native

```typescript
const url = '<your REST chain service URL>'
const chainApiType = ChainApiType.MempoolSpace
const optionalCredentials: Credentials = {
  username: '<username>',
  password: '<password>'
}
await builder.withRestChainService(url, chainApiType, optionalCredentials)
```

### Flutter

```dart
String url = "<your REST chain service URL>";
var chainApiType = ChainApiType.mempoolSpace;
var optionalCredentials = Credentials(
  username: "<username>",
  password: "<password>",
);
builder.withRestChainService(
  url: url,
  apiType: chainApiType,
  credentials: optionalCredentials,
);
```

### Python

```python
url = "<your REST chain service URL>"
chain_api_type = ChainApiType.MEMPOOL_SPACE
optional_credentials = Credentials(
    username="<username>",
    password="<password>",
)
await builder.with_rest_chain_service(
    url=url,
    api_type=chain_api_type,
    credentials=optional_credentials,
)
```

### Go

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

### Rust

```rust
let account_number = 21;
builder.with_account_number(account_number)
```

### Swift

```swift
let accountNumber = UInt32(21)
await builder.withAccountNumber(accountNumber: accountNumber)
```

### Kotlin

```kotlin
val accountNumber = 21u
builder.withAccountNumber(accountNumber)
```

### C#

```csharp
var accountNumber = 21u;
await builder.WithAccountNumber(accountNumber);
```

### Javascript (Wasm)

```typescript
builder = builder.withAccountNumber(21)
```

### React Native

```typescript
await builder.withAccountNumber(21)
```

### Flutter

```dart
var accountNumber = 21;
builder.withAccountNumber(accountNumber: accountNumber);
```

### Python

```python
account_number = 21
await builder.with_account_number(account_number=account_number)
```

### Go

```go
accountNumber := uint32(21)
builder.WithAccountNumber(accountNumber)
```



## With Payment Observer

API docs: https://breez.github.io/spark-sdk/breez_sdk_spark/struct.SdkBuilder.html#method.with_payment_observer

By implementing the Payment Observer interface you can be notified before a payment is sent. It includes information about the provisional payment including the payment ID, amount to be sent (in satoshis or token base units) and payment details based on the payment method.

**Note:** Flutter currently does not support this.

### Rust

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

### Swift

```swift
class ExamplePaymentObserver: PaymentObserver {
    func beforeSend(payments: [ProvisionalPayment]) async {
        for payment in payments {
            print("About to send payment: \(payment.paymentId) of amount \(payment.amount)")
        }
    }

    func afterSend(updates: [PaymentIdUpdate]) async {
        for update in updates {
            print("Token tx broadcast: \(update.provisionalPaymentId) -> \(update.finalPaymentId)")
        }
    }
}

func withPaymentObserver(builder: SdkBuilder) async {
    let paymentObserver = ExamplePaymentObserver()
    await builder.withPaymentObserver(paymentObserver: paymentObserver)
}
```

### Kotlin

```kotlin
class ExamplePaymentObserver : PaymentObserver {
    override suspend fun beforeSend(payments: List<ProvisionalPayment>) {
        for (payment in payments) {
            // Log.v("PaymentObserver", "About to send payment:
            // ${payment.paymentId} of amount ${payment.amount}")
        }
    }

    override suspend fun afterSend(updates: List<PaymentIdUpdate>) {
        for (update in updates) {
            // Log.v("PaymentObserver", "Token tx broadcast:
            // ${update.provisionalPaymentId} -> ${update.finalPaymentId}")
        }
    }
}

suspend fun withPaymentObserver(builder: SdkBuilder) {
    val paymentObserver = ExamplePaymentObserver()
    builder.withPaymentObserver(paymentObserver)
}
```

### C#

```csharp
class ExamplePaymentObserver : PaymentObserver
{
    public async Task BeforeSend(ProvisionalPayment[] payments)
    {
        foreach (var payment in payments)
        {
            Console.WriteLine($"About to send payment {payment.paymentId} " +
                              $"of amount {payment.amount}");
        }
    }

    public async Task AfterSend(PaymentIdUpdate[] updates)
    {
        foreach (var update in updates)
        {
            Console.WriteLine($"Token tx broadcast: {update.provisionalPaymentId} -> " +
                              $"{update.finalPaymentId}");
        }
    }
}

async Task WithPaymentObserver(SdkBuilder builder)
{
    var paymentObserver = new ExamplePaymentObserver();
    await builder.WithPaymentObserver(paymentObserver);
}
```

### Javascript (Wasm)

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

### React Native

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

const exampleWithPaymentObserver = async (builder: SdkBuilder) => {
  const paymentObserver = new ExamplePaymentObserver()
  await builder.withPaymentObserver(paymentObserver)
}
```

### Flutter

```dart

```

### Python

```python
class ExamplePaymentObserver(PaymentObserver):
    async def before_send(self, payments: typing.List[ProvisionalPayment]):
        for payment in payments:
            logging.debug(f"About to send payment {payment.payment_id} of amount {payment.amount}")

    async def after_send(self, updates: typing.List[PaymentIdUpdate]):
        for update in updates:
            logging.debug(
                f"Token tx broadcast: {update.provisional_payment_id} -> {update.final_payment_id}"
            )


async def with_payment_observer(builder: SdkBuilder):
    payment_observer = ExamplePaymentObserver()
    await builder.with_payment_observer(payment_observer=payment_observer)
```

### Go

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

Use `with_session_store` to provide your own `SessionStore`. This can be a completely different persistence layer, or a decorator that wraps the backend's own store to transform tokens on read and write while keeping its persistence: fetch the backend's store with `default_session_store`, then intercept `get_session` and `set_session`.

At-rest encryption is one such transform (the SDK does not encrypt tokens itself), shown below: encrypt the token in `set_session` and decrypt it in `get_session`.

### Rust

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

### Swift

```swift
class EncryptingSessionStore: SessionStore {
    let inner: SessionStore

    init(inner: SessionStore) {
        self.inner = inner
    }

    func getSession(serviceIdentityKey: String) async throws -> Session {
        let session = try await inner.getSession(serviceIdentityKey: serviceIdentityKey)
        // Decrypt session.token here before returning it.
        return session
    }

    func setSession(serviceIdentityKey: String, session: Session) async throws {
        // Encrypt session.token here before persisting it.
        try await inner.setSession(serviceIdentityKey: serviceIdentityKey, session: session)
    }
}

// `identity` is the wallet identity public key bytes, used to scope the store.
func withSessionStore(config: Config, seed: Seed, identity: Data) async throws -> SdkBuilder {
    // Reuse one storage backend for both the SDK storage and the session store.
    let backend = defaultStorage(storageDir: "./.data")

    // Get the session store the backend provides, then wrap it to add encryption.
    let inner = try await defaultSessionStore(
        backend: backend,
        network: config.network,
        identity: identity
    )
    let sessionStore = EncryptingSessionStore(inner: inner)

    let builder = SdkBuilder(config: config, seed: seed)
    await builder.withStorageBackend(storage: backend)
    await builder.withSessionStore(sessionStore: sessionStore)
    return builder
}
```

### Kotlin

```kotlin
class EncryptingSessionStore(
    private val inner: SessionStore,
) : SessionStore {
    override suspend fun getSession(serviceIdentityKey: String): Session {
        val session = inner.getSession(serviceIdentityKey)
        // Decrypt session.token here before returning it.
        return session
    }

    override suspend fun setSession(serviceIdentityKey: String, session: Session) {
        // Encrypt session.token here before persisting it.
        inner.setSession(serviceIdentityKey, session)
    }
}

// `identity` is the wallet identity public key bytes, used to scope the store.
suspend fun withSessionStore(
    config: breez_sdk_spark.Config,
    seed: Seed,
    identity: ByteArray,
): SdkBuilder {
    // Reuse one storage backend for both the SDK storage and the session store.
    val backend = defaultStorage("./.data")

    // Get the session store the backend provides, then wrap it to add encryption.
    val inner = defaultSessionStore(backend, config.network, identity)
    val sessionStore = EncryptingSessionStore(inner)

    val builder = SdkBuilder(config, seed)
    builder.withStorageBackend(backend)
    builder.withSessionStore(sessionStore)
    return builder
}
```

### C#

```csharp
class EncryptingSessionStore : SessionStore
{
    private readonly SessionStore inner;

    public EncryptingSessionStore(SessionStore inner)
    {
        this.inner = inner;
    }

    public async Task<Session> GetSession(string serviceIdentityKey)
    {
        var session = await inner.GetSession(serviceIdentityKey);
        // Decrypt session.token here before returning it.
        return session;
    }

    public async Task SetSession(string serviceIdentityKey, Session session)
    {
        // Encrypt session.token here before persisting it.
        await inner.SetSession(serviceIdentityKey, session);
    }
}

// `identity` is the wallet identity public key bytes, used to scope the store.
async Task<SdkBuilder> WithSessionStore(Config config, Seed seed, byte[] identity)
{
    // Reuse one storage backend for both the SDK storage and the session store.
    var backend = BreezSdkSparkMethods.DefaultStorage(storageDir: "./.data");

    // Get the session store the backend provides, then wrap it to add encryption.
    var inner = await BreezSdkSparkMethods.DefaultSessionStore(
        backend: backend,
        network: config.network,
        identity: identity
    );
    var sessionStore = new EncryptingSessionStore(inner);

    var builder = new SdkBuilder(config: config, seed: seed);
    await builder.WithStorageBackend(storage: backend);
    await builder.WithSessionStore(sessionStore: sessionStore);
    return builder;
}
```

### Javascript (Wasm)

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

### React Native

```typescript
class EncryptingSessionStore implements SessionStore {
  constructor (private readonly inner: SessionStore) {}

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

// `identity` is the wallet identity public key bytes, used to scope the store.
const exampleWithSessionStore = async (identity: ArrayBuffer): Promise<SdkBuilder> => {
  // Construct the seed using a mnemonic, entropy or passkey
  const mnemonic = '<mnemonic words>'
  const seed = new Seed.Mnemonic({ mnemonic, passphrase: undefined })

  // Create the default config
  const config = defaultConfig(Network.Mainnet)
  config.apiKey = '<breez api key>'

  // Reuse one storage backend for both the SDK storage and the session store.
  const backend = defaultStorage('./.data')
  // Get the session store the backend provides, then wrap it to add encryption.
  const inner = await defaultSessionStore(backend, config.network, identity)
  const sessionStore = new EncryptingSessionStore(inner)

  const builder = new SdkBuilder(config, seed)
  await builder.withStorageBackend(backend)
  await builder.withSessionStore(sessionStore)
  return builder
}
```

### Flutter

```dart

```

### Python

```python
class EncryptingSessionStore(SessionStore):
    def __init__(self, inner: SessionStore):
        self.inner = inner

    async def get_session(self, service_identity_key: str) -> Session:
        session = await self.inner.get_session(service_identity_key)
        # Decrypt session.token here before returning it.
        return session

    async def set_session(self, service_identity_key: str, session: Session) -> None:
        # Encrypt session.token here before persisting it.
        await self.inner.set_session(service_identity_key, session)


# `identity` is the wallet identity public key bytes, used to scope the store.
async def with_session_store(config: Config, seed: Seed, identity: bytes) -> SdkBuilder:
    # Reuse one storage backend for both the SDK storage and the session store.
    backend = default_storage(storage_dir="./.data")

    # Get the session store the backend provides, then wrap it to add encryption.
    inner = await default_session_store(
        backend=backend,
        network=config.network,
        identity=identity,
    )
    session_store = EncryptingSessionStore(inner)

    builder = SdkBuilder(config=config, seed=seed)
    await builder.with_storage_backend(storage=backend)
    await builder.with_session_store(session_store=session_store)
    return builder
```

### Go

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

---

Identifier casing: `get_info` here is `getInfo` in Swift, Kotlin, JavaScript, React Native and Flutter, and `GetInfo` in Go and C#. Enum variants: `SdkEvent::Synced` is `SdkEvent.SYNCED` in Python, `SdkEvent.synced` in Swift, `SdkEventSynced` in Go, and `SdkEvent.Synced` elsewhere.
