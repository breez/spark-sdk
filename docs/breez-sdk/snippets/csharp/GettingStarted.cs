using Breez.Sdk.Spark;

namespace BreezSdkSnippets
{
    class GettingStarted
    {
        async Task InitSdk()
        {
            // ANCHOR: init-sdk
            // Construct the seed using mnemonic words or entropy bytes
            var mnemonic = "<mnemonic words>";
            var seed = new Seed.Mnemonic(mnemonic: mnemonic, passphrase: null);
            // Create the default config
            var config = BreezSdkSparkMethods.DefaultConfig(Network.Mainnet) with
            {
                apiKey = "<breez api key>"
            };
            // Connect to the SDK using the simplified connect method
            var sdk = await BreezSdkSparkMethods.Connect(
                request: new ConnectRequest(
                    config: config,
                    seed: seed,
                    storageDir: "./.data"
                )
            );
            // ANCHOR_END: init-sdk
        }

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
            // await builder.WithRealTimeSyncStorage(<your real-time sync storage implementation>)
            // await builder.WithChainService(<your chain service implementation>)
            // await builder.WithRestClient(<your rest client implementation>)
            // await builder.WithKeySet(<your key set type>, <use address index>, <account number>)
            // await builder.WithPaymentObserver(<your payment observer implementation>)
            var sdk = await builder.Build();
            // ANCHOR_END: init-sdk-advanced
        }

        async Task FetchBalance(BreezSdk sdk)
        {
            // ANCHOR: fetch-balance
            // ensureSynced: true will ensure the SDK is synced with the Spark network
            // before returning the balance
            var info = await sdk.GetInfo(request: new GetInfoRequest(ensureSynced: false));
            var balanceSats = info.balanceSats;
            // ANCHOR_END: fetch-balance
        }

        // ANCHOR: logging
        class SdkLogger : Logger
        {
            public void Log(LogEntry l)
            {
                Console.WriteLine($"Received log [{l.level}]: {l.line}");
            }
        }

        void SetLogger(SdkLogger logger)
        {
            BreezSdkSparkMethods.InitLogging(logDir: null, appLogger: logger, logFilter: null);
        }
        // ANCHOR_END: logging

        // ANCHOR: add-event-listener
        class SdkListener : EventListener
        {
            public async Task OnEvent(SdkEvent sdkEvent)
            {
                Console.WriteLine($"Received event {sdkEvent}");
            }
        }

        async Task<string> AddEventListener(BreezSdk sdk, SdkListener listener)
        {
            var listenerId = await sdk.AddEventListener(listener: listener);
            return listenerId;
        }
        // ANCHOR_END: add-event-listener

        // ANCHOR: remove-event-listener
        async Task RemoveEventListener(BreezSdk sdk, string listenerId)
        {
            await sdk.RemoveEventListener(id: listenerId);
        }
        // ANCHOR_END: remove-event-listener

        // ANCHOR: disconnect
        async Task Disconnect(BreezSdk sdk)
        {
            await sdk.Disconnect();
        }
        // ANCHOR_END: disconnect
    }
}
