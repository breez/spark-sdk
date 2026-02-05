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

        async Task FetchBalance(BreezSdk sdk)
        {
            // ANCHOR: fetch-balance
            // ensureSynced: true will ensure the SDK is synced with the Spark network
            // before returning the balance
            var info = await sdk.GetInfo(request: new GetInfoRequest(ensureSynced: false));
            var identityPubkey = info.identityPubkey;
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
                switch (sdkEvent)
                {
                    case SdkEvent.Synced syncedEvent:
                        // Data has been synchronized with the network. When this event is received,
                        // it is recommended to refresh the payment list and wallet balance.
                        break;

                    case SdkEvent.UnclaimedDeposits unclaimedDepositsEvent:
                        // SDK was unable to claim some deposits automatically
                        var unclaimedDeposits = unclaimedDepositsEvent.unclaimedDeposits;
                        break;

                    case SdkEvent.ClaimedDeposits claimedDepositsEvent:
                        // Deposits were successfully claimed
                        var claimedDeposits = claimedDepositsEvent.claimedDeposits;
                        break;

                    case SdkEvent.PaymentSucceeded paymentSucceededEvent:
                        // A payment completed successfully
                        var payment = paymentSucceededEvent.payment;
                        break;

                    case SdkEvent.PaymentPending paymentPendingEvent:
                        // A payment is pending (waiting for confirmation)
                        var pendingPayment = paymentPendingEvent.payment;
                        break;

                    case SdkEvent.PaymentFailed paymentFailedEvent:
                        // A payment failed
                        var failedPayment = paymentFailedEvent.payment;
                        break;

                    case SdkEvent.Optimization optimizationEvent:
                        // An optimization event occurred
                        var optimization = optimizationEvent.optimizationEvent;
                        break;

                    default:
                        // Handle any future event types
                        break;
                }
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

        // ANCHOR: spark-status
        async Task GetSparkStatus()
        {
            var sparkStatus = await BreezSdkSparkMethods.GetSparkStatus();

            switch (sparkStatus.status)
            {
                case ServiceStatus.Operational:
                    Console.WriteLine("Spark is fully operational");
                    break;
                case ServiceStatus.Degraded:
                    Console.WriteLine("Spark is experiencing degraded performance");
                    break;
                case ServiceStatus.Partial:
                    Console.WriteLine("Spark is partially unavailable");
                    break;
                case ServiceStatus.Major:
                    Console.WriteLine("Spark is experiencing a major outage");
                    break;
            }

            Console.WriteLine($"Last updated: {sparkStatus.lastUpdated}");
        }
        // ANCHOR_END: spark-status

        // ANCHOR: disconnect
        async Task Disconnect(BreezSdk sdk)
        {
            await sdk.Disconnect();
        }
        // ANCHOR_END: disconnect
    }
}
