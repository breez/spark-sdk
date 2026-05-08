using Breez.Sdk.Spark;

namespace BreezSdkSnippets
{
    // ANCHOR: implement-prf-provider
    // Implement the PrfProvider interface for custom logic if no built-in
    // PasskeyProvider ships for your target. Single API surface:
    // DeriveSeeds for derivation, CreatePasskey for registration,
    // IsSupported / CheckDomainAssociation for diagnostics. Single-salt
    // derivation is the trivial 1-element bulk case.
    class CustomPrfProvider : PrfProvider
    {
        public async Task<List<byte[]>> DeriveSeeds(List<string> salts)
        {
            // Call platform passkey API with PRF extension. Use the dual-salt
            // ceremony when the authenticator supports it (one OS prompt for
            // N salts) and fall back to per-salt assertions otherwise.
            // Returns one 32-byte PRF output per salt in input order.
            throw new NotImplementedException("Implement using WebAuthn or native passkey APIs");
        }

        public async Task<bool> IsSupported()
        {
            // Check if a PRF-capable authenticator is reachable from this
            // platform / device.
            throw new NotImplementedException("Check platform passkey availability");
        }

        public async Task<RegisteredCredential> CreatePasskey(CreatePasskeyRequest request)
        {
            // Register a new credential and return its ID + AAGUID + BE flag.
            throw new NotImplementedException("Implement registration via native passkey API");
        }

        public async Task<DomainAssociation> CheckDomainAssociation()
        {
            // Optional: verify the app's identity against the platform's
            // domain verification source (e.g., Apple AASA CDN, Google
            // Digital Asset Links). Custom providers without a verification
            // source return Skipped, which tells callers "proceed with
            // WebAuthn as normal".
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

            if (await prfProvider.IsSupported())
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
            var passkey = new PasskeyClient(prfProvider, null);

            // SignIn derives the wallet seed for an existing credential.
            // With bulk PRF on iOS+Android this is a single OS prompt that
            // derives master + label seeds in one ceremony.
            var response = await passkey.SignIn(new SignInRequest(
                label: "personal",
                extraSalts: new List<NamedSalt>()
            ));

            var config = BreezSdkSparkMethods.DefaultConfig(network: Network.Mainnet);
            var sdk = await BreezSdkSparkMethods.Connect(new ConnectRequest(
                config: config,
                seed: response.wallet.seed,
                storageDir: "./.data"
            ));
            // ANCHOR_END: connect-with-passkey
            return sdk;
        }

        async Task<BreezSdk> RegisterNewPasskey()
        {
            // ANCHOR: register-passkey
            // For a brand-new user with no existing passkey: Register()
            // creates the credential AND derives the wallet seed in one
            // orchestrated call. On iOS+Android this is 2 OS prompts total
            // (1 create + 1 dual-salt assert) thanks to the SDK's bulk-PRF
            // setup_wallet path.
            var prfProvider = new CustomPrfProvider();
            var passkey = new PasskeyClient(prfProvider, null);

            var response = await passkey.Register(new RegisterRequest(
                label: "personal",
                extraSalts: new List<NamedSalt>(),
                excludeCredentialIds: new List<byte[]>()
            ));

            var config = BreezSdkSparkMethods.DefaultConfig(network: Network.Mainnet);
            var sdk = await BreezSdkSparkMethods.Connect(new ConnectRequest(
                config: config,
                seed: response.wallet.seed,
                storageDir: "./.data"
            ));
            // ANCHOR_END: register-passkey
            return sdk;
        }

        async Task<List<string>> ListLabels()
        {
            // ANCHOR: list-labels
            var prfProvider = new CustomPrfProvider();
            var relayConfig = new NostrRelayConfig(
                breezApiKey: "<breez api key>"
            );
            var passkey = new PasskeyClient(prfProvider, relayConfig);

            // SignIn with no label runs in discovery mode: it derives the
            // master seed AND lists labels in the same ceremony, so a
            // follow-up ListLabels() reads from the cached identity for free.
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
            var passkey = new PasskeyClient(prfProvider, relayConfig);

            // For a new label on an existing identity, call SignIn(newLabel)
            // first to seed the SDK's identity cache via setup_wallet, THEN
            // StoreLabel uses the cached identity for free (1 OS prompt total).
            await passkey.StoreLabel(label: "personal");
            // ANCHOR_END: store-label
        }
    }
}
