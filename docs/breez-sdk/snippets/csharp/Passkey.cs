using Breez.Sdk.Spark;

namespace BreezSdkSnippets
{
    // ANCHOR: implement-prf-provider
    // Implement using platform-specific passkey APIs if the SDK does not ship a built-in provider for your target.
    class CustomPrfProvider : PrfProvider
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

        public async Task<DomainAssociation> CheckDomainAssociation()
        {
            // Optional: verify the app's identity against the platform's domain
            // verification source (e.g., Apple AASA CDN, Google Digital Asset Links).
            // Built-in providers do this automatically; custom providers that don't
            // have a platform cache to verify against return Skipped, which tells
            // callers "proceed with WebAuthn as normal".
            return await Task.FromResult<DomainAssociation>(
                new DomainAssociation.Skipped("CustomPrfProvider does not verify domain association"));
        }
    }
    // ANCHOR_END: implement-prf-provider

    class CheckAvailabilitySnippet
    {
        async Task CheckAvailability()
        {
            // ANCHOR: check-availability
            var prfProvider = new CustomPrfProvider();

            if (await prfProvider.IsPrfAvailable())
            {
                // Show passkey as primary option
            }
            else
            {
                // Fall back to mnemonic flow
            }
            // ANCHOR_END: check-availability
        }
    }

    class PasskeySnippets
    {
        async Task<BreezSdk> ConnectWithPasskey()
        {
            // ANCHOR: connect-with-passkey
            var prfProvider = new CustomPrfProvider();
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

        async Task<string[]> ListLabels()
        {
            // ANCHOR: list-labels
            var prfProvider = new CustomPrfProvider();
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
            var prfProvider = new CustomPrfProvider();
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
