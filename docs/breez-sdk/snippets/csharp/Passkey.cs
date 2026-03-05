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
            var wallet = await passkey.GetWallet(walletName: "personal");

            var config = BreezSdkSparkMethods.DefaultConfig(network: Network.Mainnet);
            var sdk = await BreezSdkSparkMethods.Connect(new ConnectRequest(
                config: config,
                seed: wallet.seed,
                storageDir: "./.data"
            ));
            // ANCHOR_END: connect-with-passkey
            return sdk;
        }

        async Task<List<string>> ListWalletNames()
        {
            // ANCHOR: list-wallet-names
            var prfProvider = new ExamplePasskeyPrfProvider();
            var relayConfig = new NostrRelayConfig(
                breezApiKey: "<breez api key>"
            );
            var passkey = new Passkey(prfProvider, relayConfig);

            // Query Nostr for wallet names associated with this passkey
            var walletNames = await passkey.ListWalletNames();

            foreach (var walletName in walletNames)
            {
                Console.WriteLine($"Found wallet: {walletName}");
            }
            // ANCHOR_END: list-wallet-names
            return walletNames;
        }

        async Task StoreWalletName()
        {
            // ANCHOR: store-wallet-name
            var prfProvider = new ExamplePasskeyPrfProvider();
            var relayConfig = new NostrRelayConfig(
                breezApiKey: "<breez api key>"
            );
            var passkey = new Passkey(prfProvider, relayConfig);

            // Publish the wallet name to Nostr for later discovery
            await passkey.StoreWalletName(walletName: "personal");
            // ANCHOR_END: store-wallet-name
        }
    }
}
