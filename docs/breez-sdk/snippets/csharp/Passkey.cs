using Breez.Sdk.Spark;

namespace BreezSdkSnippets
{
    // ANCHOR: implement-prf-provider
    // Implement the PrfProvider interface for custom logic if no built-in
    // PasskeyProvider ships for your target. Three required methods:
    // DeriveSeeds for derivation, IsSupported for the capability probe;
    // CreatePasskey for registration is optional.
    class CustomPrfProvider : PrfProvider
    {
        public async Task<DeriveSeedsOutput> DeriveSeeds(DeriveSeedsRequest request)
        {
            throw new NotImplementedException("Implement using WebAuthn or native passkey APIs");
        }

        public async Task<bool> IsSupported()
        {
            throw new NotImplementedException("Check platform passkey availability");
        }

        public async Task<RegisteredCredential> CreatePasskey(byte[][] excludeCredentials)
        {
            // Register a new credential and return its ID, the WebAuthn
            // user.id the platform recorded (returned for host-side
            // correlation, never host-supplied), AAGUID, and BE flag.
            throw new NotImplementedException("Implement registration via native passkey API");
        }

        public async Task<DomainAssociation> CheckDomainAssociation()
        {
            return await Task.FromResult<DomainAssociation>(
                new DomainAssociation.Skipped("CustomPrfProvider does not verify domain association"));
        }

        // CredentialRegistry hooks: wire these to your app's stored
        // credential-ID set if you want the SDK to auto-merge known IDs
        // into allowCredentials / excludeCredentials. Custom providers
        // without a registry can return empty and treat the mutators as
        // no-ops.
        public async Task<byte[][]> GetKnownCredentialIds() =>
            await Task.FromResult(Array.Empty<byte[]>());

        public async Task RemoveKnownCredentialId(byte[] credentialId) =>
            await Task.CompletedTask;

        public async Task ClearKnownCredentialIds() =>
            await Task.CompletedTask;
    }
    // ANCHOR_END: implement-prf-provider

    class CheckAvailabilitySnippet
    {
        async Task CheckAvailability()
        {
            // ANCHOR: check-availability
            var prfProvider = new CustomPrfProvider();
            var passkey = new PasskeyClient(prfProvider, null, null);

            switch (await passkey.CheckAvailability())
            {
                case PasskeyAvailability.Available:
                    break;
                case PasskeyAvailability.PrfUnsupported:
                    break;
                case PasskeyAvailability.NotAssociated notAssociated:
                    Console.WriteLine($"Domain association failed (source={notAssociated.source}): {notAssociated.reason}");
                    break;
                case PasskeyAvailability.Skipped:
                    break;
            }
            // ANCHOR_END: check-availability
        }

        PasskeyClient SetupPasskeyClient()
        {
            // ANCHOR: setup-client
            var prfProvider = new CustomPrfProvider();
            return new PasskeyClient(prfProvider, "<breez api key>", null);
            // ANCHOR_END: setup-client
        }
    }

    class PasskeySnippets
    {
        async Task<BreezSdk> ConnectWithPasskey()
        {
            // ANCHOR: connect-with-passkey
            // Single-CTA onboarding: silent sign-in for a returning user,
            // fall-through to register on a fresh device. Internally pins
            // `preferImmediatelyAvailableCredentials = true` so the silent
            // attempt fast-fails (no UI) when no local credential exists;
            // only `CredentialNotFound` flips to register, all other errors
            // (cancel / timeout / configuration) propagate unchanged.
            var prfProvider = new CustomPrfProvider();
            var passkey = new PasskeyClient(prfProvider, null, null);

            var response = await passkey.ConnectWithPasskey(
                new ConnectWithPasskeyRequest(label: "personal", excludeCredentials: Array.Empty<byte[]>())
            );

            // registeredCredential doubles as the path discriminator:
            // non-null when a new credential was just registered (persist
            // credentialId for future excludeCredentials); null when
            // silent sign-in succeeded for an existing credential.
            if (response.registeredCredential is not null)
            {
                var persistedId = response.registeredCredential.credentialId;
            }

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
            var prfProvider = new CustomPrfProvider();
            var passkey = new PasskeyClient(prfProvider, null, null);

            var response = await passkey.Register(new RegisterRequest(label: "personal"));

            // Hosts SHOULD persist credential.credentialId (for excludeCredentials
            // bookkeeping) and credential.userId (for server-side correlation).
            // The SDK generates userId; it is never host-supplied.
            var _persistedCredentialId = response.credential.credentialId;
            var _persistedUserId = response.credential.userId;

            var config = BreezSdkSparkMethods.DefaultConfig(network: Network.Mainnet);
            var sdk = await BreezSdkSparkMethods.Connect(new ConnectRequest(
                config: config,
                seed: response.wallet.seed,
                storageDir: "./.data"
            ));
            // ANCHOR_END: register-passkey
            return sdk;
        }

        async Task<string[]> ListLabels()
        {
            var prfProvider = new CustomPrfProvider();
            var passkey = new PasskeyClient(prfProvider, "<breez api key>", null);
            // ANCHOR: list-labels
            var labels = await passkey.Labels().List();
            foreach (var label in labels)
            {
                Console.WriteLine($"Found label: {label}");
            }
            // ANCHOR_END: list-labels
            return labels;
        }

        async Task StoreLabel()
        {
            var prfProvider = new CustomPrfProvider();
            var passkey = new PasskeyClient(prfProvider, "<breez api key>", null);
            // ANCHOR: store-label
            await passkey.Labels().Store(label: "personal");
            // ANCHOR_END: store-label
        }


        async Task CheckDomain()
        {
            // ANCHOR: domain-association
            var prfProvider = new CustomPrfProvider();
            var result = await prfProvider.CheckDomainAssociation();

            switch (result)
            {
                case DomainAssociation.Associated:
                    break;
                case DomainAssociation.NotAssociated notAssociated:
                    Console.WriteLine($"Domain association failed (source={notAssociated.source}): {notAssociated.reason}");
                    return;
                case DomainAssociation.Skipped:
                    break;
            }
            // ANCHOR_END: domain-association
        }

        async Task<Wallet> RecoverFromAlreadyExists()
        {
            // ANCHOR: recover-already-exists
            var prfProvider = new CustomPrfProvider();
            var passkey = new PasskeyClient(prfProvider, null, null);

            try
            {
                var response = await passkey.Register(new RegisterRequest(
                    label: "personal",
                    excludeCredentials: new byte[][]
                    {
                        // app-persisted credential IDs from prior registrations
                    }
                ));
                return response.wallet;
            }
            catch (PrfProviderException.CredentialAlreadyExists)
            {
                var response = await passkey.SignIn(new SignInRequest(label: "personal"));
                return response.wallet;
            }
            // ANCHOR_END: recover-already-exists
        }

        async Task<SignInResponse> HandleTimeout()
        {
            // ANCHOR: handle-timeout
            var prfProvider = new CustomPrfProvider();
            var passkey = new PasskeyClient(prfProvider, null, null);

            try
            {
                return await passkey.SignIn(new SignInRequest(label: "personal"));
            }
            catch (PrfProviderException.UserTimedOut)
            {
                Console.WriteLine("Sign-in timed out: show \"Try Again\" UI.");
                throw;
            }
            // ANCHOR_END: handle-timeout
        }
    }
}
