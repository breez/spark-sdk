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
    }
}
