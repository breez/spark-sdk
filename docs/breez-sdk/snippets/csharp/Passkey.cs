using Breez.Sdk.Spark;

namespace BreezSdkSnippets
{
    // ANCHOR: implement-prf-provider
    // In practice, implement using platform-specific passkey APIs.
    class ExamplePasskeyPrfProvider : PasskeyPrfProvider
    {
        public async Task<byte[]> DerivePrfSeed(string salt)
        {
            // Call platform passkey API with PRF extension
            // Returns 32-byte PRF output
            throw new NotImplementedException("Implement using WebAuthn or native passkey APIs");
        }

        public async Task<bool> IsPrfAvailable()
        {
            // Check if PRF-capable passkey exists
            throw new NotImplementedException("Check platform passkey availability");
        }
    }
    // ANCHOR_END: implement-prf-provider

    class PasskeySnippets
    {
        async Task<BreezSdk> ConnectWithPasskey()
        {
            // ANCHOR: connect-with-passkey
            var prfProvider = new ExamplePasskeyPrfProvider();
            var passkey = new Passkey(prfProvider, null);

            // Derive the wallet from the passkey (pass null for the default wallet)
            var wallet = await passkey.GetWallet(label: "personal");

            var config = BreezSdkSparkMethods.DefaultConfig(network: Network.Mainnet);
            var sdk = await BreezSdkSparkMethods.Connect(new ConnectRequest(
                config: config,
                seed: wallet.seed,
                storageDir: "./.data"
            ));
            // ANCHOR_END: connect-with-passkey
            return sdk;
        }

        async Task<List<string>> ListLabels()
        {
            // ANCHOR: list-labels
            var prfProvider = new ExamplePasskeyPrfProvider();
            var relayConfig = new NostrRelayConfig(
                breezApiKey: "<breez api key>"
            );
            var passkey = new Passkey(prfProvider, relayConfig);

            // Query Nostr for labels associated with this passkey
            var labels = await passkey.ListLabels();

            foreach (var label in labels)
            {
                Console.WriteLine($"Found label: {label}");
            }
            // ANCHOR_END: list-labels
            return labels;
        }

        async Task StoreLabel()
        {
            // ANCHOR: store-label
            var prfProvider = new ExamplePasskeyPrfProvider();
            var relayConfig = new NostrRelayConfig(
                breezApiKey: "<breez api key>"
            );
            var passkey = new Passkey(prfProvider, relayConfig);

            // Publish the label to Nostr for later discovery
            await passkey.StoreLabel(label: "personal");
            // ANCHOR_END: store-label
        }
    }
}
