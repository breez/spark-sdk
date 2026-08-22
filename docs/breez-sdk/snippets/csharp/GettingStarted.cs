using Breez.Sdk.Spark;

namespace BreezSdkSnippets
{
    class GettingStarted
    {
        async Task InitSdk()
        {
            // ANCHOR: init-sdk
            // Construct the seed using a mnemonic, entropy or passkey
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

                    case SdkEvent.NewDeposits newDepositsEvent:
                        // Detected deposits, as DepositInfo. Only those with IsMature set
                        // have enough confirmations to be claimed. Show the rest as pending.
                        var newDeposits = newDepositsEvent.newDeposits;
                        break;

                    case SdkEvent.UnclaimedDeposits unclaimedDepositsEvent:
                        // Deposits the SDK could not claim. Each ClaimError says why,
                        // most often the fee exceeded the configured maximum.
                        var unclaimedDeposits = unclaimedDepositsEvent.unclaimedDeposits;
                        break;

                    case SdkEvent.ClaimedDeposits claimedDepositsEvent:
                        // Deposits claimed into the wallet. The resulting payment
                        // arrives separately as its own event.
                        var claimedDeposits = claimedDepositsEvent.claimedDeposits;
                        break;

                    case SdkEvent.PaymentSucceeded paymentSucceededEvent:
                        // A payment completed. The cached balance is already refreshed,
                        // so GetInfo returns the new value.
                        var payment = paymentSucceededEvent.payment;
                        break;

                    case SdkEvent.PaymentPending paymentPendingEvent:
                        // A payment is awaiting confirmation. It arrives again as
                        // succeeded or failed once it settles.
                        var pendingPayment = paymentPendingEvent.payment;
                        break;

                    case SdkEvent.PaymentFailed paymentFailedEvent:
                        // A payment failed. payment.Details carries the method-specific
                        // context to show the user.
                        var failedPayment = paymentFailedEvent.payment;
                        break;

                    case SdkEvent.AutoOptimization optimizationEvent:
                        // Background optimizer progress: started, round completed, or a
                        // terminal outcome. Manual OptimizeLeaves calls do not emit these.
                        var optimization = optimizationEvent.optimizationEvent;
                        break;

                    case SdkEvent.LightningAddressChanged lightningAddressChangedEvent:
                        // The lightning address changed on another device. Unset when the
                        // address was deleted.
                        var lightningAddress = lightningAddressChangedEvent.lightningAddress;
                        break;

                    case SdkEvent.UnilateralExitStateChanged unilateralExitStateChangedEvent:
                        // The unilateral exit state changed, so a previously exported
                        // one is now out of date. Export it again.
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
                case ServiceStatus.Unknown:
                    Console.WriteLine("Spark status is unknown");
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
