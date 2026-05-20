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
    // await builder.withChainService(<your chain service implementation>)
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

    func afterSend(updates: [PaymentIdUpdate]) async {
        for update in updates {
            print("Token tx broadcast: \(update.partialTxId) -> \(update.finalTxId)")
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

func initSdkServer() async throws -> BreezSdk {
    // ANCHOR: init-sdk-server
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
    // ANCHOR_END: init-sdk-server

    return sdk
}

func serverModeRequestHandler(sdk: BreezSdk) async throws -> String {
    // ANCHOR: server-mode-request-handler
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
                paymentHash: nil
            )
        ))

    // Always disconnect at the end of the request lifecycle to flush
    // outstanding storage writes.
    try await sdk.disconnect()
    // ANCHOR_END: server-mode-request-handler
    return response.paymentRequest
}

func serverModeProvisioning(sdk: BreezSdk) async throws {
    // ANCHOR: server-mode-provisioning
    // One-time setup when a wallet is first registered. The client-mode SDK
    // would normally apply the private-mode preset itself on first startup;
    // server-mode SDKs do not, so opt in once here via updateUserSettings.
    try await sdk.updateUserSettings(
        request: UpdateUserSettingsRequest(sparkPrivateModeEnabled: true))

    try await sdk.disconnect()
    // ANCHOR_END: server-mode-provisioning
}

func refundPendingConversions(sdk: BreezSdk) async throws {
    // ANCHOR: refund-pending-conversions
    // The flashnet conversion refunder doesn't run in the background in server
    // mode. Call this from your own scheduler (e.g. once per minute) to issue
    // pending refunds for failed conversions.
    try await sdk.refundPendingConversions()
    // ANCHOR_END: refund-pending-conversions
}
