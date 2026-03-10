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

    // Configure PostgreSQL storage
    // Connection string format: "host=localhost user=postgres password=secret dbname=spark"
    // Or URI format: "postgres://user:password@host:port/dbname?sslmode=require"
    var postgresConfig = defaultPostgresStorageConfig(
        connectionString: "host=localhost user=postgres dbname=spark"
    )
    // Optionally pool settings can be adjusted. Some examples:
    postgresConfig.maxPoolSize = UInt32(8) // Max connections in pool
    postgresConfig.waitTimeoutSecs = UInt64(30) // Timeout waiting for connection

    // Build the SDK with PostgreSQL storage
    let builder = SdkBuilder(config: config, seed: seed)
    await builder.withPostgresStorage(config: postgresConfig)
    let sdk = try await builder.build()
    // ANCHOR_END: init-sdk-postgres

    return sdk
}

func initSdkPostgresTreeStore() async throws -> BreezSdk {
    // ANCHOR: init-sdk-postgres-tree-store
    // Construct the seed using a mnemonic, entropy or passkey
    let mnemonic = "<mnemonic words>"
    let seed = Seed.mnemonic(mnemonic: mnemonic, passphrase: nil)

    // Create the default config
    var config = defaultConfig(network: Network.mainnet)
    config.apiKey = "<breez api key>"

    // Configure PostgreSQL storage
    let postgresConfig = defaultPostgresStorageConfig(
        connectionString: "host=localhost user=postgres dbname=spark"
    )

    // Configure PostgreSQL tree store
    // Can use the same or a different PostgreSQL database
    var treeStoreConfig = defaultPostgresStorageConfig(
        connectionString: "host=localhost user=postgres dbname=spark"
    )
    // Optionally pool settings can be adjusted. Some examples:
    treeStoreConfig.maxPoolSize = UInt32(8) // Max connections in pool
    treeStoreConfig.waitTimeoutSecs = UInt64(30) // Timeout waiting for connection

    // Build the SDK with PostgreSQL storage and tree store
    let builder = SdkBuilder(config: config, seed: seed)
    await builder.withPostgresStorage(config: postgresConfig)
    await builder.withPostgresTreeStore(config: treeStoreConfig)
    let sdk = try await builder.build()
    // ANCHOR_END: init-sdk-postgres-tree-store

    return sdk
}
