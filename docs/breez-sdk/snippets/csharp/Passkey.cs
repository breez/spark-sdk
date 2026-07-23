using Breez.Sdk.Spark;

namespace BreezSdkSnippets
{
    // ANCHOR: implement-prf-provider
    // Implement PrfProvider for a custom authenticator (hardware key, FIDO2,
    // file-backed). Only DeriveSeeds and IsSupported are required.
    class CustomPrfProvider : PrfProvider
    {
        public async Task<DeriveSeedsOutput> DeriveSeeds(DeriveSeedsRequest request)
        {
            // Return one 32-byte PRF output per salt, in input order.
            throw new NotImplementedException("Implement using WebAuthn or native passkey APIs");
        }

        public async Task<bool> IsSupported()
        {
            throw new NotImplementedException("Check platform passkey availability");
        }

        public async Task<PasskeyCredential> CreatePasskey(byte[][] excludeCredentials)
        {
            // Register a credential and return its ID plus attestation.
            throw new NotImplementedException("Implement registration via native passkey API");
        }

        public async Task<DomainAssociation> CheckDomainAssociation()
        {
            return await Task.FromResult<DomainAssociation>(
                new DomainAssociation.Skipped("CustomPrfProvider does not verify domain association"));
        }

    }
    // ANCHOR_END: implement-prf-provider

    class CheckAvailabilitySnippet
    {
        async Task CheckAvailability()
        {
            var prfProvider = new CustomPrfProvider();
            var passkey = new PasskeyClient(prfProvider, null, null);

            // ANCHOR: check-availability
            switch (await passkey.CheckAvailability())
            {
                case PasskeyAvailability.Available:
                    break;
                case PasskeyAvailability.PrfUnsupported:
                    break;
                case PasskeyAvailability.NotAssociated notAssociated:
                    Console.WriteLine($"Domain association failed (source={notAssociated.source}): " +
                                      $"{notAssociated.reason}");
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
            var prfProvider = new CustomPrfProvider();
            var passkey = new PasskeyClient(prfProvider, null, null);

            // ANCHOR: connect-with-passkey
            // Single-CTA onboarding: silent sign-in for a returning user,
            // fall-through to register on a fresh device.
            var response = await passkey.ConnectWithPasskey(
                new ConnectWithPasskeyRequest()
            );

            if (response.labels.Length > 1)
            {
                // Returning multi-wallet user: let them pick a label, then
                // SignIn to the chosen wallet.
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
            var prfProvider = new CustomPrfProvider();
            var passkey = new PasskeyClient(prfProvider, null, null);

            // ANCHOR: register-passkey
            var response = await passkey.Register(new RegisterRequest(label: "personal"));

            var config = BreezSdkSparkMethods.DefaultConfig(network: Network.Mainnet);
            var sdk = await BreezSdkSparkMethods.Connect(new ConnectRequest(
                config: config,
                seed: response.wallet.seed,
                storageDir: "./.data"
            ));
            // ANCHOR_END: register-passkey
            return sdk;
        }

        async Task CredentialMetadata()
        {
            var prfProvider = new CustomPrfProvider();
            var passkey = new PasskeyClient(prfProvider, null, null);

            // ANCHOR: credential-metadata
            var response = await passkey.Register(new RegisterRequest(label: "personal"));

            if (response.credential is not null)
            {
                // Persist to reopen the same wallet on sign-in
                Console.WriteLine(response.credential.credentialId);
                // Authenticator model (display hint, unverified)
                Console.WriteLine(response.credential.aaguid);
                // Whether the passkey syncs across devices
                Console.WriteLine(response.credential.backupEligible);
            }

            // Pin the stored credential ID so the OS can't substitute a sibling
            // credential, which would derive a different wallet.
            var signInResponse = await passkey.SignIn(new SignInRequest(
                label: "personal",
                allowCredentials: new byte[][]
                {
                    // stored credentialId bytes
                }
            ));
            // Pass to connect() to open the wallet
            Console.WriteLine(signInResponse.wallet.seed);
            // Label this wallet was derived from
            Console.WriteLine(signInResponse.wallet.label);
            // This passkey's labels (populated on discovery sign-in)
            Console.WriteLine(signInResponse.labels);
            // Credential signed in with (credential_id only)
            Console.WriteLine(signInResponse.credential);
            // ANCHOR_END: credential-metadata
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
                    Console.WriteLine($"Domain association failed (source={notAssociated.source}): " +
                                      $"{notAssociated.reason}");
                    return;
                case DomainAssociation.Skipped:
                    break;
            }
            // ANCHOR_END: domain-association
        }

        async Task<Wallet> RecoverFromAlreadyExists()
        {
            var prfProvider = new CustomPrfProvider();
            var passkey = new PasskeyClient(prfProvider, null, null);

            // ANCHOR: recover-already-exists
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
            var prfProvider = new CustomPrfProvider();
            var passkey = new PasskeyClient(prfProvider, null, null);

            // ANCHOR: handle-timeout
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
