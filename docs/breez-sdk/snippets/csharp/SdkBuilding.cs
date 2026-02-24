using Breez.Sdk.Spark;

namespace BreezSdkSnippets
{
    class SdkBuilding
    {
        async Task InitSdkAdvanced()
        {
            // ANCHOR: init-sdk-advanced
            // Construct the seed using mnemonic words or entropy bytes
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
            public async Task BeforeSend(List<ProvisionalPayment> payments)
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
            // Construct the seed using mnemonic words or entropy bytes
            var mnemonic = "<mnemonic words>";
            var seed = new Seed.Mnemonic(mnemonic: mnemonic, passphrase: null);

            // Create the default config
            var config = BreezSdkSparkMethods.DefaultConfig(Network.Mainnet) with
            {
                apiKey = "<breez api key>"
            };

            // Configure PostgreSQL storage
            // Connection string format: "host=localhost user=postgres password=secret dbname=spark"
            // Or URI format: "postgres://user:password@host:port/dbname?sslmode=require"
            var postgresConfig = BreezSdkSparkMethods.DefaultPostgresStorageConfig(
                connectionString: "host=localhost user=postgres dbname=spark"
            );
            // Optionally pool settings can be adjusted. Some examples:
            postgresConfig = postgresConfig with
            {
                maxPoolSize = 8u,        // Max connections in pool
                waitTimeoutSecs = 30ul   // Timeout waiting for connection
            };

            // Build the SDK with PostgreSQL storage
            var builder = new SdkBuilder(config: config, seed: seed);
            await builder.WithPostgresStorage(config: postgresConfig);
            var sdk = await builder.Build();
            // ANCHOR_END: init-sdk-postgres
        }
    }
}
