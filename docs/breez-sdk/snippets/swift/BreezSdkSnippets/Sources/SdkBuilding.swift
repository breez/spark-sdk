import BreezSdkSpark

func initSdkAdvanced() async throws -> BreezSdk {
    // ANCHOR: init-sdk-advanced
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
    // await builder.withChainService(wrapChainService(<your chain service implementation>))
    // await builder.withRestClient(<your rest client implementation>)
    // await builder.withKeySet(<your key set type>, <use address index>, <account number>)
    // await builder.withPaymentObserver(<your payment observer implementation>)
    let sdk = try await builder.build()
    // ANCHOR_END: init-sdk-advanced

    return sdk
}

func withRestChainService(builder: SdkBuilder) async {
    // ANCHOR: with-rest-chain-service
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
    // ANCHOR_END: with-rest-chain-service
}

func withSharedRestChainService(builder: SdkBuilder) async {
    // ANCHOR: with-shared-rest-chain-service
    // Construct one chain service handle and reuse it across every SdkBuilder
    // — they share a single pooled HTTP client.
    let url = "<your REST chain service URL>"
    let chainApiType = ChainApiType.mempoolSpace
    let optionalCredentials = Credentials(
        username: "<username>",
        password: "<password>"
    )
    let chainService = newRestChainService(
        url: url,
        network: Network.mainnet,
        apiType: chainApiType,
        credentials: optionalCredentials
    )
    await builder.withChainService(chainService: chainService)
    // ANCHOR_END: with-shared-rest-chain-service
}

func withKeySet(builder: SdkBuilder) async {
    // ANCHOR: with-key-set
    let keySetType = KeySetType.default
    let useAddressIndex = false
    let optionalAccountNumber = UInt32(21)
    
    let config = KeySetConfig(
        keySetType: keySetType,
        useAddressIndex: useAddressIndex,
        accountNumber: optionalAccountNumber
    )
    
    await builder.withKeySet(config: config)
    // ANCHOR_END: with-key-set
}

// ANCHOR: with-payment-observer
class ExamplePaymentObserver: PaymentObserver {
    func beforeSend(payments: [ProvisionalPayment]) async {
        for payment in payments {
            print("About to send payment: \(payment.paymentId) of amount \(payment.amount)")
        }
    }
}

func withPaymentObserver(builder: SdkBuilder) async {
    let paymentObserver = ExamplePaymentObserver()
    await builder.withPaymentObserver(paymentObserver: paymentObserver)
}
// ANCHOR_END: with-payment-observer

func initSdkPostgres() async throws -> BreezSdk {
    // ANCHOR: init-sdk-postgres
    // Construct the seed using a mnemonic, entropy or passkey
    let mnemonic = "<mnemonic words>"
    let seed = Seed.mnemonic(mnemonic: mnemonic, passphrase: nil)

    // Create the default config
    var config = defaultConfig(network: Network.mainnet)
    config.apiKey = "<breez api key>"

    // Configure PostgreSQL backend
    // Connection string format: "host=localhost user=postgres password=secret dbname=spark"
    // Or URI format: "postgres://user:password@host:port/dbname?sslmode=require"
    var postgresConfig = defaultPostgresStorageConfig(
        connectionString: "host=localhost user=postgres dbname=spark"
    )
    // Optionally pool settings can be adjusted. Some examples:
    postgresConfig.maxPoolSize = UInt32(8) // Max connections in pool
    postgresConfig.waitTimeoutSecs = UInt64(30) // Timeout waiting for connection
    // If your service owns SDK-compatible schema migrations:
    postgresConfig.runMigration = false

    // Construct the connection pool. The same pool can be passed to
    // multiple SdkBuilders to share connections across SDKs; per-tenant
    // scoping (rows isolated by seed identity) is preserved.
    let pool = try createPostgresConnectionPool(config: postgresConfig)

    // Build the SDK with PostgreSQL backend (storage, tree store, and token store)
    let builder = SdkBuilder(config: config, seed: seed)
    await builder.withPostgresConnectionPool(pool: pool)
    let sdk = try await builder.build()
    // ANCHOR_END: init-sdk-postgres

    return sdk
}

func initSdkMysql() async throws -> BreezSdk {
    // ANCHOR: init-sdk-mysql
    // Construct the seed using a mnemonic, entropy or passkey
    let mnemonic = "<mnemonic words>"
    let seed = Seed.mnemonic(mnemonic: mnemonic, passphrase: nil)

    // Create the default config
    var config = defaultConfig(network: Network.mainnet)
    config.apiKey = "<breez api key>"

    // Configure MySQL backend (MySQL 8.0+).
    // Connection string format (URL only):
    //   "mysql://user:password@host:3306/dbname?ssl-mode=required"
    var mysqlConfig = defaultMysqlStorageConfig(
        connectionString: "mysql://user:password@localhost:3306/spark"
    )
    // Optionally pool settings can be adjusted. Some examples:
    mysqlConfig.maxPoolSize = UInt32(8) // Max connections in pool
    mysqlConfig.recycleTimeoutSecs = UInt64(60) // Recycle idle connections after this many seconds
    // Provide a custom CA certificate when using ssl-mode=verify_ca or verify_identity:
    // mysqlConfig.rootCaPem = "-----BEGIN CERTIFICATE-----\n..."

    // Construct the connection pool. The same pool can be passed to
    // multiple SdkBuilders to share connections across SDKs; per-tenant
    // scoping (rows isolated by seed identity) is preserved.
    let pool = try createMysqlConnectionPool(config: mysqlConfig)

    // Build the SDK with MySQL backend (storage, tree store, and token store)
    let builder = SdkBuilder(config: config, seed: seed)
    await builder.withMysqlConnectionPool(pool: pool)
    let sdk = try await builder.build()
    // ANCHOR_END: init-sdk-mysql

    return sdk
}
