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
            // Register a new credential and return its ID, the WebAuthn
            // user.id the platform recorded (returned for host-side
            // correlation, never host-supplied), AAGUID, and BE flag.
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
            var passkey = new PasskeyClient(prfProvider, null);

            // CheckAvailability collapses IsSupported + CheckDomainAssociation
            // into a single tagged value. Branch on the variant the host needs.
            switch (await passkey.CheckAvailability())
            {
                case PasskeyAvailability.Available:
                    // Show passkey as primary option.
                    break;
                case PasskeyAvailability.PrfUnsupported:
                    // Fall back to mnemonic flow.
                    break;
                case PasskeyAvailability.NotAssociated notAssociated:
                    Console.WriteLine($"Domain association failed (source={notAssociated.source}): {notAssociated.reason}");
                    break;
                case PasskeyAvailability.Skipped:
                    // No verification source on this platform; proceed normally.
                    break;
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

            // Hosts SHOULD persist credential.credentialId (for excludeCredentialIds
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

        async Task<List<string>> ListLabels()
        {
            // ANCHOR: list-labels
            var prfProvider = new CustomPrfProvider();
            var config = new PasskeyConfig(
                breezApiKey: "<breez api key>",
                // Optional: override the default wallet label used when
                // Register / SignIn receive `label = null`. Falls back to
                // the SDK's internal "Default" when unset.
                defaultLabel: "personal"
            );
            var passkey = new PasskeyClient(prfProvider, config);

            // SignIn with no label runs in discovery mode: it derives the
            // master seed AND lists labels in the same ceremony, so a
            // follow-up Labels().List() reads from the cached identity for free.
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
            // ANCHOR: store-label
            var prfProvider = new CustomPrfProvider();
            var config = new PasskeyConfig(
                breezApiKey: "<breez api key>",
                defaultLabel: null
            );
            var passkey = new PasskeyClient(prfProvider, config);

            // For a new label on an existing identity, call SignIn(newLabel)
            // first to seed the SDK's identity cache via setup_wallet, THEN
            // Labels().Store() uses the cached identity for free (1 OS prompt total).
            await passkey.Labels().Store(label: "personal");
            // ANCHOR_END: store-label
        }

        async Task<Wallet> SingleCtaOnboarding()
        {
            // ANCHOR: signin-fallback-register
            // Single-CTA onboarding: try silent SignIn first, fall through
            // to Register on CredentialNotFound. The OS shows ONE prompt
            // for a returning user (silent assertion succeeds), TWO for a
            // new user (silent assertion fast-fails, then create + dual-salt
            // assert).
            var prfProvider = new CustomPrfProvider();
            var passkey = new PasskeyClient(prfProvider, null);

            try
            {
                // Discovery mode (label=null): derives master + configured
                // default label in a single ceremony. The fresh-device user
                // fast-fails in <300ms with no UI shown.
                var response = await passkey.SignIn(new SignInRequest(
                    label: null,
                    extraSalts: new List<NamedSalt>()
                ));
                return response.wallet;
            }
            catch (PrfProviderException.CredentialNotFound)
            {
                // CredentialNotFound is the SDK's classification for "no
                // matching credential on this device", including iOS's
                // <300ms fast-fail case where the platform conflates
                // no-cred with user-cancel. The variant carries a string
                // payload with diagnostic detail.
                var response = await passkey.Register(new RegisterRequest(
                    label: "personal",
                    extraSalts: new List<NamedSalt>(),
                    excludeCredentialIds: new List<byte[]>()
                ));
                return response.wallet;
            }
            // ANCHOR_END: signin-fallback-register
        }

        async Task CheckDomain()
        {
            // ANCHOR: domain-association
            // Verify Apple AASA / Android Asset Links / Web Related Origins
            // before the first WebAuthn ceremony. Diagnostic only: never
            // blocks.
            var prfProvider = new CustomPrfProvider();
            var result = await prfProvider.CheckDomainAssociation();

            switch (result)
            {
                case DomainAssociation.Associated:
                    // Safe to proceed.
                    break;
                case DomainAssociation.NotAssociated notAssociated:
                    // Configuration is wrong (entitlement missing, AASA
                    // stale, assetlinks malformed). Surface a
                    // developer-facing error.
                    Console.WriteLine($"Domain association failed (source={notAssociated.source}): {notAssociated.reason}");
                    return;
                case DomainAssociation.Skipped:
                    // Verification could not be performed (offline,
                    // endpoint timeout, no public-suffix match). Proceed
                    // normally: this is NOT a negative signal.
                    break;
            }
            // ANCHOR_END: domain-association
        }

        async Task<Wallet> RecoverFromAlreadyExists()
        {
            // ANCHOR: recover-already-exists
            // The OS rejected Register because the user's password
            // manager already holds a credential matching
            // `excludeCredentialIds`. Route the user to the sign-in path:
            // the OS picker will surface the existing credential and the
            // SDK's identity cache will warm up on the assertion.
            var prfProvider = new CustomPrfProvider();
            var passkey = new PasskeyClient(prfProvider, null);

            try
            {
                var response = await passkey.Register(new RegisterRequest(
                    label: "personal",
                    extraSalts: new List<NamedSalt>(),
                    excludeCredentialIds: new List<byte[]>
                    {
                        // app-persisted credential IDs from prior registrations
                    }
                ));
                return response.wallet;
            }
            catch (PrfProviderException.CredentialAlreadyExists)
            {
                // Flip to sign-in. The existing credential's PRF output
                // is the same wallet seed the host would have minted on
                // register.
                var response = await passkey.SignIn(new SignInRequest(
                    label: "personal",
                    extraSalts: new List<NamedSalt>()
                ));
                return response.wallet;
            }
            // ANCHOR_END: recover-already-exists
        }

        async Task<SignInResponse> HandleTimeout()
        {
            // ANCHOR: handle-timeout
            // The OS biometric inactivity timeout (~55s+) tore down the
            // prompt without user intent. Distinct from a real cancel:
            // hosts may surface a re-prompt UI without treating it as the
            // user opting out. The SDK fires
            // PrfProviderException.UserTimedOut when assertion or register
            // elapsed time crosses 55_000 ms.
            var prfProvider = new CustomPrfProvider();
            var passkey = new PasskeyClient(prfProvider, null);

            try
            {
                return await passkey.SignIn(new SignInRequest(
                    label: "personal",
                    extraSalts: new List<NamedSalt>()
                ));
            }
            catch (PrfProviderException.UserTimedOut)
            {
                // Show a sticky retry screen with timeout-specific copy.
                // Do NOT auto-retry without user input.
                Console.WriteLine("Sign-in timed out: show \"Try Again\" UI.");
                throw;
            }
            // ANCHOR_END: handle-timeout
        }
    }
}
