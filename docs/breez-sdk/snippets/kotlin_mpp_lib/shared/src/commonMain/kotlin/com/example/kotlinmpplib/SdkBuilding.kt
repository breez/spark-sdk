package com.example.kotlinmpplib

import breez_sdk_spark.*
class SdkBuilding {
    suspend fun initSdkAdvanced() {
        // ANCHOR: init-sdk-advanced
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
            // builder.withKeySet(<your key set type>, <use address index>, <account number>)
            // builder.withPaymentObserver(<your payment observer implementation>)
            val sdk = builder.build()
        } catch (e: Exception) {
            // handle error
        }
        // ANCHOR_END: init-sdk-advanced
    }

    suspend fun withRestChainService(builder: SdkBuilder) { 
        // ANCHOR: with-rest-chain-service
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
        // ANCHOR_END: with-rest-chain-service
    }

    suspend fun withKeySet(builder: SdkBuilder) {
        // ANCHOR: with-key-set
        val keySetType = KeySetType.DEFAULT
        val useAddressIndex = false
        val optionalAccountNumber = 21u
        
        val keySetConfig = KeySetConfig(
            keySetType = keySetType,
            useAddressIndex = useAddressIndex,
            accountNumber = optionalAccountNumber
        )
        
        builder.withKeySet(keySetConfig)
        // ANCHOR_END: with-key-set
    }

    // ANCHOR: with-payment-observer
    class ExamplePaymentObserver : PaymentObserver {
        override suspend fun beforeSend(payments: List<ProvisionalPayment>) {
            for (payment in payments) {
                // Log.v("PaymentObserver", "About to send payment: ${payment.paymentId} of amount ${payment.amount}")
            }
        }
    }

    suspend fun withPaymentObserver(builder: SdkBuilder) {
        val paymentObserver = ExamplePaymentObserver()
        builder.withPaymentObserver(paymentObserver)
    }
    // ANCHOR_END: with-payment-observer

    suspend fun initSdkPostgres() {
        // ANCHOR: init-sdk-postgres
        // Construct the seed using a mnemonic, entropy or passkey
        val mnemonic = "<mnemonic words>"
        val seed = Seed.Mnemonic(mnemonic, null)

        // Create the default config
        val config = defaultConfig(Network.MAINNET)
        config.apiKey = "<breez api key>"

        // Configure PostgreSQL backend
        // Connection string format: "host=localhost user=postgres password=secret dbname=spark"
        // Or URI format: "postgres://user:password@host:port/dbname?sslmode=require"
        val postgresConfig = defaultPostgresStorageConfig("host=localhost user=postgres dbname=spark")
        // Optionally pool settings can be adjusted. Some examples:
        postgresConfig.maxPoolSize = 8u // Max connections in pool
        postgresConfig.waitTimeoutSecs = 30u // Timeout waiting for connection
        // If your service owns SDK-compatible schema migrations:
        postgresConfig.runMigration = false

        // Construct the connection pool. The same pool can be passed to
        // multiple SdkBuilders to share connections across SDKs; per-tenant
        // scoping (rows isolated by seed identity) is preserved.
        val pool = createPostgresConnectionPool(postgresConfig)

        try {
            // Build the SDK with PostgreSQL backend (storage, tree store, and token store)
            val builder = SdkBuilder(config, seed)
            builder.withPostgresConnectionPool(pool)
            val sdk = builder.build()
        } catch (e: Exception) {
            // handle error
        }
        // ANCHOR_END: init-sdk-postgres
    }

    suspend fun initSdkMysql() {
        // ANCHOR: init-sdk-mysql
        // Construct the seed using a mnemonic, entropy or passkey
        val mnemonic = "<mnemonic words>"
        val seed = Seed.Mnemonic(mnemonic, null)

        // Create the default config
        val config = defaultConfig(Network.MAINNET)
        config.apiKey = "<breez api key>"

        // Configure MySQL backend (MySQL 8.0+).
        // Connection string format (URL only):
        //   "mysql://user:password@host:3306/dbname?ssl-mode=required"
        val mysqlConfig = defaultMysqlStorageConfig("mysql://user:password@localhost:3306/spark")
        // Optionally pool settings can be adjusted. Some examples:
        mysqlConfig.maxPoolSize = 8u // Max connections in pool
        mysqlConfig.recycleTimeoutSecs = 60u // Recycle idle connections after this many seconds

        // Construct the connection pool. The same pool can be passed to
        // multiple SdkBuilders to share connections across SDKs; per-tenant
        // scoping (rows isolated by seed identity) is preserved.
        val pool = createMysqlConnectionPool(mysqlConfig)

        try {
            // Build the SDK with MySQL backend (storage, tree store, and token store)
            val builder = SdkBuilder(config, seed)
            builder.withMysqlConnectionPool(pool)
            val sdk = builder.build()
        } catch (e: Exception) {
            // handle error
        }
        // ANCHOR_END: init-sdk-mysql
    }

    suspend fun initSdkServer() {
        // ANCHOR: init-sdk-server
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
        // ANCHOR_END: init-sdk-server
    }

    suspend fun serverModeRequestHandler(sdk: BreezSdk) {
        // ANCHOR: server-mode-request-handler
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
                )
            )
        )

        // Always disconnect at the end of the request lifecycle to flush
        // outstanding storage writes.
        sdk.disconnect()
        // ANCHOR_END: server-mode-request-handler
    }

    suspend fun serverModeProvisioning(sdk: BreezSdk) {
        // ANCHOR: server-mode-provisioning
        // One-time setup when a wallet is first registered. The client-mode
        // SDK would normally apply the private-mode preset itself on first
        // startup; server-mode SDKs do not, so opt in once here via
        // updateUserSettings.
        sdk.updateUserSettings(UpdateUserSettingsRequest(sparkPrivateModeEnabled = true))

        sdk.disconnect()
        // ANCHOR_END: server-mode-provisioning
    }

    suspend fun refundPendingConversions(sdk: BreezSdk) {
        // ANCHOR: refund-pending-conversions
        // The flashnet conversion refunder doesn't run in the background in
        // server mode. Call this from your own scheduler (e.g. once per
        // minute) to issue pending refunds for failed conversions.
        sdk.refundPendingConversions()
        // ANCHOR_END: refund-pending-conversions
    }
}
