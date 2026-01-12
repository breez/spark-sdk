using Breez.Sdk.Spark;

namespace BreezSdkSnippets
{
    // ANCHOR: implement-prf-provider
    // In practice, implement PRF provider using platform passkey APIs
    class ExamplePasskeyPrfProvider : PasskeyPrfProvider
    {
        public async Task<byte[]> DerivePrfSeed(string salt)
        {
            // Call platform passkey API with PRF extension
            // Returns 32-byte PRF output
            throw new NotImplementedException("Implement using platform passkey APIs");
        }

        public async Task<bool> IsPrfAvailable()
        {
            // Check if PRF-capable passkey exists
            throw new NotImplementedException("Check platform passkey availability");
        }
    }
    // ANCHOR_END: implement-prf-provider

    class SeedlessRestoreSnippets
    {
        async Task<Seed> CreateSeed()
        {
            // ANCHOR: create-seed
            var prfProvider = new ExamplePasskeyPrfProvider();
            var seedless = new SeedlessRestore(prfProvider, null);

            // Create a new seed with user-chosen salt
            // The salt is published to Nostr for later discovery
            var seed = await seedless.CreateSeed(salt: "personal");

            // Use the seed to initialize the SDK
            var config = BreezSdkSparkMethods.DefaultConfig(network: Network.Mainnet);
            var builder = new SdkBuilder(config: config, seed: seed);
            await builder.WithDefaultStorage(storageDir: "./.data");
            var sdk = await builder.Build();
            // ANCHOR_END: create-seed
            return seed;
        }

        async Task<List<string>> ListSalts()
        {
            // ANCHOR: list-salts
            var prfProvider = new ExamplePasskeyPrfProvider();
            var seedless = new SeedlessRestore(prfProvider, null);

            // Query Nostr for salts associated with this passkey
            var salts = await seedless.ListSalts();

            foreach (var salt in salts)
            {
                Console.WriteLine($"Found wallet: {salt}");
            }
            // ANCHOR_END: list-salts
            return salts;
        }

        async Task<Seed> RestoreSeed()
        {
            // ANCHOR: restore-seed
            var prfProvider = new ExamplePasskeyPrfProvider();
            var seedless = new SeedlessRestore(prfProvider, null);

            // Restore seed using a known salt
            var seed = await seedless.RestoreSeed(salt: "personal");

            // Use the seed to initialize the SDK
            var config = BreezSdkSparkMethods.DefaultConfig(network: Network.Mainnet);
            var builder = new SdkBuilder(config: config, seed: seed);
            await builder.WithDefaultStorage(storageDir: "./.data");
            var sdk = await builder.Build();
            // ANCHOR_END: restore-seed
            return seed;
        }
    }
}
