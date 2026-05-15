using Breez.Sdk.Spark;

namespace BreezSdkSnippets
{
    class SdkBuilding
    {
        async Task InitSdkAdvanced()
        {
            // ANCHOR: init-sdk-advanced
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
            // await builder.WithKeySet(<your key set type>, <use address index>, <account number>)
            // await builder.WithPaymentObserver(<your payment observer implementation>);
            var sdk = await builder.Build();
            // ANCHOR_END: init-sdk-advanced
        }

        async Task WithRestChainService(SdkBuilder builder)
        {
            // ANCHOR: with-rest-chain-service
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
            // ANCHOR_END: with-rest-chain-service
        }

        async Task WithKeySet(SdkBuilder builder)
        {
            // ANCHOR: with-key-set
            var keySetType = KeySetType.Default;
            var useAddressIndex = false;
            var optionalAccountNumber = 21u;

            var keySetConfig = new KeySetConfig(
                keySetType: keySetType,
                useAddressIndex: useAddressIndex,
                accountNumber: optionalAccountNumber
            );

            await builder.WithKeySet(keySetConfig);
            // ANCHOR_END: with-key-set
        }

        // ANCHOR: with-payment-observer
        class ExamplePaymentObserver : PaymentObserver
        {
            public async Task BeforeSend(ProvisionalPayment[] payments)
            {
                foreach (var payment in payments)
                {
                    Console.WriteLine($"About to send payment {payment.paymentId} of amount {payment.amount}");
                }
            }
        }

        async Task WithPaymentObserver(SdkBuilder builder)
        {
            var paymentObserver = new ExamplePaymentObserver();
            await builder.WithPaymentObserver(paymentObserver);
        }
        // ANCHOR_END: with-payment-observer

        async Task InitSdkPostgres()
        {
            // ANCHOR: init-sdk-postgres
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

            // Construct the connection pool. The same pool can be passed to
            // multiple SdkBuilders to share connections across SDKs; per-tenant
            // scoping (rows isolated by seed identity) is preserved.
            var pool = BreezSdkSparkMethods.CreatePostgresConnectionPool(config: postgresConfig);

            // Build the SDK with PostgreSQL backend (storage, tree store, and token store)
            var builder = new SdkBuilder(config: config, seed: seed);
            await builder.WithPostgresConnectionPool(pool: pool);
            var sdk = await builder.Build();
            // ANCHOR_END: init-sdk-postgres
        }

        async Task InitSdkMysql()
        {
            // ANCHOR: init-sdk-mysql
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
            var mysqlConfig = BreezSdkSparkMethods.DefaultMysqlStorageConfig(
                connectionString: "mysql://user:password@localhost:3306/spark"
            );
            // Optionally pool settings can be adjusted. Some examples:
            mysqlConfig = mysqlConfig with
            {
                maxPoolSize = 8u,             // Max connections in pool
                recycleTimeoutSecs = 60ul     // Recycle idle connections after this many seconds
            };

            // Construct the connection pool. The same pool can be passed to
            // multiple SdkBuilders to share connections across SDKs; per-tenant
            // scoping (rows isolated by seed identity) is preserved.
            var pool = BreezSdkSparkMethods.CreateMysqlConnectionPool(config: mysqlConfig);

            // Build the SDK with MySQL backend (storage, tree store, and token store)
            var builder = new SdkBuilder(config: config, seed: seed);
            await builder.WithMysqlConnectionPool(pool: pool);
            var sdk = await builder.Build();
            // ANCHOR_END: init-sdk-mysql
        }
    }
}
