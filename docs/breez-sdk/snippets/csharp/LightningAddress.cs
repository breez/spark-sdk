using Breez.Sdk.Spark;

namespace BreezSdkSnippets
{
    class LightningAddress
    {
        void ConfigureLightningAddress()
        {
            // ANCHOR: config-lightning-address
            var config = BreezSdkSparkMethods.DefaultConfig(Network.Mainnet) with
            {
                apiKey = "your-api-key",
                lnurlDomain = "yourdomain.com"
            };
            // ANCHOR_END: config-lightning-address
        }

        async Task<bool> CheckLightningAddressAvailability(BreezSdk sdk, string username)
        {
            username = "myusername";

            // ANCHOR: check-lightning-address
            var request = new CheckLightningAddressRequest(username: username);
            var isAvailable = await sdk.CheckLightningAddressAvailable(request);
            // ANCHOR_END: check-lightning-address
            return isAvailable;
        }

        async Task<LightningAddressInfo> RegisterLightningAddress(BreezSdk sdk, string username, string description)
        {
            username = "myusername";
            description = "My Lightning Address";

            // ANCHOR: register-lightning-address
            var request = new RegisterLightningAddressRequest(
                username: username,
                description: description
            );

            var addressInfo = await sdk.RegisterLightningAddress(request);
            var lightningAddress = addressInfo.lightningAddress;
            var lnurl = addressInfo.lnurl;
            // ANCHOR_END: register-lightning-address
            return addressInfo;
        }

        async Task GetLightningAddress(BreezSdk sdk)
        {
            // ANCHOR: get-lightning-address
            var addressInfoOpt = await sdk.GetLightningAddress();

            if (addressInfoOpt != null)
            {
                var lightningAddress = addressInfoOpt.lightningAddress;
                var username = addressInfoOpt.username;
                var description = addressInfoOpt.description;
                var lnurl = addressInfoOpt.lnurl;
            }
            // ANCHOR_END: get-lightning-address
        }

        async Task DeleteLightningAddress(BreezSdk sdk)
        {
            // ANCHOR: delete-lightning-address
            await sdk.DeleteLightningAddress();
            // ANCHOR_END: delete-lightning-address
        }

        async Task AccessSenderComment(BreezSdk sdk)
        {
            var paymentId = "<payment id>";
            var response = await sdk.GetPayment(new GetPaymentRequest(paymentId: paymentId));
            var payment = response.payment;

            // ANCHOR: access-sender-comment
            // Check if this is a lightning payment with LNURL receive metadata
            if (payment.details is PaymentDetails.Lightning lightningDetails)
            {
                var metadata = lightningDetails.lnurlReceiveMetadata;

                // Access the sender comment if present
                if (metadata?.senderComment != null)
                {
                    Console.WriteLine($"Sender comment: {metadata.senderComment}");
                }
            }
            // ANCHOR_END: access-sender-comment
        }

        async Task AccessNostrZap(BreezSdk sdk)
        {
            var paymentId = "<payment id>";
            var response = await sdk.GetPayment(new GetPaymentRequest(paymentId: paymentId));
            var payment = response.payment;

            // ANCHOR: access-nostr-zap
            // Check if this is a lightning payment with LNURL receive metadata
            if (payment.details is PaymentDetails.Lightning lightningDetails)
            {
                var metadata = lightningDetails.lnurlReceiveMetadata;

                if (metadata != null)
                {
                    // Access the Nostr zap request if present
                    if (metadata.nostrZapRequest != null)
                    {
                        // The nostrZapRequest is a JSON string containing the Nostr event (kind 9734)
                        Console.WriteLine($"Nostr zap request: {metadata.nostrZapRequest}");
                    }

                    // Access the Nostr zap receipt if present
                    if (metadata.nostrZapReceipt != null)
                    {
                        // The nostrZapReceipt is a JSON string containing the Nostr event (kind 9735)
                        Console.WriteLine($"Nostr zap receipt: {metadata.nostrZapReceipt}");
                    }
                }
            }
            // ANCHOR_END: access-nostr-zap
        }
    }
}
