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
}
